# SPDX-License-Identifier: Apache-2.0
#
# The vendor directory compiles nothing of its own; the SoC being built
# is `SOC_NAME`, and its directory holds whatever code it needs.

add_subdirectory(${SOC_NAME})
