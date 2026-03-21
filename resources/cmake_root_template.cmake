# Disable vcpkg's MSBuild-style AppLocal DLL copy (we handle DLLs ourselves with TARGET_RUNTIME_DLLS)
# set(VCPKG_APPLOCAL_DEPS OFF CACHE BOOL "Disable vcpkg auto-copy of runtime DLLs" FORCE)

project(triton_components LANGUAGES CXX)

# Enable CTest globally so `ctest` works
enable_testing()

# === Global MSVC settings ===
if(MSVC)
  # Match vcpkg defaults: Release=/MD, Debug=/MDd
  set(CMAKE_MSVC_RUNTIME_LIBRARY
      "MultiThreaded$<$<CONFIG:Debug>:Debug>DLL"
      CACHE STRING "" FORCE)
endif()

# Knob: keep ON so we always enforce iterator level
set(TRITON_ENFORCE_MSVC_ITERATOR_LEVEL ON CACHE BOOL
    "Force _ITERATOR_DEBUG_LEVEL across all targets (2 in Debug, 0 otherwise)")

# ---------- read triton.json (only what's needed) ----------
set(_TRITON_JSON_PATH "")
foreach(_cand "${CMAKE_SOURCE_DIR}/../triton.json" "${CMAKE_SOURCE_DIR}/triton.json")
  if(NOT _TRITON_JSON_PATH AND EXISTS "${_cand}")
    set(_TRITON_JSON_PATH "${_cand}")
  endif()
endforeach()

# Return the JSON object for the first deps[i] whose "name" == HINT.
function(_triton_json_find_dep_by_name OUT_OBJ HINT)
  set(${OUT_OBJ} "" PARENT_SCOPE)
  if(NOT _TRITON_JSON_PATH)
    return()
  endif()
  file(READ "${_TRITON_JSON_PATH}" _json)
  string(JSON _deps_type TYPE "${_json}" deps)
  if(NOT _deps_type STREQUAL "ARRAY")
    return()
  endif()
  string(JSON _n LENGTH "${_json}" deps)
  math(EXPR _last "${_n}-1")
  foreach(i RANGE ${_last})
    string(JSON _item_type TYPE "${_json}" deps ${i})
    if(_item_type STREQUAL "OBJECT")
      string(JSON _dep GET "${_json}" deps ${i})
      string(JSON _name GET "${_dep}" name)
      if(_name STREQUAL "${HINT}")
        set(${OUT_OBJ} "${_dep}" PARENT_SCOPE)
        return()
      endif()
    endif()
  endforeach()
endfunction()

# Convert a JSON array of strings to a CMake list
function(_triton_json_array_to_list OUT JSON_ARRAY)
  set(${OUT} "" PARENT_SCOPE)
  if(NOT JSON_ARRAY)
    return()
  endif()
  string(JSON _type TYPE "${JSON_ARRAY}")
  if(NOT _type STREQUAL "ARRAY")
    return()
  endif()
  string(JSON _n LENGTH "${JSON_ARRAY}")
  if(_n EQUAL 0)
    return()
  endif()
  math(EXPR _last "${_n}-1")
  set(_res "")
  foreach(i RANGE ${_last})
    string(JSON _val GET "${JSON_ARRAY}" ${i})
    list(APPEND _res "${_val}")
  endforeach()
  set(${OUT} "${_res}" PARENT_SCOPE)
endfunction()

# Get the "cmake" key/value pairs (KEY=VALUE strings) for a dep
function(_triton_dep_cmake_kv_from_json HINT OUT_KV_LIST)
  set(${OUT_KV_LIST} "" PARENT_SCOPE)
  _triton_json_find_dep_by_name(_dep "${HINT}")
  if(NOT _dep)
    return()
  endif()
  string(JSON _cmake GET "${_dep}" cmake)
  _triton_json_array_to_list(_arr "${_cmake}")
  set(${OUT_KV_LIST} "${_arr}" PARENT_SCOPE)
endfunction()

# ---------- snapshot helpers ----------
if(NOT COMMAND triton_find_vcpkg_and_link_strict)

  function(_triton_dir_targets OUT_BS OUT_IMP)
    get_directory_property(_bs BUILDSYSTEM_TARGETS)
    get_directory_property(_imp IMPORTED_TARGETS)
    if(NOT _bs)
      set(_bs "")
    endif()
    if(NOT _imp)
      set(_imp "")
    endif()
    set(${OUT_BS} "${_bs}" PARENT_SCOPE)
    set(${OUT_IMP} "${_imp}" PARENT_SCOPE)
  endfunction()

  function(_triton_new_targets OUT BEFORE_BS BEFORE_IMP)
    _triton_dir_targets(_after_bs _after_imp)
    set(_new "")
    foreach(t IN LISTS _after_bs)
      list(FIND ${BEFORE_BS} "${t}" _ix)
      if(_ix EQUAL -1)
        list(APPEND _new "${t}")
      endif()
    endforeach()
    foreach(t IN LISTS _after_imp)
      list(FIND ${BEFORE_IMP} "${t}" _ix)
      if(_ix EQUAL -1)
        list(APPEND _new "${t}")
      endif()
    endforeach()
    list(REMOVE_DUPLICATES _new)
    set(${OUT} "${_new}" PARENT_SCOPE)
  endfunction()

  function(_triton_make_iface_from_module_vars OUT PKG VAR_PREFIX)
    set(_incs "")
    foreach(v IN ITEMS "${VAR_PREFIX}_INCLUDE_DIRS" "${VAR_PREFIX}_INCLUDE_DIR")
      if(DEFINED ${v})
        set(_incs ${${v}})
        break()
      endif()
    endforeach()
    set(_libs "")
    foreach(v IN ITEMS "${VAR_PREFIX}_LIBRARIES" "${VAR_PREFIX}_LIBRARY")
      if(DEFINED ${v})
        set(_libs ${${v}})
        break()
      endif()
    endforeach()
    if(_incs OR _libs)
      set(_tgt "triton::${PKG}")
      if(NOT TARGET ${_tgt})
        add_library(${_tgt} INTERFACE)
        if(_incs)
          target_include_directories(${_tgt} INTERFACE ${_incs})
        endif()
        if(_libs)
          target_link_libraries(${_tgt} INTERFACE ${_libs})
        endif()
      endif()
      set(${OUT} ${_tgt} PARENT_SCOPE)
    else()
      set(${OUT} "" PARENT_SCOPE)
    endif()
  endfunction()

  # Try to pick the best target from a list of candidates for a given package name.
  # Heuristic: prefer Pkg::Pkg or Pkg::pkg, then filter out common auxiliaries
  # (*main, *-static, *-shared), then fall back to the first remaining target.
  function(_triton_pick_best_target OUT PKG CANDIDATES)
    set(${OUT} "" PARENT_SCOPE)

    # Normalise: lowercase the hint for matching
    string(TOLOWER "${PKG}" _hint_lower)

    # Pass 1: exact Pkg::Pkg match (case-insensitive)
    foreach(t IN LISTS CANDIDATES)
      string(TOLOWER "${t}" _tl)
      if(_tl MATCHES "^[^:]+::${_hint_lower}$")
        set(${OUT} "${t}" PARENT_SCOPE)
        return()
      endif()
    endforeach()

    # Pass 2: filter out common auxiliary targets (*main, *-static, *-shared)
    set(_filtered "")
    foreach(t IN LISTS CANDIDATES)
      string(TOLOWER "${t}" _tl)
      if(NOT _tl MATCHES "(main|static|shared)$")
        list(APPEND _filtered "${t}")
      endif()
    endforeach()
    list(LENGTH _filtered _fn)
    if(_fn EQUAL 1)
      list(GET _filtered 0 _t)
      set(${OUT} "${_t}" PARENT_SCOPE)
      return()
    endif()

    # Pass 3: among filtered, prefer one containing the hint name
    foreach(t IN LISTS _filtered)
      string(TOLOWER "${t}" _tl)
      if(_tl MATCHES "${_hint_lower}")
        set(${OUT} "${t}" PARENT_SCOPE)
        return()
      endif()
    endforeach()

    # Pass 4: just pick the first filtered candidate (or first overall)
    if(_fn GREATER 0)
      list(GET _filtered 0 _t)
      set(${OUT} "${_t}" PARENT_SCOPE)
    else()
      list(GET CANDIDATES 0 _t)
      set(${OUT} "${_t}" PARENT_SCOPE)
    endif()
  endfunction()

  function(triton_find_vcpkg_and_link_strict tgt pkg)
    _triton_dir_targets(_before_bs _before_imp)

    find_package(${pkg} CONFIG QUIET)
    if(NOT ${pkg}_FOUND)
      find_package(${pkg} QUIET)
    endif()

    _triton_new_targets(_new _before_bs _before_imp)
    list(LENGTH _new _n)
    if(_n EQUAL 1)
      list(GET _new 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
      return()
    elseif(_n GREATER 1)
      _triton_pick_best_target(_best "${pkg}" "${_new}")
      if(_best)
        target_link_libraries(${tgt} PRIVATE ${_best})
        return()
      endif()
      message(FATAL_ERROR [[
triton: multiple targets introduced by package '${pkg}':
  ${_new}
Please specify an explicit mapping in triton.json:
  { "name": "${pkg}", "package": "<Pkg>", "target": "<Pkg::Target>" }
]])
    endif()

    string(REGEX REPLACE "[^A-Za-z0-9]" "_" _pfx "${pkg}")
    string(TOUPPER "${_pfx}" _PFX)
    _triton_make_iface_from_module_vars(_synth "${pkg}" "${_PFX}")
    if(_synth)
      target_link_libraries(${tgt} PRIVATE ${_synth})
      return()
    endif()

    # Fallback: try pkg-config
    find_package(PkgConfig QUIET)
    if(PKG_CONFIG_FOUND)
      # Try common pkg-config names: pkg, pkg2, libpkg
      foreach(_pc_name ${pkg} ${pkg}2 lib${pkg})
        pkg_check_modules(_pc_${_pfx} QUIET IMPORTED_TARGET ${_pc_name})
        if(_pc_${_pfx}_FOUND)
          target_link_libraries(${tgt} PRIVATE PkgConfig::_pc_${_pfx})
          return()
        endif()
      endforeach()
    endif()

    # Fallback: try find_library directly
    find_library(_fl_${_pfx} NAMES ${pkg} ${pkg}2 lib${pkg})
    if(_fl_${_pfx})
      target_link_libraries(${tgt} PRIVATE ${_fl_${_pfx}})
      find_path(_fh_${_pfx} NAMES ${pkg}.h ${pkg}/lzo1x.h)
      if(_fh_${_pfx})
        target_include_directories(${tgt} PRIVATE ${_fh_${_pfx}})
      endif()
      return()
    endif()

    message(FATAL_ERROR "triton: could not determine a target for package '${pkg}'.")
  endfunction()

  # --- Helper: apply iterator policy to a target (non-IMPORTED, non-INTERFACE)
  function(_triton_apply_iterator_policy_to_target _t)
    if(NOT MSVC OR NOT TRITON_ENFORCE_MSVC_ITERATOR_LEVEL)
      return()
    endif()
    if(NOT TARGET ${_t})
      return()
    endif()
    get_target_property(_imp ${_t} IMPORTED)
    if(_imp)
      return()
    endif()
    get_target_property(_type ${_t} TYPE)
    if(_type STREQUAL "INTERFACE_LIBRARY")
      return()
    endif()

    # Ensure CRT matches vcpkg defaults
    set_property(TARGET ${_t} PROPERTY
      MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>DLL")

    # Enforce: 2 in Debug, 0 otherwise. Use /U then /D so ours win.
    target_compile_options(${_t} PRIVATE
      $<$<CONFIG:Debug>:/U_ITERATOR_DEBUG_LEVEL /D_ITERATOR_DEBUG_LEVEL=2>
      $<$<NOT:$<CONFIG:Debug>>:/U_ITERATOR_DEBUG_LEVEL /D_ITERATOR_DEBUG_LEVEL=0>)
    target_compile_definitions(${_t} PRIVATE
      $<$<CONFIG:Debug>:_ITERATOR_DEBUG_LEVEL=2>
      $<$<NOT:$<CONFIG:Debug>>:_ITERATOR_DEBUG_LEVEL=0>)
  endfunction()

  # --- Git subdir helper: add once, link one target, pass per-dep cmake, and enforce iterator policy.
  function(triton_add_subdir_and_link_strict tgt path hint)
    get_filename_component(_abs "${path}" ABSOLUTE)
    get_filename_component(_dir "${_abs}" NAME)
    set(_bin "${CMAKE_BINARY_DIR}/third_party/${_dir}")

    # Gather KEY=VALUE pairs from triton.json's "cmake" for this dep.
    _triton_dep_cmake_kv_from_json("${hint}" _cmake_kv_pairs)

    # Temporarily push those KEY=VALUE into cache so the subdir sees them.
    set(_saved_kv "")
    foreach(kv IN LISTS _cmake_kv_pairs)
      string(REPLACE "=" ";" _pair "${kv}")
      list(LENGTH _pair _len)
      if(_len GREATER 1)
        list(GET _pair 0 _k)
        list(REMOVE_AT _pair 0)
        string(REPLACE ";" "=" _v "${_pair}") # rejoin in case VALUE itself had '='
        set(_was_set FALSE)
        if(DEFINED ${_k})
          set(_was_set TRUE)
          set(_old_val "${${_k}}")
        endif()
        list(APPEND _saved_kv "${_k}=${_was_set}=${_old_val}")
        set(${_k} "${_v}" CACHE STRING "" FORCE)
      endif()
    endforeach()

    get_property(_added GLOBAL PROPERTY TRITON_ADDED_SUBDIRS)
    if(NOT _added)
      set(_added "")
    endif()
    list(FIND _added "${_abs}|${_bin}" _ix)

    _triton_dir_targets(_before_bs _before_imp)
    if(_ix EQUAL -1)
      add_subdirectory("${_abs}" "${_bin}" EXCLUDE_FROM_ALL)
      set_property(GLOBAL PROPERTY TRITON_ADDED_SUBDIRS "${_added};${_abs}|${_bin}")
    endif()

    # Restore previous cache values so we don't leak into other deps.
    foreach(ent IN LISTS _saved_kv)
      string(REPLACE "=" ";" _triple "${ent}")
      list(GET _triple 0 _k)
      list(GET _triple 1 _was_set)
      list(GET _triple 2 _old)
      if(_was_set)
        set(${_k} "${_old}" CACHE STRING "" FORCE)
      else()
        unset(${_k} CACHE)
      endif()
    endforeach()

    # Find newly created targets and everything under this source tree (recursively).
    _triton_new_targets(_new _before_bs _before_imp)
    get_property(_all GLOBAL PROPERTY TARGETS)
    set(_tree_targets "")
    foreach(t IN LISTS _all)
      if(TARGET ${t})
        get_target_property(_src ${t} SOURCE_DIR)
        if(_src)
          string(FIND "${_src}" "${_abs}" _pos)
          if(_pos EQUAL 0)
            list(APPEND _tree_targets "${t}")
          endif()
        endif()
      endif()
    endforeach()
    list(REMOVE_DUPLICATES _tree_targets)

    # Enforce iterator policy on everything built in that tree.
    foreach(_t IN LISTS _tree_targets)
      _triton_apply_iterator_policy_to_target(${_t})
    endforeach()

    # Strict linking behavior
    list(LENGTH _new _cnt)
    if(_cnt EQUAL 1)
      list(GET _new 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
      return()
    elseif(_cnt GREATER 1)
      message(FATAL_ERROR [[
triton: multiple library targets were created by '${_abs}':
  ${_new}
Please set the 'target' for git dep '${hint}' in triton.json.
]])
    endif()

    # Fallback: targets whose SOURCE_DIR == repo root
    get_property(_all2 GLOBAL PROPERTY TARGETS)
    set(_cand "")
    foreach(t IN LISTS _all2)
      if(TARGET ${t})
        get_target_property(_src ${t} SOURCE_DIR)
        if(_src AND _src STREQUAL _abs)
          list(APPEND _cand "${t}")
        endif()
      endif()
    endforeach()
    list(REMOVE_DUPLICATES _cand)
    list(LENGTH _cand _c)
    if(_c EQUAL 0)
      message(FATAL_ERROR "triton: no library targets were created by '${_abs}'.")
    elseif(_c EQUAL 1)
      list(GET _cand 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
      return()
    else()
      string(TOLOWER "${hint}" _h)
      set(_both "")
      foreach(t IN LISTS _cand)
        string(TOLOWER "${t}" _tl)
        if(_tl MATCHES "::${_h}($|::)" OR _tl MATCHES "(^|[^A-Za-z0-9])${_h}($|[^A-Za-z0-9])")
          list(APPEND _both "${t}")
        endif()
      endforeach()
      list(REMOVE_DUPLICATES _both)
      list(LENGTH _both _bh)
      if(_bh EQUAL 1)
        list(GET _both 0 _t)
        target_link_libraries(${tgt} PRIVATE ${_t})
        return()
      endif()
      message(FATAL_ERROR [[
triton: multiple library targets live under '${_abs}':
  ${_cand}
]])
    endif()
  endfunction()

endif() # end helper definitions

# ---- Subdirectories (managed by Triton) ----
# Triton will inject:
#   add_subdirectory(Engine)
#   add_subdirectory(Game)
#   ...
# ## triton:components begin
# ## triton:components end

# ---- Tests integration (robust discovery on Windows/vcpkg) ----
include(CTest)
# Prefer discovering gtests at ctest time (after DLLs are copied by build steps)
set(GTEST_DISCOVER_TESTS_DISCOVERY_MODE PRE_TEST CACHE STRING "Discover gtests at ctest time")
# IMPORTANT: Do NOT add_subdirectory(tests) here; Triton injects it in the managed block above.

# ---- FINAL SWEEP: apply iterator policy AFTER all targets exist ----
if(MSVC AND TRITON_ENFORCE_MSVC_ITERATOR_LEVEL)
  get_property(_all_targets GLOBAL PROPERTY TARGETS)
  foreach(_t IN LISTS _all_targets)
    _triton_apply_iterator_policy_to_target(${_t})
  endforeach()
endif()
