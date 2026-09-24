# SPDX-License-Identifier: Apache-2.0

zephyr_library()
add_subdirectory(drivers)

# A network build takes its random numbers from the entropy source or
# it does not build. Zephyr's test generator is a counter, and a stack
# whose sequence numbers come from a counter is the oldest spoofing
# hole there is; the build that took it succeeded and was wrong, which
# is the bad state (issue 458). So the port refuses: a stack with no
# entropy driver is an error with a reason rather than an image.
if(CONFIG_NETWORKING AND NOT CONFIG_ENTROPY_HAS_DRIVER)
  message(FATAL_ERROR
    "CONFIG_NETWORKING without an entropy driver: the stack would take "
    "its random numbers from a counter. Enable the entropy source "
    "(CONFIG_ENTROPY_GENERATOR=y, which enables ENTROPY_VRETENO from "
    "the device tree) rather than CONFIG_TEST_RANDOM_GENERATOR.")
endif()
