# SPDX-License-Identifier: Apache-2.0
"""The boot image as a rule (issue 1019).

`boot_image(name, opensbi, kernel, system_map, initramfs, bootargs,
model)` writes NAME.bin: the initramfs's range from the kernel's map, the device
tree with that range and the command line in `/chosen`, compiled by
`dtc`, and the image packed by `//tools/bootimg`. Fastboot takes it
whole: `fastboot boot NAME.bin`.

`bootargs` defaults to the device tree's own, `earlycon console=ttySIF0`
(issue 1125). `model = True` adds `mem=64M`, for a boot on the machine
model: the kernel then sets up 64 MiB rather than the board's gigabyte,
which is most of what it does before devtmpfs. The board boots with all
of its memory, so the board's image leaves it off.

The initramfs goes in gzipped, by `//tools/gzip` with no timestamp, so
the image is the same every build; the kernel unpacks it
itself (`CONFIG_RD_GZIP`). With the parts packed and the shim moving
each into place (issue 1201), the image is the parts and nothing else.
"""

BOOTARGS = "earlycon console=ttySIF0"

def boot_image(
        name,
        opensbi,
        kernel,
        system_map,
        initramfs,
        bootargs = BOOTARGS,
        model = False,
        framebuffer = None,
        harts = 2,
        **kwargs):
    """A boot image. `framebuffer`, an address in the DDR3 as hex text,
    puts a simple-framebuffer node over the scanout's frame there in the
    tree and has the shim show it, for a console on HDMI (issue 1440).
    `harts` is how many harts the tree names, the board's two or one
    (issue 1408)."""
    if model:
        bootargs = bootargs + " mem=64M"
    # Every step carries the image's tags, so that an image of manual
    # inputs, a kernel built from source say, is manual all the way and
    # `bazel build //...` does not reach it through a step.
    tags = kwargs.get("tags", [])
    native.genrule(
        name = name + "_initramfs_gz",
        srcs = [initramfs],
        outs = [name + ".cpio.gz"],
        cmd = "$(location //tools/gzip) $(location " + initramfs + ") $@",
        tools = ["//tools/gzip"],
        tags = tags,
    )
    initramfs = name + ".cpio.gz"
    native.genrule(
        name = name + "_layout",
        srcs = [system_map, initramfs],
        outs = [name + ".layout"],
        cmd = "$(location //tools/bootimg) layout" +
              " --system-map $(location " + system_map + ")" +
              " --initramfs $(location " + initramfs + ") > $@",
        tools = ["//tools/bootimg"],
        tags = tags,
    )
    native.genrule(
        name = name + "_dts",
        srcs = [name + ".layout"],
        outs = [name + ".dts"],
        cmd = "$(location //tools/devtree) --initrd $$(cat $(location " +
              name + ".layout)) --bootargs '" + bootargs + "'" +
              (" --framebuffer " + framebuffer if framebuffer else "") +
              " --harts " + str(harts) +
              " > $@",
        tools = ["//tools/devtree"],
        tags = tags,
    )
    native.genrule(
        name = name + "_dtb",
        srcs = [name + ".dts"],
        outs = [name + ".dtb"],
        cmd = "$(location @dtc//:dtc) -I dts -O dtb -o $@ $(location " +
              name + ".dts) 2> $@.log; s=$$?; cat $@.log >&2; " +
              "[ $$s -eq 0 ] && [ ! -s $@.log ]",
        tools = ["@dtc//:dtc"],
        tags = tags,
    )
    native.genrule(
        name = name,
        srcs = [opensbi, kernel, system_map, initramfs, name + ".dtb"],
        outs = [name + ".bin"],
        cmd = "$(location //tools/bootimg) pack" +
              " --opensbi $(location " + opensbi + ")" +
              " --dtb $(location " + name + ".dtb)" +
              " --kernel $(location " + kernel + ")" +
              " --system-map $(location " + system_map + ")" +
              " --initramfs $(location " + initramfs + ")" +
              (" --scanout " + framebuffer if framebuffer else "") +
              " -o $@",
        tools = ["//tools/bootimg"],
        **kwargs
    )
