# === Triton CMake helpers (simple & strict) ===================================

# Global norms (apply once for the whole configure)
if (MSVC)
  # Prefer the DLL runtime everywhere (/MD, /MDd)
  if (NOT DEFINED CMAKE_MSVC_RUNTIME_LIBRARY)
    set(CMAKE_MSVC_RUNTIME_LIBRARY
        "MultiThreaded$<$<CONFIG:Debug>:Debug>DLL"
        CACHE STRING "" FORCE)
  endif()
  # Proper exception unwinding (silences C4530)
  add_compile_options(/EHsc)
endif()

# On Windows, strongly prefer Win32 threads rather than pthreads
set(THREADS_PREFER_PTHREAD_FLAG OFF CACHE BOOL "" FORCE)

# ==============================================================================
# Only define the helpers once per configure
if (NOT COMMAND triton_find_vcpkg_and_link_strict)

  # Snapshot directory-scope targets
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

  # Compute "new" targets since BEFORE_* snapshots (both buildsystem + imported)
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

  # Build INTERFACE target from typical Find-module variables (<PFX>_INCLUDE_DIR(S), <PFX>_LIBRARIES)
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

  # One function to rule them all (vcpkg)
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
      message(FATAL_ERROR
"triton: multiple targets introduced by package '${pkg}':
  ${_new}
Please specify an explicit mapping in triton.json: { \"name\": \"${pkg}\", \"package\": \"<Pkg>\", \"target\": \"<Pkg::Target>\" }")
    endif()

    string(REGEX REPLACE "[^A-Za-z0-9]" "_" _pfx "${pkg}")
    string(TOUPPER "${_pfx}" _PFX)
    _triton_make_iface_from_module_vars(_synth "${pkg}" "${_PFX}")
    if(_synth)
      target_link_libraries(${tgt} PRIVATE ${_synth})
      return()
    endif()

    message(FATAL_ERROR
"triton: could not determine a target for package '${pkg}'.")
  endfunction()

  # Git subdir helper (deduplicated): add once globally and link one new/unique target.
  function(triton_add_subdir_and_link_strict tgt path hint)
    get_filename_component(_abs "${path}" ABSOLUTE)
    get_filename_component(_dir "${_abs}" NAME)
    set(_bin "${CMAKE_BINARY_DIR}/third_party/${_dir}")

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

    # What targets were created by the subdir?
    _triton_new_targets(_new _before_bs _before_imp)

    # Normalize MSVC CRT and iterator level for newly-created targets (prevents /MTd and _ITERATOR_DEBUG_LEVEL=0)
    if (MSVC)
      foreach(_t IN LISTS _new)
        if (TARGET ${_t})
          get_target_property(_ty ${_t} TYPE)
          if(_ty MATCHES "EXECUTABLE|STATIC_LIBRARY|SHARED_LIBRARY|MODULE_LIBRARY|OBJECT_LIBRARY")
            # Force DLL runtime on those targets
            set_property(TARGET ${_t} PROPERTY MSVC_RUNTIME_LIBRARY "${CMAKE_MSVC_RUNTIME_LIBRARY}")
            # Make Debug builds compatible with vcpkg Debug libs
            target_compile_definitions(${_t} PRIVATE $<$<CONFIG:Debug>:_ITERATOR_DEBUG_LEVEL=2>)
          endif()
        endif()
      endforeach()
    endif()

    # If exactly one target showed up, link it
    list(LENGTH _new _cnt)
    if(_cnt EQUAL 1)
      list(GET _new 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
      return()
    elseif(_cnt GREATER 1)
      message(FATAL_ERROR
"triton: multiple library targets were created by '${_abs}':
  ${_new}
Please set the 'target' for git dep '${hint}' in triton.json.")
    endif()

    # Otherwise, try to find all targets whose SOURCE_DIR == that subdir
    get_property(_all GLOBAL PROPERTY TARGETS)
    set(_cand "")
    foreach(t IN LISTS _all)
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
      message(FATAL_ERROR
"triton: no library targets were created by '${_abs}'.")
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
      message(FATAL_ERROR
"triton: multiple library targets live under '${_abs}':
  ${_cand}")
    endif()
  endfunction()

endif()
