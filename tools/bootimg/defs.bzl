# SPDX-License-Identifier: Apache-2.0
"""The boot image as a rule (issue 1019).

`boot_image(name, opensbi, kernel, system_map, initramfs, bootargs)`
writes NAME.bin: the initramfs's range from the kernel's map, the device
tree with that range and the command line in `/chosen`, compiled by
`dtc`, and the image packed by `//tools/bootimg`. Fastboot takes it
whole: `fastboot boot NAME.bin`.
"""

def boot_image(name, opensbi, kernel, system_map, initramfs, bootargs, **kwargs):
    native.genrule(
        name = name + "_layout",
        srcs = [system_map, initramfs],
        outs = [name + ".layout"],
        cmd = "$(location //tools/bootimg) layout" +
              " --system-map $(location " + system_map + ")" +
              " --initramfs $(location " + initramfs + ") > $@",
        tools = ["//tools/bootimg"],
    )
    native.genrule(
        name = name + "_dts",
        srcs = [name + ".layout"],
        outs = [name + ".dts"],
        cmd = "$(location //tools/devtree) --initrd $$(cat $(location " +
              name + ".layout)) --bootargs '" + bootargs + "' > $@",
        tools = ["//tools/devtree"],
    )
    native.genrule(
        name = name + "_dtb",
        srcs = [name + ".dts"],
        outs = [name + ".dtb"],
        cmd = "$(location @dtc//:dtc) -I dts -O dtb -o $@ $(location " +
              name + ".dts) 2> $@.log; s=$$?; cat $@.log >&2; " +
              "[ $$s -eq 0 ] && [ ! -s $@.log ]",
        tools = ["@dtc//:dtc"],
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
              " --initramfs $(location " + initramfs + ") -o $@",
        tools = ["//tools/bootimg"],
        **kwargs
    )
