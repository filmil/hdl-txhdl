// SPDX-License-Identifier: Apache-2.0
//! The boot image: OpenSBI's `fw_jump`, the kernel, the device tree and
//! the initramfs as one blob for fastboot to load at `0x4000_0000`, with
//! a shim in front that starts OpenSBI as a bootloader would (issue
//! 1019, item M11 of #279).
//!
//! Fastboot jumps to the image's first byte with the registers as it
//! left them, so the image starts with a few instructions: `a0` the
//! hart's number, `a1` the device tree's address, and a jump to
//! OpenSBI. OpenSBI then starts the kernel at `FW_JUMP_ADDR` and hands
//! it the same tree.
//!
//! Before the jump the shim leaves the serial port and the interrupt
//! controller as SiFive's parts leave them at reset, which is what
//! Linux's drivers expect (issue 1136). Our port resets with its
//! receive interrupt enabled, so the serial loader's bytes raise its
//! line, and the controller's gateway keeps the request after the line
//! falls. Linux's driver enables the source before it has registered
//! the port, takes the request at once, and dies on the missing port.
//! So the shim writes the port's `ie` to zero, then claims and
//! completes, in the machine's context, every request the controller
//! holds, and leaves every source disabled at priority zero.
//!
//! The layout, from the image's base:
//!
//! | address       | what                                             |
//! |---------------|--------------------------------------------------|
//! | `0x4000_0000` | the shim                                         |
//! | `0x4008_0000` | `fw_jump`, its `FW_TEXT_START`                   |
//! | `0x4030_0000` | the device tree blob                             |
//! | `0x4040_0000` | the kernel, `FW_JUMP_ADDR`                        |
//! | after `_end`  | the initramfs, on a 64 KiB boundary              |
//!
//! The kernel's extent is its `System.map`'s `_end` less its `_start`,
//! which covers the BSS and the early tables past the end of `Image`,
//! so the initramfs is placed past all of it; and the tree is placed
//! below the kernel, so its `/chosen`, which names the initramfs's range,
//! does not move what it names. The image may not be longer than
//! fastboot will take (issue 1078): the server's `max-download-size`,
//! less the header page stock `fastboot boot` wraps a file in. The
//! packer says by how much if it would be.
//!
//! `bootimg layout --system-map F --initramfs F` prints the initramfs's
//! range, for `//tools/devtree`'s `--initrd`; `bootimg pack` writes the
//! image.

/// The image's base, where fastboot puts it and jumps.
pub const BASE: u32 = 0x4000_0000;
/// OpenSBI's `FW_TEXT_START`, agreed with M1.
pub const OPENSBI: u32 = 0x4008_0000;
/// The device tree blob.
pub const DTB: u32 = 0x4030_0000;
/// The kernel, OpenSBI's `FW_JUMP_ADDR`.
pub const KERNEL: u32 = 0x4040_0000;
/// What the board's fastboot server takes, its `max-download-size`:
/// the staging area of 16 MiB less the page `jump.S` runs from
/// (`zephyr/fastboot/app/src/main.c`, `MAX_DOWNLOAD`). A test reads
/// both from the server's sources.
pub const MAX_DOWNLOAD: u32 = 0x0100_0000 - 0x1000;
/// The header stock `fastboot boot` puts in front of a plain file: one
/// page of a version 0 Android boot image, at the tool's default page
/// size of 2048, with the file after it. The server finds the file at
/// the header's page size (`fb_kernel` in `zephyr/fastboot/fastboot.c`).
pub const HEADER: u32 = 2048;
/// The first byte past the longest image fastboot takes: the file and
/// its header within `MAX_DOWNLOAD`, the file rounded up to a page as
/// the tool pads it.
pub const LIMIT: u32 = BASE + (MAX_DOWNLOAD - HEADER) / HEADER * HEADER;
// The longest image and the tool's header fit the server's download,
// and the image needs no padding past it.
const _: () = assert!(LIMIT - BASE + HEADER <= MAX_DOWNLOAD);
const _: () = assert!((LIMIT - BASE).is_multiple_of(HEADER));
/// The initramfs's alignment.
const ALIGN: u32 = 0x1_0000;

/// The serial port's page, and its `ie` word in it.
pub const UART: u32 = 0x3000;
pub const UART_IE: u32 = 0x10;
/// The interrupt controller, its sources, numbered from one, and the
/// machine context's enable word, and its threshold and claim words.
pub const PLIC: u32 = 0x0c00_0000;
pub const PLIC_SOURCES: u32 = 3;
pub const PLIC_ENABLE: u32 = PLIC + 0x2000;
pub const PLIC_THRESHOLD: u32 = PLIC + 0x20_0000;
pub const PLIC_CLAIM: u32 = 4;

/// The shim's instructions. First the port's `ie` is written zero,
/// and the controller drained: each source at priority one and
/// enabled, the threshold zero, then a claim read and written back
/// until one reads zero, then each source disabled and at priority
/// zero again. Then `a0 = 0`, `a1 = DTB`, `t0 = OPENSBI`, and a jump to
/// `t0`. Every address it names has low twelve bits of zero, or is
/// within twelve bits of one that has, so a `lui` loads each.
pub fn shim() -> Vec<u32> {
    let lui = |rd: u32, imm: u32| (imm & 0xffff_f000) | (rd << 7) | 0x37;
    let addi = |rd: u32, rs: u32, imm: i32| {
        ((imm as u32 & 0xfff) << 20) | (rs << 15) | (rd << 7) | 0x13
    };
    let lw = |rd: u32, rs: u32, off: u32| {
        (off << 20) | (rs << 15) | (2 << 12) | (rd << 7) | 0x03
    };
    let sw = |src: u32, rs: u32, off: u32| {
        ((off >> 5) << 25)
            | (src << 20)
            | (rs << 15)
            | (2 << 12)
            | ((off & 0x1f) << 7)
            | 0x23
    };
    // A branch or a jump of `off` bytes, forwards or back.
    let beq = |rs1: u32, rs2: u32, off: i32| {
        let o = off as u32;
        (((o >> 12) & 1) << 31)
            | (((o >> 5) & 0x3f) << 25)
            | (rs2 << 20)
            | (rs1 << 15)
            | (((o >> 1) & 0xf) << 8)
            | (((o >> 11) & 1) << 7)
            | 0x63
    };
    let jal0 = |off: i32| {
        let o = off as u32;
        (((o >> 20) & 1) << 31)
            | (((o >> 1) & 0x3ff) << 21)
            | (((o >> 11) & 1) << 20)
            | (((o >> 12) & 0xff) << 12)
            | 0x6f
    };
    let (t0, t1, t2) = (5, 6, 7);
    let enable_all: u32 = ((1 << (PLIC_SOURCES + 1)) - 1) & !1;
    let mut w = vec![
        lui(t0, UART),
        sw(0, t0, UART_IE), // ie = 0: the port's line falls
        lui(t0, PLIC),
        addi(t1, 0, 1),
    ];
    for s in 1..=PLIC_SOURCES {
        w.push(sw(t1, t0, 4 * s)); // priority 1
    }
    w.extend([
        lui(t2, PLIC_ENABLE),
        addi(t1, 0, enable_all as i32),
        sw(t1, t2, 0), // every source enabled
        lui(t2, PLIC_THRESHOLD),
        sw(0, t2, 0), // threshold 0
        // Claim until nothing is left, completing each.
        lw(t1, t2, PLIC_CLAIM),
        beq(t1, 0, 12),
        sw(t1, t2, PLIC_CLAIM),
        jal0(-12),
        lui(t2, PLIC_ENABLE),
        sw(0, t2, 0), // every source disabled
    ]);
    for s in 1..=PLIC_SOURCES {
        w.push(sw(0, t0, 4 * s)); // priority 0, as reset leaves it
    }
    w.extend([
        addi(10, 0, 0),    // a0 = 0
        lui(11, DTB),      // a1 = DTB
        lui(t0, OPENSBI),  // t0 = OPENSBI
        (t0 << 15) | 0x67, // jalr x0, 0(t0)
    ]);
    w
}

/// A symbol's address from a `System.map`: `ADDR TYPE NAME` a line.
fn symbol(map: &str, name: &str) -> Option<u32> {
    map.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        let addr = f.next()?;
        let _ = f.next()?;
        (f.next()? == name).then(|| u32::from_str_radix(addr, 16).ok())?
    })
}

/// Where everything lands, and the image's end.
#[derive(Debug, PartialEq)]
pub struct Layout {
    pub initrd: (u32, u32),
    pub end: u32,
}

/// The layout for a kernel whose map is `map` and an initramfs of
/// `initrd_len` bytes, or why it does not fit.
pub fn layout(map: &str, initrd_len: u32) -> Result<Layout, String> {
    let start = symbol(map, "_start").ok_or("System.map has no _start")?;
    let end = symbol(map, "_end").ok_or("System.map has no _end")?;
    let extent = end.checked_sub(start).ok_or("_end is before _start")?;
    let at = (KERNEL + extent).div_ceil(ALIGN) * ALIGN;
    let initrd = (at, at + initrd_len);
    if initrd.1 > LIMIT {
        return Err(format!(
            "the image ends at {:#x}, {:#x} past the {:#x} bytes fastboot \
             takes: the kernel is {extent:#x} with its BSS and the \
             initramfs {initrd_len:#x}",
            initrd.1,
            initrd.1 - LIMIT,
            LIMIT - BASE
        ));
    }
    Ok(Layout {
        initrd,
        end: initrd.1,
    })
}

/// The image, from `BASE` to the initramfs's end.
pub fn pack(
    opensbi: &[u8],
    dtb: &[u8],
    kernel: &[u8],
    initramfs: &[u8],
    layout: &Layout,
) -> Result<Vec<u8>, String> {
    let fits = |what: &str, at: u32, len: usize, next: u32| {
        if at + len as u32 > next {
            Err(format!(
                "{what} is {len:#x} bytes at {at:#x}, past {next:#x}"
            ))
        } else {
            Ok(())
        }
    };
    fits("fw_jump", OPENSBI, opensbi.len(), DTB)?;
    fits("the device tree", DTB, dtb.len(), KERNEL)?;
    fits("the kernel", KERNEL, kernel.len(), layout.initrd.0)?;
    let mut img = vec![0u8; (layout.end - BASE) as usize];
    let mut put = |at: u32, bytes: &[u8]| {
        let o = (at - BASE) as usize;
        img[o..o + bytes.len()].copy_from_slice(bytes);
    };
    let words: Vec<u8> = shim().iter().flat_map(|w| w.to_le_bytes()).collect();
    put(BASE, &words);
    put(OPENSBI, opensbi);
    put(DTB, dtb);
    put(KERNEL, kernel);
    put(layout.initrd.0, initramfs);
    Ok(img)
}

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |flag: &str| -> String {
        let i = args
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag} is required"));
        args.get(i + 1)
            .cloned()
            .unwrap_or_else(|| panic!("{flag} wants a value"))
    };
    let map =
        String::from_utf8(read(&get("--system-map"))).expect("System.map");
    let initramfs = read(&get("--initramfs"));
    let lay = layout(&map, initramfs.len() as u32).unwrap_or_else(|e| {
        eprintln!("bootimg: {e}");
        std::process::exit(1)
    });
    match args.first().map(String::as_str) {
        Some("layout") => {
            println!("{:#010x} {:#010x}", lay.initrd.0, lay.initrd.1)
        }
        Some("pack") => {
            let img = pack(
                &read(&get("--opensbi")),
                &read(&get("--dtb")),
                &read(&get("--kernel")),
                &initramfs,
                &lay,
            )
            .unwrap_or_else(|e| {
                eprintln!("bootimg: {e}");
                std::process::exit(1)
            });
            std::fs::write(get("-o"), img).expect("the image");
        }
        _ => panic!("bootimg layout|pack ..."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vreteno32::machine::Machine;

    /// A map whose kernel spans `extent` bytes.
    fn map(extent: u32) -> String {
        format!(
            "c0000000 T _start\nc0001000 T start_kernel\n{:08x} B _end\n",
            0xc000_0000u32 + extent
        )
    }

    #[test]
    fn the_initramfs_goes_past_the_kernels_end_on_a_64k_boundary() {
        let l = layout(&map(0x48_1234), 0x2_0000).unwrap();
        assert_eq!(l.initrd, (0x4089_0000, 0x408b_0000));
        assert_eq!(l.end, 0x408b_0000);
    }

    #[test]
    fn an_image_past_16_mib_is_refused_with_its_sizes() {
        let e = layout(&map(0x60_0000), 0x70_0000).unwrap_err();
        assert!(e.contains("bytes fastboot takes"), "{e}");
        assert!(e.contains("0x600000") && e.contains("0x700000"), "{e}");
    }

    /// The last byte fastboot takes is allowed and the next is not.
    #[test]
    fn the_image_may_be_as_long_as_fastboot_takes_and_no_longer() {
        let room = LIMIT - 0x4042_0000;
        assert_eq!(layout(&map(0x2_0000), room).unwrap().end, LIMIT);
        assert!(layout(&map(0x2_0000), room + 1).is_err());
    }

    /// `MAX_DOWNLOAD` is what the server's sources make it: the
    /// staging area's size in the overlay, less `JUMP_PAGE`.
    #[test]
    fn max_download_is_the_servers() {
        let text = |p: &str| {
            let r = std::env::var("TEST_SRCDIR").unwrap();
            std::fs::read_to_string(format!("{r}/_main/{p}"))
                .unwrap_or_else(|e| panic!("{p}: {e}"))
        };
        let overlay = text("zephyr/fastboot/app/boards/ax7a200b.overlay");
        let reg = overlay
            .lines()
            .find_map(|l| l.trim().strip_prefix("reg = <"))
            .expect("the staging area's reg");
        let size = reg.trim_end_matches(">;").split_whitespace().nth(1);
        let hex = |s: &str| u32::from_str_radix(s.trim_start_matches("0x"), 16);
        let stage = hex(size.expect("its size")).unwrap();
        let main = text("zephyr/fastboot/app/src/main.c");
        let jump: u32 = main
            .lines()
            .find_map(|l| l.strip_prefix("#define JUMP_PAGE "))
            .expect("JUMP_PAGE")
            .trim()
            .parse()
            .unwrap();
        assert!(
            main.contains("#define MAX_DOWNLOAD (STAGE_SIZE - JUMP_PAGE)"),
            "the server's limit is computed as this test computes it"
        );
        assert_eq!(MAX_DOWNLOAD, stage - jump);
    }

    /// The shim's words are the instructions they claim to be: run in
    /// the machine model, they leave `a0` 0 and `a1` the tree's address
    /// and land at OpenSBI, where a stand-in halts. Every part is where
    /// the layout says, read back from the model's memory.
    #[test]
    fn the_image_boots_in_the_machine_model() {
        let mhalt =
            ((0x7c0u32 << 20) | (1 << 15) | (5 << 12) | 0x73).to_le_bytes();
        let opensbi: Vec<u8> =
            mhalt.iter().copied().chain([0xaa; 60]).collect();
        let dtb = vec![0xd0u8; 100];
        let kernel = vec![0x4bu8; 0x1000];
        let initramfs = vec![0x1au8; 0x200];
        let l = layout(&map(0x2_0000), initramfs.len() as u32).unwrap();
        let img = pack(&opensbi, &dtb, &kernel, &initramfs, &l).unwrap();
        let mut m = Machine::new();
        m.load(BASE, &img);
        m.model.pc = BASE;
        m.model.x[10] = 0x5555;
        m.model.x[11] = 0x6666;
        m.run(100);
        assert!(m.model.halted.is_some(), "the stand-in halted");
        assert_eq!(m.model.pc, OPENSBI + 4, "at OpenSBI, past its halt");
        assert_eq!(m.model.x[10], 0, "a0 is hart 0");
        assert_eq!(m.model.x[11], DTB, "a1 is the tree");
        let b = &m.board;
        use vreteno32::model::Bus;
        assert_eq!(b.load(DTB), Some(0xd0d0_d0d0));
        assert_eq!(b.load(KERNEL), Some(0x4b4b_4b4b));
        assert_eq!(b.load(l.initrd.0), Some(0x1a1a_1a1a));
        assert_eq!(b.load(l.initrd.0 - 4), Some(0), "nothing before it");
    }

    /// The addresses the shim names are the machine's, and the board's.
    #[test]
    fn the_shims_addresses_are_the_machines() {
        use vreteno32::machine::{plic, PLIC_SOURCES as N};
        assert_eq!(UART, vreteno32::isa::UART_BASE);
        assert_eq!(PLIC_SOURCES as usize, N);
        let d = vreteno32::machine::Map::board();
        assert_eq!(d.uart.0, UART);
        assert_eq!(d.plic.0, PLIC);
        assert_eq!(PLIC_ENABLE - PLIC, plic::ENABLE[0]);
        assert_eq!(PLIC_THRESHOLD - PLIC, plic::THRESHOLD[0]);
        assert_eq!(PLIC_THRESHOLD - PLIC + PLIC_CLAIM, plic::CLAIM[0]);
    }

    /// The serial port as the loader leaves it on the board (issue
    /// 1136): its receive interrupt enabled and a byte waiting, so the
    /// controller holds the request before the image runs. The shim
    /// leaves the port's `ie` zero, nothing pending or in service,
    /// every source disabled at priority zero, and the byte still there
    /// to be read; and it still lands at OpenSBI as before.
    #[test]
    fn the_shim_leaves_no_request_from_the_loader() {
        let mhalt =
            ((0x7c0u32 << 20) | (1 << 15) | (5 << 12) | 0x73).to_le_bytes();
        let opensbi: Vec<u8> = mhalt.to_vec();
        let l = layout(&map(0x2_0000), 0x200).unwrap();
        let img =
            pack(&opensbi, &[0xd0; 4], &[0x4b; 4], &[0x1a; 4], &l).unwrap();
        let mut m = Machine::new();
        m.load(BASE, &img);
        m.boot(BASE, DTB);
        m.board.0.borrow_mut().uart.ie = 2;
        m.type_bytes(b"x");
        // The first step gives the controller the port's line.
        m.step();
        {
            let d = m.board.0.borrow();
            assert_ne!(d.plic.pending, 0, "the loader's request is held");
        }
        m.run(200);
        assert_eq!(m.model.pc, OPENSBI + 4, "at OpenSBI, past its halt");
        assert_eq!((m.model.x[10], m.model.x[11]), (0, DTB));
        let d = m.board.0.borrow();
        assert_eq!(d.uart.ie, 0, "the port's interrupts off");
        assert_eq!(d.plic.pending, 0, "nothing pending");
        assert_eq!(d.plic.active, 0, "nothing in service");
        assert_eq!(d.plic.enable, [0, 0], "every source disabled");
        assert!(d.plic.prio.iter().all(|&p| p == 0), "priorities zero");
        assert_eq!(d.uart.rx.len(), 1, "the byte is still to be read");
    }
}
