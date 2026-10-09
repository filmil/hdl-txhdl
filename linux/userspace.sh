#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
#
# Builds the userspace for Linux on Vreteno: BusyBox on musl, static, for
# rv32imac at the soft-float ABI ilp32, with the hermetic LLVM, and an
# initramfs that holds it (issue 1018).
#
#   userspace.sh sysroot   <compiler-rt src> <musl src> <linux src> <sysroot.tar out>
#   userspace.sh busybox   <sysroot.tar> <busybox src> <fragment> <busybox out> <config out>
#   userspace.sh initramfs <busybox> <init> <linux src> <initramfs.cpio out>
#
# The tools come from the environment the rule sets, as for build.sh:
# KBUILD_FILES, the Debian packages that give make, bzip2 and the rest;
# LLVM_FILES, the LLVM release's programs; and SYSROOT, the Debian sysroot
# host programs are built against.
set -euo pipefail

# Where a tree begins, as in build.sh.
root_of() {
  local suffix=$1 f
  shift
  for f in "$@"; do
    case $f in */"$suffix") echo "${f%/"$suffix"}"; return ;; esac
  done
  echo "userspace.sh: no $suffix among the inputs" >&2
  exit 2
}
# shellcheck disable=SC2086
KBUILD_TREE=$(root_of usr/bin/make $KBUILD_FILES)
# shellcheck disable=SC2086
LLVM_ROOT=$(root_of bin/clang $LLVM_FILES)

what=$1
shift
root=$PWD

# The tools on a path of our own, ahead of the system's, as in build.sh.
bin=$root/.ubin
rm -rf "$bin" && mkdir -p "$bin"
for t in make flex bison bc m4 bzip2; do
  ln -sf "$root/$KBUILD_TREE/usr/bin/$t" "$bin/$t"
done
for t in clang ld.lld llvm-ar llvm-nm llvm-objcopy llvm-objdump \
  llvm-ranlib llvm-readelf llvm-strip; do
  ln -sf "$root/$LLVM_ROOT/bin/$t" "$bin/$t"
done
export PATH="$bin:/usr/bin:/bin"
export LD_LIBRARY_PATH="$root/$KBUILD_TREE/usr/lib/x86_64-linux-gnu:$root/$KBUILD_TREE/lib/x86_64-linux-gnu"
export BISON_PKGDATADIR="$root/$KBUILD_TREE/usr/share/bison"
export M4="$bin/m4"
host_cc="clang --sysroot=$root/$SYSROOT"
# The same bytes from the same sources, whoever builds them; a date with
# no zone is negative east of Greenwich, as build.sh found.
export TZ=UTC
export SOURCE_DATE_EPOCH=0
export KBUILD_BUILD_TIMESTAMP="1970-01-01 00:00:00 UTC"
export KBUILD_BUILD_USER="txhdl"
export KBUILD_BUILD_HOST="txhdl"
jobs=$(nproc)

# The core's target: 32 bits, the multiply, atomic and compressed
# extensions, and no floating point unit, so floating point is done in
# software and passed in integer registers.
target="--target=riscv32-unknown-linux-musl -march=rv32imac -mabi=ilp32"
# The sandbox's own directory, which differs from one build to the next,
# is cut out of every path a compile records, such as a `__FILE__` in an
# assertion's message, so the objects are the same bytes each build.
target="$target -ffile-prefix-map=$root/="
# The compiler's own headers, which a resource directory of ours has to
# carry beside the builtins it adds.
clang_include=$(ls -d "$root/$LLVM_ROOT"/lib/clang/*/include)
# What a tar of the build's records about each file: nothing that changes
# from one build to the next.
tar_flags=(--sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner)

# The compiler for the core, once the sysroot and the resource directory
# exist: clang finds musl's start files and library in the sysroot, and
# compiler-rt's builtins and start and end objects in the resource
# directory, so a plain `-static` link needs nothing named by hand.
cross_cc() {
  echo "clang $target --sysroot=$1 -resource-dir=$2" \
    "--rtlib=compiler-rt --unwindlib=none -fuse-ld=lld"
}
# What a dynamic program links with: musl's loader by musl's own name
# for this ABI, soft-float, which the driver's default lacks (issue
# 1439). Not for a static program, into which lld would write the
# loader's name all the same.
dyn_ld="-Wl,--dynamic-linker=/lib/ld-musl-riscv32-sf.so.1"

# The sysroot tar holds the sysroot and the resource directory's library
# half; the resource directory's headers are linked back in after.
unpack_sysroot() {
  local tarball=$1 into=$2
  rm -rf "$into" && mkdir -p "$into"
  tar -xf "$tarball" -C "$into"
  ln -sfn "$clang_include" "$into/rd/include"
}

case $what in
sysroot)
  crt=$root/$1 musl=$root/$2 linux=$root/$3 out=$4
  work=$root/.usys
  rm -rf "$work" && mkdir -p "$work/sysroot" "$work/rd" "$work/musl"
  sys=$work/sysroot
  rd=$work/rd
  rtdir=$rd/lib/riscv32-unknown-linux-musl
  mkdir -p "$rtdir"
  ln -sfn "$clang_include" "$rd/include"

  # musl, configured out of its tree: its headers first, which the
  # builtins include, and its library after them, which wants them.
  (cd "$work/musl" && "$musl/configure" --target=riscv32-linux-musl \
    --prefix=/ --syslibdir=/lib \
    CC="clang $target" AR=llvm-ar RANLIB=llvm-ranlib CFLAGS="-O2" \
    LDFLAGS="-fuse-ld=lld" \
    LIBCC="$rtdir/libclang_rt.builtins.a" >/dev/null)
  make -s -C "$work/musl" install-headers DESTDIR="$sys"

  # The kernel's interface headers, which BusyBox includes.
  make -s -C "$linux" O="$work/kheaders" ARCH=riscv LLVM=1 \
    HOSTCC="$host_cc" HOSTLDFLAGS="-fuse-ld=lld" \
    headers_install INSTALL_HDR_PATH="$sys"

  # compiler-rt's builtins for riscv32: the sources its build lists for
  # the target, with the flags it gives them, software int128 included.
  awk -v want=riscv32_SOURCES -f - "$crt/lib/builtins/CMakeLists.txt" \
    >"$work/builtins.txt" <<'EOF'
# The sources a CMake list names, with every ${LIST} in it expanded: the
# first `set(NAME` that opens a list on a line of its own, an item a
# line, up to the `)`. Comments and blank lines are skipped.
/^[ \t]*set\([A-Za-z0-9_]+[ \t]*$/ {
  name = $0
  sub(/^[ \t]*set\(/, "", name)
  sub(/[ \t]*$/, "", name)
  if (name in seen) { skip = 1 } else { seen[name] = 1; skip = 0; n[name] = 0 }
  cur = name
  inlist = 1
  next
}
inlist {
  line = $0
  done = (line ~ /\)/)
  sub(/\).*/, "", line)
  gsub(/^[ \t]+|[ \t]+$/, "", line)
  if (!skip && line != "" && line !~ /^#/) items[cur, ++n[cur]] = line
  if (done) inlist = 0
  next
}
function expand(nm,   i, it) {
  for (i = 1; i <= n[nm]; i++) {
    it = items[nm, i]
    if (it ~ /^\$\{.*\}$/) { sub(/^\$\{/, "", it); sub(/\}$/, "", it); expand(it) }
    else print it
  }
}
END { expand(want) }
EOF
  mkdir -p "$work/builtins"
  builtin_flags=(-O2 -std=c11 -fPIC -fno-builtin -fvisibility=hidden
    -fomit-frame-pointer -DVISIBILITY_HIDDEN -fforce-enable-int128
    -nostdinc -isystem "$sys/include" -isystem "$clang_include")
  n=0
  while read -r src; do
    n=$((n + 1))
    # shellcheck disable=SC2086
    clang $target "${builtin_flags[@]}" -c "$crt/lib/builtins/$src" \
      -o "$work/builtins/$n.o"
  done <"$work/builtins.txt"
  llvm-ar rcsD "$rtdir/libclang_rt.builtins.a" "$work"/builtins/*.o
  # Its start and end objects, which clang links into a program in
  # place of GCC's when the runtime is compiler-rt.
  for which in begin end; do
    # shellcheck disable=SC2086
    clang $target -O2 -std=c11 -fPIC -Wno-pedantic \
      -DCRT_HAS_INITFINI_ARRAY -DEH_USE_FRAME_REGISTRY \
      -nostdinc -isystem "$sys/include" -isystem "$clang_include" \
      -c "$crt/lib/builtins/crt$which.c" -o "$rtdir/clang_rt.crt$which.o"
  done

  make -s -C "$work/musl" -j"$jobs"
  make -s -C "$work/musl" install DESTDIR="$sys"

  rm "$rd/include"
  tar "${tar_flags[@]}" -C "$work" -cf "$out" sysroot rd
  ;;
busybox)
  tarball=$root/$1 src=$root/$2 frag=$root/$3 bin_out=$4 config_out=$5
  work=$root/.ubb
  unpack_sysroot "$tarball" "$work"
  out=$work/out
  mkdir -p "$out"
  b() {
    make -C "$src" O="$out" CC="$(cross_cc "$work/sysroot" "$work/rd")" \
      HOSTCC="$host_cc -fuse-ld=lld" AR=llvm-ar NM=llvm-nm \
      STRIP=llvm-strip OBJCOPY=llvm-objcopy OBJDUMP=llvm-objdump "$@"
  }
  b -s defconfig >/dev/null
  # The fragment over the defaults: each line replaces the option's line,
  # and every line must be in the configuration afterwards, since an
  # option BusyBox does not know is dropped without a word.
  while IFS= read -r line; do
    case $line in "" | "##" | "## "*) continue ;; esac
    sym=${line#"# "}
    sym=${sym%%[= ]*}
    sed -i -e "/^$sym=/d" -e "/^# $sym is not set/d" "$out/.config"
    echo "$line" >>"$out/.config"
  done <"$frag"
  # `yes` dies of the pipe closing when oldconfig is done, which is
  # how it ends and not a failure.
  { yes "" || true; } | b -s oldconfig >/dev/null
  while IFS= read -r line; do
    case $line in "" | "##" | "## "*) continue ;; esac
    if ! grep -qxF "$line" "$out/.config"; then
      echo "the fragment asks for '$line' and the configuration does" \
        "not have it" >&2
      exit 1
    fi
  done <"$frag"
  b -s -j"$jobs"
  cp "$out/busybox" "$bin_out"
  cp "$out/.config" "$config_out"
  ;;
initramfs)
  busybox=$root/$1 init=$root/$2 linux=$root/$3 smpcount=$root/$4 out=$5
  work=$root/.uinit
  rm -rf "$work" && mkdir -p "$work"
  # The kernel's own packer, built for the host: it writes the newc
  # format the kernel unpacks, device nodes included, from a list, so
  # nothing here has to run as root.
  $host_cc -fuse-ld=lld -O2 "$linux/usr/gen_init_cpio.c" -o "$work/gen_init_cpio"
  cat >"$work/list" <<EOF
dir /dev 0755 0 0
nod /dev/console 0600 0 0 c 5 1
nod /dev/null 0666 0 0 c 1 3
dir /bin 0755 0 0
dir /sbin 0755 0 0
dir /usr 0755 0 0
dir /usr/bin 0755 0 0
dir /usr/sbin 0755 0 0
dir /proc 0755 0 0
dir /sys 0755 0 0
dir /tmp 1777 0 0
dir /root 0700 0 0
file /bin/busybox $busybox 0755 0 0
slink /bin/sh busybox 0777 0 0
file /init $init 0755 0 0
file /bin/smpcount $smpcount 0755 0 0
EOF
  # Every entry at time nought, so the archive is the same each build.
  "$work/gen_init_cpio" -t 0 "$work/list" >"$out"
  ;;
nfsboot)
  # The initramfs of the image whose root is on NFS (issue 1438): the
  # console and nothing to run, so the kernel, finding no /init, goes on
  # to mount the root its command line names.
  linux=$root/$1 out=$2
  work=$root/.unfs
  rm -rf "$work" && mkdir -p "$work"
  $host_cc -fuse-ld=lld -O2 "$linux/usr/gen_init_cpio.c" -o "$work/gen_init_cpio"
  printf '%s\n' 'dir /dev 0755 0 0' 'nod /dev/console 0600 0 0 c 5 1' \
    >"$work/list"
  "$work/gen_init_cpio" -t 0 "$work/list" >"$out"
  ;;
roottar)
  # The same system as the initramfs, as a tree for the NFS export on
  # srv (issue 1438): a tar, written by Python's tarfile so that the
  # device nodes and root's ownership are in it without running as root,
  # and unpacked there with `tar -xpf` as root. Every entry at time
  # nought, so the archive is the same each build.
  # With musl's shared library, Dropbear and a dynamic program beside
  # BusyBox (issue 1439), and the accounts an ssh login reads.
  busybox=$root/$1 init=$root/$2 libc=$root/$3 dropbear=$root/$4
  dynhello=$root/$5 out=$6 cpio_out=$7
  "$PYTHON3" - "$busybox" "$init" "$libc" "$dropbear" "$dynhello" "$out" \
    "$cpio_out" <<'PY'
import io
import sys
import tarfile

busybox, init, libc, dropbear, dynhello, out, cpio_out = sys.argv[1:]
# The same tree as a newc archive too, the format the kernel unpacks as
# an initramfs, so the model, which has no NFS server, boots it.
cpio = bytearray()
ino = [0]


def newc(name, mode, data=b"", dev=(0, 0)):
    ino[0] += 1
    nb = name.encode() + b"\0"
    fields = [ino[0], mode, 0, 0, 1, 0, len(data), 0, 0, dev[0], dev[1],
              len(nb), 0]
    cpio.extend(b"070701" + b"".join(b"%08x" % v for v in fields) + nb)
    cpio.extend(b"\0" * (-len(cpio) % 4))
    cpio.extend(data)
    cpio.extend(b"\0" * (-len(cpio) % 4))


with tarfile.open(out, "w", format=tarfile.GNU_FORMAT) as t:

    def add(name, kind, mode, data=b"", target="", dev=(0, 0)):
        i = tarfile.TarInfo(name)
        i.type, i.mode, i.uid, i.gid, i.mtime = kind, mode, 0, 0, 0
        i.uname = i.gname = "root"
        if kind == tarfile.REGTYPE:
            i.size = len(data)
        i.linkname = target
        i.devmajor, i.devminor = dev
        t.addfile(i, io.BytesIO(data) if kind == tarfile.REGTYPE else None)
        kinds = {tarfile.DIRTYPE: 0o040000, tarfile.REGTYPE: 0o100000,
                 tarfile.SYMTYPE: 0o120000, tarfile.CHRTYPE: 0o020000}
        body = target.encode() if kind == tarfile.SYMTYPE else data
        newc(name, kinds[kind] | mode, body, dev)

    for d, m in [("dev", 0o755), ("bin", 0o755), ("sbin", 0o755),
                 ("usr", 0o755), ("usr/bin", 0o755), ("usr/sbin", 0o755),
                 ("proc", 0o755), ("sys", 0o755), ("tmp", 0o1777),
                 ("root", 0o700)]:
        add(d, tarfile.DIRTYPE, m)
    add("dev/console", tarfile.CHRTYPE, 0o600, dev=(5, 1))
    add("dev/null", tarfile.CHRTYPE, 0o666, dev=(1, 3))
    with open(busybox, "rb") as f:
        add("bin/busybox", tarfile.REGTYPE, 0o755, f.read())
    add("bin/sh", tarfile.SYMTYPE, 0o777, target="busybox")
    with open(init, "rb") as f:
        add("init", tarfile.REGTYPE, 0o755, f.read())
    # musl's shared library, and its loader's name for this ABI.
    add("lib", tarfile.DIRTYPE, 0o755)
    with open(libc, "rb") as f:
        add("lib/libc.so", tarfile.REGTYPE, 0o755, f.read())
    add("lib/ld-musl-riscv32-sf.so.1", tarfile.SYMTYPE, 0o777,
        target="libc.so")
    with open(dynhello, "rb") as f:
        add("bin/dynhello", tarfile.REGTYPE, 0o755, f.read())
    # Dropbear: one program, which is what its name calls it.
    with open(dropbear, "rb") as f:
        add("usr/bin/dropbearmulti", tarfile.REGTYPE, 0o755, f.read())
    add("usr/sbin/dropbear", tarfile.SYMTYPE, 0o777,
        target="../bin/dropbearmulti")
    for name in ["dbclient", "dropbearkey", "scp"]:
        add("usr/bin/" + name, tarfile.SYMTYPE, 0o777, target="dropbearmulti")
    # The accounts: root, whose password is no password at all ("*"),
    # so a login is by a key in /root/.ssh/authorized_keys.
    add("etc", tarfile.DIRTYPE, 0o755)
    add("etc/dropbear", tarfile.DIRTYPE, 0o700)
    add("etc/passwd", tarfile.REGTYPE, 0o644, b"root:x:0:0:root:/root:/bin/sh\n")
    add("etc/group", tarfile.REGTYPE, 0o644, b"root:x:0:\n")
    add("etc/shadow", tarfile.REGTYPE, 0o600, b"root:*:0:0:99999:7:::\n")
    add("root/.ssh", tarfile.DIRTYPE, 0o700)
newc("TRAILER!!!", 0)
cpio.extend(b"\0" * (-len(cpio) % 512))
with open(cpio_out, "wb") as f:
    f.write(cpio)
PY
  ;;
dropbear)
  # Dropbear, the ssh server (issue 1439): one program for the server,
  # the client, the key maker and scp, linked against musl's shared
  # library, so it runs through /lib/ld-musl-riscv32-sf.so.1 as any
  # dynamic program on the board does. No zlib, and no login records,
  # which the root does not keep.
  tarball=$root/$1 src=$root/$2 out=$3
  work=$root/.udb
  unpack_sysroot "$tarball" "$work"
  cp -r "$src" "$work/src"
  chmod -R u+w "$work/src"
  (cd "$work/src" && ./configure --host=riscv32-unknown-linux-musl \
    --disable-zlib --disable-lastlog --disable-utmp --disable-utmpx \
    --disable-wtmp --disable-wtmpx --disable-pututline \
    --disable-pututxline --disable-harden \
    CC="$(cross_cc "$work/sysroot" "$work/rd")" AR=llvm-ar \
    RANLIB=llvm-ranlib STRIP=llvm-strip CFLAGS="-O2" LDFLAGS="$dyn_ld" \
    >/dev/null)
  make -s -C "$work/src" -j"$jobs" MULTI=1 \
    PROGRAMS="dropbear dbclient dropbearkey scp"
  llvm-strip -o "$out" "$work/src/dropbearmulti"
  ;;
dynprog)
  # A C program linked against musl's shared library rather than its
  # archive (issue 1439): what the board runs to show that a dynamic
  # program loads and runs.
  tarball=$root/$1 c=$root/$2 out=$3
  work=$root/.udyn
  unpack_sysroot "$tarball" "$work"
  # shellcheck disable=SC2046
  $(cross_cc "$work/sysroot" "$work/rd") $dyn_ld -O2 "$c" -o "$work/prog"
  llvm-strip -o "$out" "$work/prog"
  ;;
staticprog)
  # A C program linked statically against musl, for the initramfs,
  # which has no shared library: the threads of issue 1408.
  tarball=$root/$1 c=$root/$2 out=$3
  work=$root/.ustat
  unpack_sysroot "$tarball" "$work"
  # shellcheck disable=SC2046
  $(cross_cc "$work/sysroot" "$work/rd") -static -O2 "$c" -o "$work/prog"
  llvm-strip -o "$out" "$work/prog"
  ;;
musl_so)
  # musl's shared library out of the sysroot, for the root's /lib, where
  # its loader's name links to it (issue 1439).
  tarball=$root/$1 out=$2
  work=$root/.uso
  rm -rf "$work" && mkdir -p "$work"
  tar -xf "$tarball" -C "$work" sysroot/lib/libc.so
  llvm-strip -o "$out" "$work/sysroot/lib/libc.so"
  ;;
*)
  echo "userspace.sh: sysroot, busybox, initramfs, nfsboot, roottar, dropbear, dynprog, staticprog or musl_so, not $what" >&2
  exit 2
  ;;
esac
