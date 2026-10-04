#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
#
# Builds the mainline kernel or OpenSBI for Vreteno with the hermetic
# LLVM, and nothing of the system's but the shell's own commands
# (issue 1008).
#
#   build.sh kernel  <src> <fragment> <image out> <config out> <map out>
#   build.sh opensbi <src> <fw_jump.bin out> <fw_jump.elf out>
#
# The tools come from the environment the rule sets: KBUILD_FILES, the
# unpacked Debian packages that give `make`, `flex`, `bison` and `bc`;
# LLVM_FILES, the LLVM release's programs; SYSROOT, the Debian sysroot
# host programs are built against; and PYTHON3, the hermetic Python
# OpenSBI's Kconfig is read with.
set -euo pipefail

# Where a tree begins: the directory above the one that holds a file
# known to be in it.
root_of() {
  local suffix=$1 f
  shift
  for f in "$@"; do
    case $f in */"$suffix") echo "${f%/"$suffix"}"; return ;; esac
  done
  echo "build.sh: no $suffix among the inputs" >&2
  exit 2
}
# shellcheck disable=SC2086
KBUILD_TREE=$(root_of usr/bin/make $KBUILD_FILES)
# shellcheck disable=SC2086
LLVM_ROOT=$(root_of bin/clang $LLVM_FILES)

what=$1
shift
root=$PWD

# The tools, on one path of our own and ahead of the system's, so that
# the fetched ones are the ones found; /usr/bin and /bin stay for the
# shell's own commands, as in the Zephyr build.
bin=$root/.kbin
rm -rf "$bin" && mkdir -p "$bin"
for t in make flex bison bc m4; do
  ln -sf "$root/$KBUILD_TREE/usr/bin/$t" "$bin/$t"
done
for t in clang ld.lld llvm-ar llvm-nm llvm-objcopy llvm-objdump \
  llvm-readelf llvm-strip; do
  ln -sf "$root/$LLVM_ROOT/bin/$t" "$bin/$t"
done
ln -sf "$root/$PYTHON3" "$bin/python3"
export PATH="$bin:/usr/bin:/bin"
export LD_LIBRARY_PATH="$root/$KBUILD_TREE/usr/lib/x86_64-linux-gnu:$root/$KBUILD_TREE/lib/x86_64-linux-gnu"
# bison finds its skeletons and m4 from where its package put them.
export BISON_PKGDATADIR="$root/$KBUILD_TREE/usr/share/bison"
export M4="$bin/m4"
# Host programs, Kbuild's own tools among them, are built against the
# Debian sysroot, not the machine's headers.
host_cc="clang --sysroot=$root/$SYSROOT"
# The same bytes from the same sources, whoever builds them, and
# wherever: a date with no zone is read in the local one, and east of
# UTC 1970-01-01 is before the epoch, which the initramfs refuses.
export TZ=UTC
export KBUILD_BUILD_TIMESTAMP="1970-01-01 00:00:00 UTC"
export KBUILD_BUILD_USER="txhdl"
export KBUILD_BUILD_HOST="txhdl"
jobs=$(nproc)

case $what in
kernel)
  src=$1 frag=$2 image=$3 config=$4 map=$5
  out=$root/.kout
  rm -rf "$out" && mkdir -p "$out"
  k() {
    make -C "$root/$src" O="$out" ARCH=riscv LLVM=1 \
      HOSTCC="$host_cc" HOSTLDFLAGS="-fuse-ld=lld" "$@"
  }
  # The smallest kernel there is, then this machine over it (issue
  # 1074): cutting the rv32 defconfig down stayed at 25 MiB.
  k tinyconfig
  "$root/$src/scripts/kconfig/merge_config.sh" -m -O "$out" \
    "$out/.config" "$root/$frag"
  k olddefconfig
  # Every line of the fragment must have survived the merge: a symbol
  # Kconfig refused, for a dependency the fragment did not meet, is
  # silently dropped otherwise. Every such line is named, then the
  # build stops, so one run shows them all.
  dropped=0
  while IFS= read -r line; do
    case $line in "" | "#"*) continue ;; esac
    if [[ $line == *"=n" ]]; then
      sym=${line%=n}
      if grep -q "^$sym=" "$out/.config"; then
        echo "the fragment asks for $line and the config has" \
          "$(grep "^$sym=" "$out/.config")" >&2
        dropped=1
      fi
    elif ! grep -qx "$line" "$out/.config"; then
      echo "the fragment asks for $line and the config does not have it" >&2
      dropped=1
    fi
  done < "$root/$frag"
  [ "$dropped" -eq 0 ] || exit 1
  k -j"$jobs" Image
  cp "$out/arch/riscv/boot/Image" "$image"
  cp "$out/.config" "$config"
  # Where the kernel ends in memory, past the Image: its BSS and early
  # page tables, which the boot image (issue 1019) must leave free.
  cp "$out/System.map" "$map"
  ;;
opensbi)
  src=$1 bin_out=$2 elf_out=$3
  out=$root/.sout
  rm -rf "$out" && mkdir -p "$out"
  # The generic platform, for a core of RV32IMA with the CSRs and no
  # floating point (soft float, `ilp32`), in the boot image's layout
  # (issue 1019): the shim at the DDR3's base, OpenSBI 512 KiB above
  # it, and the kernel 4 MiB in, the alignment an RV32 kernel's first
  # megapage needs. No FW_JUMP_FDT_ADDR, so the kernel gets the device
  # tree the shim passed in a1, where the image put it.
  make -C "$root/$src" O="$out" LLVM=1 PLATFORM=generic \
    PLATFORM_RISCV_XLEN=32 PLATFORM_RISCV_ISA=rv32ima_zicsr_zifencei \
    PLATFORM_RISCV_ABI=ilp32 FW_TEXT_START=0x40080000 \
    FW_JUMP=y FW_JUMP_ADDR=0x40400000 \
    -j"$jobs"
  cp "$out/platform/generic/firmware/fw_jump.bin" "$bin_out"
  cp "$out/platform/generic/firmware/fw_jump.elf" "$elf_out"
  ;;
*)
  echo "build.sh: kernel or opensbi, not $what" >&2
  exit 2
  ;;
esac
