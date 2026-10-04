// SPDX-License-Identifier: Apache-2.0
//! What the initramfs holds, read back from the archive (issue 1018).
//!
//! The archive is the newc format the kernel unpacks: per entry a header
//! of thirteen eight-digit hexadecimal fields after the magic `070701`,
//! the name, and the data, each padded to four bytes. Nothing here can
//! boot it; that is M9's model. What can be checked without a core is
//! that every file a boot needs is there, as what it should be, and that
//! BusyBox is a program for this core: 32-bit RISC-V, compressed
//! instructions, the soft-float ABI, and static, with no interpreter to
//! look for.
use std::collections::BTreeMap;

/// One entry: its mode, the device's major and minor, and its data.
struct Entry {
    mode: u32,
    major: u32,
    minor: u32,
    data: Vec<u8>,
}

/// The archive's entries by name.
fn entries(bytes: &[u8]) -> BTreeMap<String, Entry> {
    let field = |at: usize, k: usize| {
        let s = std::str::from_utf8(&bytes[at + 6 + 8 * k..at + 14 + 8 * k])
            .expect("a header field is ASCII");
        u32::from_str_radix(s, 16).expect("a header field is hexadecimal")
    };
    let pad = |n: usize| (n + 3) & !3;
    let mut out = BTreeMap::new();
    let mut at = 0;
    loop {
        assert_eq!(&bytes[at..at + 6], b"070701", "a newc header at {at}");
        let (mode, size) = (field(at, 1), field(at, 6) as usize);
        let (major, minor) = (field(at, 9), field(at, 10));
        let namesize = field(at, 11) as usize;
        let name = String::from_utf8(
            bytes[at + 110..at + 110 + namesize - 1].to_vec(),
        )
        .expect("a name is UTF-8");
        let body = pad(at + 110 + namesize);
        let data = bytes[body..body + size].to_vec();
        at = pad(body + size);
        if name == "TRAILER!!!" {
            return out;
        }
        out.insert(
            name,
            Entry {
                mode,
                major,
                minor,
                data,
            },
        );
    }
}

/// The archive the build made, from the path the rule gives.
fn archive() -> BTreeMap<String, Entry> {
    let path = std::env::var("INITRAMFS").expect("INITRAMFS names the archive");
    entries(&std::fs::read(&path).expect("the archive reads"))
}

#[test]
fn every_entry_a_boot_needs_is_there() {
    let files = archive();
    let want = [
        ("dev", 0o40755),
        ("dev/console", 0o20600),
        ("dev/null", 0o20666),
        ("bin", 0o40755),
        ("bin/busybox", 0o100755),
        ("bin/sh", 0o120777),
        ("proc", 0o40755),
        ("sys", 0o40755),
        ("tmp", 0o41777),
        ("init", 0o100755),
    ];
    for (name, mode) in want {
        let e = files
            .get(name)
            .unwrap_or_else(|| panic!("{name} is missing"));
        assert_eq!(e.mode, mode, "{name}'s mode is {:o}", e.mode);
    }
}

#[test]
fn the_devices_are_the_console_and_null() {
    let files = archive();
    let dev = |n: &str| (files[n].major, files[n].minor);
    assert_eq!(dev("dev/console"), (5, 1));
    assert_eq!(dev("dev/null"), (1, 3));
}

#[test]
fn the_shell_is_busybox() {
    // gen_init_cpio stores a link's target with its terminating NUL, and
    // the kernel reads it as the C string it is.
    assert_eq!(archive()["bin/sh"].data, b"busybox\0");
}

#[test]
fn init_says_the_userspace_is_up() {
    let init = String::from_utf8(archive()["init"].data.clone())
        .expect("init is text");
    assert!(init.starts_with("#!/bin/sh\n"));
    assert!(init.contains("echo \"txhdl: userspace is up\""));
    assert!(init.contains("exec /bin/sh"));
}

#[test]
fn busybox_is_a_static_program_for_the_core() {
    let elf = archive()["bin/busybox"].data.clone();
    let u16_at = |at: usize| u16::from_le_bytes([elf[at], elf[at + 1]]);
    let u32_at =
        |at: usize| u32::from_le_bytes(elf[at..at + 4].try_into().unwrap());
    assert_eq!(&elf[..4], b"\x7fELF");
    assert_eq!(elf[4], 1, "ELFCLASS32");
    assert_eq!(elf[5], 1, "little-endian");
    assert_eq!(u16_at(16), 2, "an executable, not position-independent");
    assert_eq!(u16_at(18), 243, "EM_RISCV");
    let flags = u32_at(36);
    assert_eq!(flags & 0x1, 0x1, "compressed instructions");
    assert_eq!(flags & 0x6, 0, "the soft-float ABI");
    let (phoff, phentsize, phnum) =
        (u32_at(28) as usize, u16_at(42) as usize, u16_at(44));
    let interp =
        (0..phnum as usize).any(|k| u32_at(phoff + k * phentsize) == 3);
    assert!(!interp, "no PT_INTERP: static");
}
