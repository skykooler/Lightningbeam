cmake_minimum_required(VERSION 3.5)
project(NeuralAudioStatic)

# Override add_library to force STATIC even when SHARED is specified
macro(add_library name)
    set(_args ${ARGN})
    list(FIND _args "SHARED" _shared_idx)
    if(NOT _shared_idx EQUAL -1)
        list(REMOVE_AT _args ${_shared_idx})
        _add_library(${name} STATIC ${_args})
    else()
        _add_library(${name} ${_args})
    endif()
endmacro()

add_subdirectory(${NEURALAUDIO_SOURCE_DIR} ${CMAKE_CURRENT_BINARY_DIR}/NeuralAudio)
