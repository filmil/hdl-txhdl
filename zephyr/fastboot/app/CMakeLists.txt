# SPDX-License-Identifier: Apache-2.0
#
# The fastboot server for the board (issue 143): the transport-free
# core in `../fastboot.c`, which the host tests run too, on a Zephyr
# TCP socket.
cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(fastboot)

target_include_directories(app PRIVATE ..)
target_sources(app PRIVATE
  src/main.c
  src/jump.S
  ../fastboot.c
)
