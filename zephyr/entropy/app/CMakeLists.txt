# SPDX-License-Identifier: Apache-2.0
#
# The entropy source's conditioned output on the serial port, in the
# format trngstat reads (issue 780): the board check of the driver.
cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(entropy)

target_sources(app PRIVATE src/main.c)
