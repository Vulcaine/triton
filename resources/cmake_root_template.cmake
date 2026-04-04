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
  string(JSON _cmake ERROR_VARIABLE _cmake_err GET "${_dep}" cmake)
  if(_cmake_err)
    return()
  endif()
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

  # Internal: try find_package with a candidate name + snapshot/link.
  # Returns via _triton_found_result variable in PARENT_SCOPE.
  function(_triton_try_find_and_link tgt candidate before_bs before_imp)
    set(_triton_found_result FALSE PARENT_SCOPE)
    find_package(${candidate} CONFIG QUIET)
    if(NOT ${candidate}_FOUND)
      find_package(${candidate} QUIET)
    endif()
    if(NOT ${candidate}_FOUND)
      return()
    endif()
    _triton_new_targets(_new2 ${before_bs} ${before_imp})
    list(LENGTH _new2 _n2)
    if(_n2 EQUAL 1)
      list(GET _new2 0 _t2)
      target_link_libraries(${tgt} ${_link_vis} ${_t2})
      set(_triton_found_result TRUE PARENT_SCOPE)
      return()
    elseif(_n2 GREATER 1)
      _triton_pick_best_target(_best2 "${candidate}" "${_new2}")
      if(_best2)
        target_link_libraries(${tgt} ${_link_vis} ${_best2})
        set(_triton_found_result TRUE PARENT_SCOPE)
        return()
      endif()
    endif()
  endfunction()

  function(triton_find_vcpkg_and_link_strict tgt pkg)
    set(_link_vis "PUBLIC")
    if(ARGC GREATER 2)
      set(_link_vis "${ARGV2}")
    endif()
    _triton_dir_targets(_before_bs _before_imp)

    # --- Stage 1: try the package name as-is ---
    find_package(${pkg} CONFIG QUIET)
    if(NOT ${pkg}_FOUND)
      find_package(${pkg} QUIET)
    endif()

    _triton_new_targets(_new _before_bs _before_imp)
    list(LENGTH _new _n)
    if(_n EQUAL 1)
      list(GET _new 0 _t)
      target_link_libraries(${tgt} ${_link_vis} ${_t})
      return()
    elseif(_n GREATER 1)
      _triton_pick_best_target(_best "${pkg}" "${_new}")
      if(_best)
        target_link_libraries(${tgt} ${_link_vis} ${_best})
        return()
      endif()
      message(FATAL_ERROR [[
triton: multiple targets introduced by package '${pkg}':
  ${_new}
Please specify an explicit mapping in triton.json:
  { "name": "${pkg}", "package": "<Pkg>", "targets": ["<Pkg::Target>"] }
]])
    endif()

    # --- Stage 2: try common name variations ---
    # Replace hyphens with underscores: nlohmann-json → nlohmann_json
    string(REPLACE "-" "_" _underscore "${pkg}")
    # Uppercase: sdl2 → SDL2
    string(TOUPPER "${pkg}" _upper)
    # Uppercase with underscores: sdl2-mixer → SDL2_MIXER
    string(TOUPPER "${_underscore}" _upper_underscore)

    set(_variants "${_underscore}" "${_upper}" "${_upper_underscore}")
    list(REMOVE_DUPLICATES _variants)
    list(REMOVE_ITEM _variants "${pkg}") # don't retry the original

    foreach(_try IN LISTS _variants)
      _triton_dir_targets(_vb_bs _vb_imp)
      _triton_try_find_and_link(${tgt} "${_try}" _vb_bs _vb_imp)
      if(_triton_found_result)
        return()
      endif()
    endforeach()

    # --- Stage 3: scan vcpkg share directory for matching Config.cmake ---
    if(DEFINED VCPKG_INSTALLED_DIR AND DEFINED VCPKG_TARGET_TRIPLET)
      set(_share_root "${VCPKG_INSTALLED_DIR}/${VCPKG_TARGET_TRIPLET}/share")
      if(EXISTS "${_share_root}")
        string(TOLOWER "${pkg}" _pkg_lower)
        string(REPLACE "-" "_" _pkg_norm "${_pkg_lower}")
        file(GLOB _share_entries LIST_DIRECTORIES true "${_share_root}/*")
        foreach(_entry IN LISTS _share_entries)
          if(IS_DIRECTORY "${_entry}")
            get_filename_component(_dirname "${_entry}" NAME)
            string(TOLOWER "${_dirname}" _dirname_lower)
            string(REPLACE "-" "_" _dirname_norm "${_dirname_lower}")
            # Match: case-insensitive or hyphen/underscore normalized
            if(_dirname_lower STREQUAL _pkg_lower OR _dirname_norm STREQUAL _pkg_norm)
              if(NOT _dirname STREQUAL "${pkg}")
                # Found a matching dir with different casing — try it
                _triton_dir_targets(_sd_bs _sd_imp)
                _triton_try_find_and_link(${tgt} "${_dirname}" _sd_bs _sd_imp)
                if(_triton_found_result)
                  return()
                endif()
              endif()
            endif()
          endif()
        endforeach()
      endif()
    endif()

    # --- Stage 4: legacy module variables ---
    string(REGEX REPLACE "[^A-Za-z0-9]" "_" _pfx "${pkg}")
    string(TOUPPER "${_pfx}" _PFX)
    string(SUBSTRING "${_pfx}" 0 1 _first_char)
    string(TOUPPER "${_first_char}" _first_upper)
    string(SUBSTRING "${_pfx}" 1 -1 _rest)
    set(_TitleCase "${_first_upper}${_rest}")
    set(_synth "")
    foreach(_try_pfx IN ITEMS "${_PFX}" "${_pfx}" "${_TitleCase}" "${pkg}")
      if(NOT _synth)
        _triton_make_iface_from_module_vars(_synth "${pkg}" "${_try_pfx}")
      endif()
    endforeach()
    if(_synth)
      target_link_libraries(${tgt} ${_link_vis} ${_synth})
      return()
    endif()

    # --- Stage 5: pkg-config ---
    find_package(PkgConfig QUIET)
    if(PKG_CONFIG_FOUND)
      foreach(_pc_name ${pkg} ${pkg}2 lib${pkg})
        pkg_check_modules(_pc_${_pfx} QUIET IMPORTED_TARGET ${_pc_name})
        if(_pc_${_pfx}_FOUND)
          target_link_libraries(${tgt} ${_link_vis} PkgConfig::_pc_${_pfx})
          return()
        endif()
      endforeach()
    endif()

    # --- Stage 6: raw find_library ---
    find_library(_fl_${_pfx} NAMES ${pkg} ${pkg}2 lib${pkg})
    if(_fl_${_pfx})
      target_link_libraries(${tgt} ${_link_vis} ${_fl_${_pfx}})
      find_path(_fh_${_pfx} NAMES ${pkg}.h ${pkg}/lzo1x.h)
      if(_fh_${_pfx})
        target_include_directories(${tgt} PRIVATE ${_fh_${_pfx}})
      endif()
      return()
    endif()

    # --- Stage 7: warning (not fatal) ---
    message(WARNING
      "triton: could not auto-detect a CMake target for '${pkg}'.\n"
      "You likely need to specify the package name in triton.json:\n"
      "  { \"name\": \"${pkg}\", \"package\": \"<CMakePackageName>\" }\n"
      "Run 'triton find-target ${pkg}' or check vcpkg/installed/<triplet>/share/ for the correct name.")
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

  function(_triton_git_repo_autolink tgt abs hint)
    get_target_property(_manual_includes ${tgt} TRITON_MANUAL_INCLUDE_DIRS)
    get_target_property(_manual_sources ${tgt} TRITON_MANUAL_SOURCES)
    if(_manual_includes STREQUAL "TRUE")
      set(_manual_includes ON)
    else()
      set(_manual_includes OFF)
    endif()
    if(_manual_sources STREQUAL "TRUE")
      set(_manual_sources ON)
    else()
      set(_manual_sources OFF)
    endif()

    file(GLOB_RECURSE _raw_sources RELATIVE "${abs}" CONFIGURE_DEPENDS
      "${abs}/*.c" "${abs}/*.cc" "${abs}/*.cxx" "${abs}/*.cpp" "${abs}/*.ixx")
    file(GLOB_RECURSE _raw_headers RELATIVE "${abs}" CONFIGURE_DEPENDS
      "${abs}/*.h" "${abs}/*.hh" "${abs}/*.hpp" "${abs}/*.hxx" "${abs}/*.inl" "${abs}/*.inc")

    set(_sources "")
    foreach(_rel IN LISTS _raw_sources)
      string(REPLACE "\\" "/" _rel_norm "${_rel}")
      if(_rel_norm MATCHES "(^|/)(examples?|samples?|tests?|test|docs?|doc|benchmarks?|benchmark|fuzz|cmake)(/|$)")
        continue()
      endif()
      list(APPEND _sources "${abs}/${_rel_norm}")
    endforeach()

    set(_includes "${abs}")
    foreach(_rel IN LISTS _raw_headers)
      string(REPLACE "\\" "/" _rel_norm "${_rel}")
      if(_rel_norm MATCHES "(^|/)(examples?|samples?|tests?|test|docs?|doc|benchmarks?|benchmark|fuzz|cmake)(/|$)")
        continue()
      endif()
      get_filename_component(_hdr_dir "${abs}/${_rel_norm}" DIRECTORY)
      if(_hdr_dir)
        list(APPEND _includes "${_hdr_dir}")
      endif()
    endforeach()
    list(REMOVE_DUPLICATES _includes)

    if(_manual_sources AND _manual_includes)
      set(_handled TRUE PARENT_SCOPE)
      return()
    endif()

    list(LENGTH _sources _src_count)
    list(LENGTH _includes _inc_count)
    if(_src_count EQUAL 0 AND _inc_count EQUAL 0)
      set(_handled FALSE PARENT_SCOPE)
      return()
    endif()

    message(WARNING
      "triton: git dep '${hint}' does not expose a usable CMake target. "
      "Triton is attempting source/include auto-detection under '${abs}'. "
      "If this is wrong, specify component 'sources' and/or 'include_dirs' explicitly.")

    string(MAKE_C_IDENTIFIER "triton_git_${hint}" _fallback_target)
    if(_manual_sources)
      if(NOT _manual_includes)
        target_include_directories(${tgt} PRIVATE ${_includes})
      endif()
      set(_handled TRUE PARENT_SCOPE)
      return()
    endif()

    if(_src_count GREATER 0)
      if(NOT TARGET ${_fallback_target})
        add_library(${_fallback_target} STATIC)
        target_sources(${_fallback_target} PRIVATE ${_sources})
        if(_manual_includes)
          target_include_directories(${_fallback_target} PRIVATE ${_includes})
        else()
          target_include_directories(${_fallback_target} PUBLIC ${_includes})
        endif()
        get_target_property(_consumer_links ${tgt} LINK_LIBRARIES)
        if(_consumer_links)
          target_link_libraries(${_fallback_target} PRIVATE ${_consumer_links})
        endif()
        set_target_properties(${_fallback_target} PROPERTIES FOLDER "third_party")
        _triton_apply_iterator_policy_to_target(${_fallback_target})
      endif()
      target_link_libraries(${tgt} ${_link_vis} ${_fallback_target})
      set(_handled TRUE PARENT_SCOPE)
      return()
    endif()

    if(_manual_includes)
      set(_handled TRUE PARENT_SCOPE)
      return()
    endif()

    if(NOT TARGET ${_fallback_target})
      add_library(${_fallback_target} INTERFACE)
      target_include_directories(${_fallback_target} INTERFACE ${_includes})
    endif()
    target_link_libraries(${tgt} ${_link_vis} ${_fallback_target})
    set(_handled TRUE PARENT_SCOPE)
  endfunction()

  # --- Git subdir helper: add once, link one target, pass per-dep cmake, and enforce iterator policy.
  function(triton_add_subdir_and_link_strict tgt path hint)
    set(_link_vis "PUBLIC")
    if(ARGC GREATER 3)
      set(_link_vis "${ARGV3}")
    endif()
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
    if(EXISTS "${_abs}/CMakeLists.txt")
      if(_ix EQUAL -1)
        add_subdirectory("${_abs}" "${_bin}" EXCLUDE_FROM_ALL)
        set_property(GLOBAL PROPERTY TRITON_ADDED_SUBDIRS "${_added};${_abs}|${_bin}")
      endif()
    else()
      _triton_git_repo_autolink(${tgt} "${_abs}" "${hint}")
      if(_handled)
        return()
      endif()
      message(FATAL_ERROR "triton: git dep '${hint}' has no CMakeLists.txt and Triton could not infer usable sources/includes under '${_abs}'. Consider explicit component 'sources'/'include_dirs'.")
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
      target_link_libraries(${tgt} ${_link_vis} ${_t})
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
      _triton_git_repo_autolink(${tgt} "${_abs}" "${hint}")
      if(_handled)
        return()
      endif()
      message(FATAL_ERROR "triton: no library targets were created by '${_abs}'. Consider explicit component 'sources'/'include_dirs'.")
    elseif(_c EQUAL 1)
      list(GET _cand 0 _t)
      target_link_libraries(${tgt} ${_link_vis} ${_t})
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
        target_link_libraries(${tgt} ${_link_vis} ${_t})
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
