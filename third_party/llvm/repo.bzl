# SPDX-License-Identifier: Apache-2.0
"""The LLVM release, with the libraries its linker needs beside it.

`toolchains_llvm` unpacks an LLVM release and uses it as it comes.
The release's `lld` needs `libxml2.so.2`, and it looks for it first in
`$ORIGIN/../lib` and then on the machine (issue 1948).
Ubuntu 26.04 has no `libxml2.so.2`, so on GitHub's runners every link
failed before it started.
lld uses libxml2 only to merge Windows manifests, but the loader needs
the library to start the program at all.

This rule unpacks the same release, by the same checksum, and puts four
libraries from conda-forge into its `lib/`, where lld looks first.
conda-forge builds each of them to need nothing but glibc, and to find
the others in its own directory, so the toolchain needs nothing else
from the machine.
`toolchains_llvm` is given this repository through `toolchain_roots`,
and its own BUILD file for an LLVM release is written here unchanged
apart from the linker's file group, which also names the libraries.
"""

# The release that `toolchains_llvm` 1.10.0 fetches for LLVM 20.1.8 on
# x86_64 Linux, with the checksum it keeps for it.
_LLVM = struct(
    url = "https://github.com/llvm/llvm-project/releases/download/llvmorg-20.1.8/LLVM-20.1.8-Linux-X64.tar.xz",
    sha256 = "1ead36b3dfcb774b57be530df42bec70ab2d239fbce9889447c7a29a4ddc1ae6",
    strip_prefix = "LLVM-20.1.8-Linux-X64",
    major = "20",
)

_CONDA = "https://conda.anaconda.org/conda-forge/linux-64/"

# The libraries, each with the conda-forge package that holds it.
# libxml2 is the build without ICU, so the closure stops at these four.
_LIBS = [
    struct(
        lib = "libxml2.so.2",
        pkg = "libxml2-2.13.9-h2cb61b6_0",
        sha256 = "87ba9b4620cbc7e66a67da5008476c81e8218c3affba26a520b6dcd468c80173",
    ),
    struct(
        lib = "libiconv.so.2",
        pkg = "libiconv-1.18-h0cb94f2_3",
        sha256 = "f943117edb9cd4d9c61cc972eee5a34291dc55ea7a6e9e38da104995841cbcb6",
    ),
    struct(
        lib = "liblzma.so.5",
        pkg = "liblzma-5.8.3-hb03c661_1",
        sha256 = "9787df8c22a59c9a70d3e5a10db9ad663485e75e9ccc3f09bd092cb7b95e0dab",
    ),
    struct(
        lib = "libz.so.1",
        pkg = "libzlib-1.3.2-h25fd6f3_3",
        sha256 = "eb8a0db0aa570124f7d2a93d7c7f596e3390df5e047818d873baad32985fc736",
    ),
]

# The line of the template that the libraries are added after.
_LD_ANCHOR = '        "bin/ld64.lld",\n'

def _llvm_dist_impl(ctx):
    ctx.download_and_extract(
        url = _LLVM.url,
        sha256 = _LLVM.sha256,
        stripPrefix = _LLVM.strip_prefix,
    )

    # A `.conda` file is a zip archive, and the files are in a
    # `tar.zst` inside it, so each package is unpacked twice into a
    # scratch directory, and only its library is kept.
    for lib in _LIBS:
        scratch = "unpack/" + lib.pkg
        ctx.download(
            url = _CONDA + lib.pkg + ".conda",
            output = scratch + "/pkg.conda",
            sha256 = lib.sha256,
        )
        ctx.extract(scratch + "/pkg.conda", output = scratch, type = "zip")
        ctx.extract(scratch + "/pkg-" + lib.pkg + ".tar.zst", output = scratch + "/tree")
        dest = "lib/" + lib.lib
        if ctx.path(dest).exists:
            fail("%s is already in the LLVM release" % dest)

        # The library's name is a symlink to the versioned file. The
        # file itself is moved, under the name the loader asks for.
        ctx.rename(ctx.path(scratch + "/tree/lib/" + lib.lib).realpath, dest)
    ctx.delete("unpack")

    build = ctx.read(Label("@toolchains_llvm//toolchain:BUILD.llvm_repo.tpl"))
    if _LD_ANCHOR not in build:
        fail("toolchains_llvm's BUILD.llvm_repo.tpl changed; the linker's file group is not where it was")
    build = build.replace(
        _LD_ANCHOR,
        _LD_ANCHOR + "".join(['        "lib/%s",\n' % lib.lib for lib in _LIBS]),
    )
    ctx.file("BUILD.bazel", build.format(LLVM_VERSION = _LLVM.major), executable = False)

llvm_dist = repository_rule(
    implementation = _llvm_dist_impl,
    doc = "LLVM 20.1.8 for x86_64 Linux, with lld's libraries in its lib/.",
)
