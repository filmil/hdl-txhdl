# SPDX-License-Identifier: Apache-2.0
"""Building a Zephyr image with this repository's own toolchain.

Zephyr's build is CMake, Kconfig and its own Python, and none of that
becomes Bazel here. What Bazel does is hand that build every tool it
needs, pinned by checksum, and take the image out the other end, so
that the port under `//zephyr` is compiled by `bazel test //...`
rather than by a reader following a README (issue 390).

The flag that matters is `-DZEPHYR_MODULES`. Zephyr's own
documentation says `ZEPHYR_EXTRA_MODULES`, and that does nothing
here: Zephyr collects it and hands it to a script it runs only
`if(WEST OR ZEPHYR_MODULES)`. A hermetic build has no `west` by
construction, so a rule written from the documented flag would fetch
Zephyr, build an ELF, pass, and be testing a Zephyr with none of this
repository in it.
"""

def _zephyr_image_impl(ctx):
    if bool(ctx.attr.sample) == bool(ctx.attr.app):
        fail("give exactly one of `sample` and `app`")
    elf = ctx.actions.declare_file(ctx.label.name + ".elf")
    binary = ctx.actions.declare_file(ctx.label.name + ".bin")
    config = ctx.actions.declare_file(ctx.label.name + ".config")

    # The module's root is the package the port lives in, which is
    # what CMake wants; a file's dirname would be whichever file the
    # glob happened to put first.
    module = ctx.attr.module.label.package

    inputs = depset(
        ctx.files.module + ctx.files._cmake + ctx.files._ninja +
        ctx.files._dtc + ctx.files._python + ctx.files._py_deps,
        transitive = [depset(ctx.files._zephyr), depset(ctx.files._gcc)],
    )

    # The devicetree tooling's packages: the `site-packages` directory
    # of each, taken from the files the pip rules hand over rather than
    # searched for, so that the path is the same in a sandbox and out
    # of one (issue 721).
    site = []
    for f in ctx.files._py_deps:
        at = f.path.find("/site-packages/")
        if at >= 0:
            d = f.path[:at + len("/site-packages")]
            if d not in site:
                site.append(d)
    if not site:
        fail("no site-packages among the Python dependencies")

    # The build runs where Bazel put it, so every path handed to CMake
    # is made absolute from the execution root rather than assumed.
    ctx.actions.run_shell(
        inputs = inputs,
        outputs = [elf, binary, config],
        command = _SCRIPT,
        env = {
            "ZEPHYR_ROOT": ctx.files._zephyr[0].owner.workspace_root,
            "CMAKE": ctx.files._cmake[0].path,
            "NINJA": ctx.files._ninja[0].path,
            "DTC": ctx.files._dtc[0].path,
            "PYTHON": ctx.files._python[0].path,
            "PY_SITE": ":".join(site),
            "GCC_BIN": ctx.files._gcc_bin[0].dirname,
            "MODULE_DIR": module,
            "BOARD": ctx.attr.board,
            "SAMPLE": ctx.attr.sample,
            "APP": ctx.attr.app,
            "EXTRA_CONF": (
                ctx.file.conf.path if ctx.file.conf else ""
            ),
            "OUT_ELF": elf.path,
            "OUT_BIN": binary.path,
            "OUT_CONFIG": config.path,
        },
        mnemonic = "ZephyrImage",
        progress_message = "Building Zephyr %s for %s" % (
            ctx.attr.sample or ctx.attr.app,
            ctx.attr.board,
        ),
    )
    # Named groups as well as the default, so that a check can ask for
    # the generated configuration on its own: what is worth asserting
    # on is that file and not the image beside it.
    return [
        DefaultInfo(files = depset([elf, binary, config])),
        OutputGroupInfo(
            bin = depset([binary]),
            config = depset([config]),
            elf = depset([elf]),
        ),
    ]

_SCRIPT = r"""
set -eu
root=$(pwd)

# Where Bazel unpacked Zephyr. Taken from the fetched repository's own
# root rather than searched for: a `find` for a marker file picks up
# Zephyr's test data, which has trees that look like a Zephyr.
zbase=$root/$ZEPHYR_ROOT

# The tools, on one path of our own, so nothing of the system's is
# reachable even if it is installed.
bin=$root/.zbin
rm -rf "$bin" && mkdir -p "$bin"
ln -sf "$root/$CMAKE" "$bin/cmake"
ln -sf "$root/$NINJA" "$bin/ninja"
ln -sf "$root/$DTC" "$bin/dtc"
ln -sf "$root/$PYTHON" "$bin/python3"
ln -sf "$root/$PYTHON" "$bin/python"
export PATH="$bin:$root/$GCC_BIN:/usr/bin:/bin"

# The devicetree tooling, where the pip rules put it: each package's
# directory, as the rule found it among its inputs.
PYTHONPATH=""
IFS=: read -r -a dirs <<< "$PY_SITE"
for d in "${dirs[@]}"; do
  PYTHONPATH="$PYTHONPATH${PYTHONPATH:+:}$root/$d"
done
export PYTHONPATH

export ZEPHYR_BASE="$zbase"
export ZEPHYR_TOOLCHAIN_VARIANT=cross-compile
export CROSS_COMPILE="$root/$GCC_BIN/riscv-none-elf-"

build=$root/.zbuild
rm -rf "$build"

# `ZEPHYR_MODULES` and not `ZEPHYR_EXTRA_MODULES`: see this file's
# module docstring. The roots are given as well, since the board and
# the SoC live here rather than in Zephyr's tree.
conf_arg=""
if [ -n "$EXTRA_CONF" ]; then
  conf_arg="-DEXTRA_CONF_FILE=$root/$EXTRA_CONF"
fi

# An application of this repository is inside the module, one of
# Zephyr's own samples inside Zephyr.
src="$zbase/$SAMPLE"
if [ -n "$APP" ]; then
  src="$root/$MODULE_DIR/$APP"
fi

# The build runs in a sandbox whose path changes on every run, and the
# compiler writes the paths it was given into the debug information and
# into `__FILE__`. Mapped to `.`, every one of them is relative to the
# execution root, the build directory's among them, so two builds of
# one tree give one `.elf` (issue 711). The compiler's own headers are
# found through the real path of its executable, in Bazel's
# repository cache, which is this machine's and not the tree's; that is
# mapped to a name too.
gcc_root=$(dirname "$(dirname "$(readlink -f "$root/$GCC_BIN/riscv-none-elf-gcc")")")
remap="-ffile-prefix-map=$root=. -ffile-prefix-map=$gcc_root=riscv-none-elf-gcc"
# CMake reaches some of Zephyr's sources, and of the module's, through
# their real paths, where Bazel keeps them: Zephyr in the repository
# cache, the module in the workspace. Each is found from a file's real
# path, since in a sandbox only the files are links, and mapped to the
# name it has under the execution root.
zreal=$(dirname "$(readlink -f "$zbase/VERSION")")
mreal=$(dirname "$(dirname "$(readlink -f "$root/$MODULE_DIR/zephyr/module.yml")")")
remap="$remap -ffile-prefix-map=$zreal=./$ZEPHYR_ROOT"
remap="$remap -ffile-prefix-map=$mreal=./$MODULE_DIR"

"$root/$CMAKE" -B "$build" -S "$src" -G Ninja \
  -DBOARD="$BOARD" $conf_arg \
  -DEXTRA_CFLAGS="$remap" \
  -DEXTRA_CXXFLAGS="$remap" \
  -DEXTRA_AFLAGS="$remap" \
  -DBOARD_ROOT="$root/$MODULE_DIR" \
  -DSOC_ROOT="$root/$MODULE_DIR" \
  -DDTS_ROOT="$root/$MODULE_DIR" \
  -DZEPHYR_MODULES="$root/$MODULE_DIR" \
  > "$build.log" 2>&1 || { cat "$build.log"; exit 1; }

"$root/$CMAKE" --build "$build" >> "$build.log" 2>&1 || {
  cat "$build.log"; exit 1;
}

cp "$build/zephyr/zephyr.elf" "$root/$OUT_ELF"
cp "$build/zephyr/zephyr.bin" "$root/$OUT_BIN"
# Kconfig names the module by its absolute path, on the comment lines
# that open and close its section; the path is the sandbox's, and is
# taken off so that the file says the module's place in the tree.
sed "s|$root/||g" "$build/zephyr/.config" > "$root/$OUT_CONFIG"
"""

zephyr_image = rule(
    implementation = _zephyr_image_impl,
    doc = "A Zephyr image for a board of this repository's, built " +
          "with the fetched CMake, ninja, devicetree compiler, " +
          "Python and RISC-V compiler, and nothing of the system's.",
    attrs = {
        "board": attr.string(
            doc = "The board, as `-DBOARD` takes it.",
            mandatory = True,
        ),
        "module": attr.label(
            doc = "The port: this repository as a Zephyr module.",
            allow_files = True,
            mandatory = True,
        ),
        "conf": attr.label(
            doc = "An extra Kconfig fragment merged into the build, " +
                  "for turning on what a sample does not.",
            allow_single_file = [".conf"],
        ),
        "sample": attr.string(
            doc = "The application, as a path inside Zephyr's tree. " +
                  "Exactly one of this and `app`.",
        ),
        "app": attr.string(
            doc = "The application, as a path inside the module, for " +
                  "one this repository writes (issue 143).",
        ),
        "_cmake": attr.label(
            default = "@cmake_host//:cmake",
            allow_files = True,
        ),
        "_dtc": attr.label(default = "@dtc//:dtc", allow_files = True),
        "_gcc": attr.label(default = "@riscv_none_elf_gcc//:all"),
        "_gcc_bin": attr.label(
            default = "@riscv_none_elf_gcc//:objcopy",
            allow_files = True,
        ),
        "_ninja": attr.label(default = "@ninja//:ninja", allow_files = True),
        "_py_deps": attr.label_list(
            default = [
                "@zephyr_py//pyyaml",
                "@zephyr_py//pykwalify",
                "@zephyr_py//pyelftools",
                "@zephyr_py//packaging",
                "@zephyr_py//anytree",
                "@zephyr_py//intelhex",
                "@zephyr_py//six",
                "@zephyr_py//docopt",
                "@zephyr_py//python_dateutil",
                "@zephyr_py//ruamel_yaml",
                "@zephyr_py//ruamel_yaml_clib",
            ],
        ),
        "_python": attr.label(
            default = "@python_3_12_host//:python",
            allow_files = True,
        ),
        "_zephyr": attr.label(default = "@zephyr//:all"),
    },
)
