cmake_minimum_required(VERSION 3.8)

project(lwip C)

#set (BUILD_SHARED_LIBS ON)

set(LWIP_DIR ${CMAKE_CURRENT_SOURCE_DIR}/../../..)
set(LWIP_CONTRIB_DIR ${LWIP_DIR}/contrib)
#include(${LWIP_DIR}/contrib/ports/CMakeCommon.cmake)

set (LWIP_DEFINITIONS -DLWIP_DEBUG)
set (LWIP_INCLUDE_DIRS
    "${LWIP_DIR}/src/include"
    "${LWIP_CONTRIB_DIR}/"
    "${LWIP_CONTRIB_DIR}/ports/tunlib/include"
    "${CMAKE_CURRENT_SOURCE_DIR}/"
)

set (LWIP_EXCLUDE_SLIPIF TRUE)
include(${LWIP_DIR}/src/Filelists.cmake)

set(lwipcontribtun_SRCS
    ${LWIP_CONTRIB_DIR}/ports/tunlib/sys_arch.c
)

add_library(lwip ${lwipcore_SRCS} ${lwipcontribtun_SRCS})

target_compile_options(lwip PRIVATE ${LWIP_COMPILER_FLAGS})
target_compile_definitions(lwip PRIVATE ${LWIP_DEFINITIONS} ${LWIP_MBEDTLS_DEFINITIONS})
target_include_directories(lwip PRIVATE ${LWIP_INCLUDE_DIRS} ${LWIP_MBEDTLS_INCLUDE_DIRS})
target_link_libraries(lwip ${LWIP_SANITIZER_LIBS})

find_library(LIBPTHREAD pthread)
target_link_libraries(lwip ${LIBPTHREAD})
