# SPDX-License-Identifier: Apache-2.0

if(CONFIG_ENTROPY_VRETENO)
  zephyr_library_sources(entropy_vreteno.c vreteno_condition.c)
  # The conditioner's SHA-256, from Zephyr's Mbed TLS (issue 780).
  zephyr_library_link_libraries(mbedTLS)
endif()
