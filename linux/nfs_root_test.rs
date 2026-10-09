// SPDX-License-Identifier: Apache-2.0
//! What the NFS root's tree holds, read back from its tar (issue 1438).
//!
//! The tree is unpacked as root on srv into the export, so every entry
//! must be root's, and the device nodes and the shell's link must be
//! what they are in the initramfs, since this is the same system with
//! its root elsewhere.
use std::collections::BTreeMap;

/// One entry: its type, mode, owner, link target, device and size.
#[derive(Debug)]
struct Entry {
    kind: u8,
    mode: u32,
    uid: u32,
    link: String,
    dev: (u32, u32),
    size: usize,
}

fn octal(field: &[u8]) -> u32 {
    let s = std::str::from_utf8(field).unwrap();
    let s = s.trim_matches(|c: char| c == '\0' || c == ' ');
    if s.is_empty() {
        0
    } else {
        u32::from_str_radix(s, 8).unwrap()
    }
}

fn text(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8(field[..end].to_vec()).unwrap()
}

/// The archive's entries by name: a 512-byte header each, then the
/// data rounded up to 512, until a header of zeros.
fn entries(bytes: &[u8]) -> BTreeMap<String, Entry> {
    let mut out = BTreeMap::new();
    let mut at = 0;
    while at + 512 <= bytes.len() && bytes[at..at + 512].iter().any(|&b| b != 0)
    {
        let h = &bytes[at..at + 512];
        let size = octal(&h[124..136]) as usize;
        out.insert(
            text(&h[0..100]),
            Entry {
                kind: h[156],
                mode: octal(&h[100..108]),
                uid: octal(&h[108..116]),
                link: text(&h[157..257]),
                dev: (octal(&h[329..337]), octal(&h[337..345])),
                size,
            },
        );
        at += 512 + size.div_ceil(512) * 512;
    }
    out
}

#[test]
fn the_nfs_root_is_the_initramfs_system_owned_by_root() {
    let path = std::env::var("NFS_ROOT").expect("NFS_ROOT");
    let bytes = std::fs::read(&path).expect("the tar");
    let e = entries(&bytes);
    assert!(e.values().all(|x| x.uid == 0), "all root's: {e:?}");
    let console = &e["dev/console"];
    assert_eq!((console.kind, console.dev), (b'3', (5, 1)), "{console:?}");
    let null = &e["dev/null"];
    assert_eq!((null.kind, null.dev), (b'3', (1, 3)), "{null:?}");
    let init = &e["init"];
    assert_eq!((init.kind, init.mode), (b'0', 0o755), "{init:?}");
    assert!(init.size > 0);
    let busybox = &e["bin/busybox"];
    assert_eq!((busybox.kind, busybox.mode), (b'0', 0o755));
    assert!(busybox.size > 500_000, "a static BusyBox: {}", busybox.size);
    let sh = &e["bin/sh"];
    assert_eq!((sh.kind, sh.link.as_str()), (b'2', "busybox"));
    // A directory is named with its slash.
    assert_eq!(e["tmp/"].mode, 0o1777, "tmp is sticky");
    // musl's shared library, its loader's name, a dynamic program and
    // the ssh server, with the accounts a login reads (issue 1439).
    assert_eq!(e["lib/libc.so"].kind, b'0');
    let ld = &e["lib/ld-musl-riscv32-sf.so.1"];
    assert_eq!((ld.kind, ld.link.as_str()), (b'2', "libc.so"));
    assert_eq!(e["bin/dynhello"].mode, 0o755);
    let db = &e["usr/sbin/dropbear"];
    assert_eq!((db.kind, db.link.as_str()), (b'2', "../bin/dropbearmulti"));
    assert_eq!(e["usr/bin/dropbearmulti"].mode, 0o755);
    assert_eq!(e["etc/shadow"].mode, 0o600, "shadow is root's alone");
    assert_eq!(e["root/.ssh/"].mode, 0o700, "the keys' directory");
}
