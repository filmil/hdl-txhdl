# SPDX-License-Identifier: Apache-2.0

zephyr_library()

# The register maps the parts declare, as the headers
# //tools/regmap writes from them: a driver includes
# <vreteno/regs/<map>.h> rather than typing the offsets (issue 709).
zephyr_include_directories(include)
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

# The entropy driver conditions its words with Mbed TLS's SHA-256
# (issue 780). It cannot select Mbed TLS, since Mbed TLS can select
# ENTROPY_GENERATOR, which the driver depends on, and Kconfig calls that
# a loop; so an image that has the driver says CONFIG_MBEDTLS=y, and one
# that forgets is an error with a reason here.
if(CONFIG_ENTROPY_VRETENO AND NOT CONFIG_MBEDTLS_SHA256)
  message(FATAL_ERROR
    "CONFIG_ENTROPY_VRETENO without CONFIG_MBEDTLS_SHA256: the driver "
    "conditions the source's words with SHA-256. Add CONFIG_MBEDTLS=y "
    "beside CONFIG_ENTROPY_GENERATOR=y.")
endif()
