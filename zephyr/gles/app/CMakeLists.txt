# SPDX-License-Identifier: Apache-2.0
#
# A Zephyr program drawing through EGL and GL ES 1.1 on Razboj (issue
# 996): the GL library, EGL and the board's machine come from Bazel as
# one static library built for the core, with the C that writes the
# entry points the library does not implement, and Khronos's headers.
cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(gles)

target_sources(app PRIVATE src/main.c ${TXHDL_SOURCES})
target_include_directories(app PRIVATE ${TXHDL_INCLUDES})
target_compile_definitions(app PRIVATE EGL_NO_PLATFORM_SPECIFIC_TYPES)
target_link_libraries(app PRIVATE ${TXHDL_LIBS})
