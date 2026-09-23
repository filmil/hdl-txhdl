// SPDX-License-Identifier: Apache-2.0
//! The board programs against the map of the board they run on.
//!
//! Every program under `cpu/vreteno/rust/` reaches its peripherals
//! through an address written by hand, and nothing else checks those.
//! The programs that matter are built `#[cfg(board_run)]`, so the
//! compiler never sees their constants in a normal build, and the
//! core does not trap on a bus error, so a wrong address is not
//! caught when it runs either: the router answers `DecErr`, the core
//! takes the answer's data, and the program reads zero and carries on
//! (issue 417).
//!
//! So a mistake here is silent at build time and at run time both.
//! `ddr3_board` read the timer at `0x2000` from the day it was
//! written until issue 415, and the only symptom was a board that
//! printed its header and then nothing at all.
//!
//! This test is what stands in the way of the next one.
use vreteno32::isa::{
    CLINT_BASE, ETH_BASE, ETH_BUF_BASE, MTIME_OFF, UART_BASE,
};

const BOOT: &str = include_str!("../rust/boot.rs");
const DDR3: &str = include_str!("../rust/ddr3.rs");
const FADE: &str = include_str!("../rust/fade.rs");
const ICO_HDMI: &str = include_str!("../rust/ico_hdmi.rs");

/// The board's address map, as `BoardRouter` in
/// `cpu/vreteno/src/board.rs` states it: each port's base and the bits
/// of an address that must equal it, in the router's own order.
///
/// This is a copy and copies go stale. It was written against a
/// six-port router and stayed at six when the debug module took a
/// seventh (issue 154), so an address in its range would have been
/// reported as decoding nowhere: a test that lies in the direction of
/// failure, which is the better direction but is still a lie. Issue
/// 444 is the argument for generating this rather than keeping it by
/// hand; until then, a port added to `BoardRouter` is added here in
/// the same change.
const MAP: [(u32, u32, &str); 7] = [
    (0x0000_1000, 0xffff_f000, "the data memory"),
    (0x0200_0000, 0xffff_0000, "the timer"),
    (0x0000_3000, 0xffff_f000, "the peripheral page"),
    (0x4000_0000, 0xc000_0000, "the DDR3"),
    (0x0c00_0000, 0xfc00_0000, "the interrupt controller"),
    (0x0000_0000, 0xffff_f000, "the boot memory"),
    (0x1000_0000, 0xffff_0000, "the debug module"),
];

/// The value of a `const NAME: *mut u32 = 0x...` in a program.
fn addr_of(src: &str, name: &str) -> u32 {
    let pat = format!("const {name}: *mut u32 = ");
    let at = src
        .find(&pat)
        .unwrap_or_else(|| panic!("no `{name}` in the program"));
    let tail = &src[at + pat.len()..];
    let end = tail.find(' ').expect("a constant that never ends");
    let text = tail[..end].trim_start_matches("0x").replace('_', "");
    u32::from_str_radix(&text, 16).expect("an address that is not a number")
}

/// Which port of the board decodes an address, if any.
fn decoded_by(addr: u32) -> Option<&'static str> {
    MAP.iter()
        .find(|(base, mask, _)| addr & mask == *base)
        .map(|(_, _, name)| *name)
}

/// Every address a board program names is one the board answers.
///
/// This is the general form of issue 415, and it is the check that
/// matters: an address that decodes to nothing is not a slow path or
/// a wrong value, it is a word the program invents.
#[test]
fn every_address_a_board_program_uses_is_one_the_board_decodes() {
    let programs: [(&str, &str, &[&str]); 4] = [
        ("boot.rs", BOOT, &["UART"]),
        ("ddr3.rs", DDR3, &["UART", "MTIME"]),
        ("fade.rs", FADE, &["UART", "MTIME", "PWM"]),
        ("ico_hdmi.rs", ICO_HDMI, &["UART", "VIDEO"]),
    ];
    for (file, src, names) in programs {
        for name in names {
            let addr = addr_of(src, name);
            assert!(
                decoded_by(addr).is_some(),
                "{file}'s `{name}` is {addr:#010x}, which no port of \
                 the board decodes; the router answers such a burst \
                 `DecErr` and the core reads zero without saying so"
            );
        }
    }
}

/// The serial port is where the design puts it, in every program that
/// speaks on it. A program that prints is a program whose first
/// symptom of a wrong address is silence.
#[test]
fn every_program_writes_to_the_serial_port_the_design_has() {
    let programs = [
        ("boot.rs", BOOT),
        ("ddr3.rs", DDR3),
        ("fade.rs", FADE),
        ("ico_hdmi.rs", ICO_HDMI),
    ];
    for (file, src) in programs {
        assert_eq!(addr_of(src, "UART"), UART_BASE, "{file}'s serial port");
    }
}

/// The timer's low half is at the offset the hardware puts it, in both
/// programs that wait on it.
///
/// `ddr3.rs` said `0x2000` until issue 415, which looks like
/// `0x0200_0000` with four digits lost, and `fade.rs` beside it had
/// the right value the whole time.
#[test]
fn the_programs_that_wait_read_the_timer_the_hardware_has() {
    let want = CLINT_BASE + MTIME_OFF;
    assert_eq!(addr_of(DDR3, "MTIME"), want, "ddr3.rs's timer");
    assert_eq!(addr_of(FADE, "MTIME"), want, "fade.rs's timer");
}

/// The four small peripherals share the page at the serial port's
/// base, a sixteenth of it each, so anything on a slot is within the
/// page and on a sixteenth's boundary.
#[test]
fn the_peripherals_on_a_slot_are_inside_the_page_they_share() {
    for (file, src, name) in
        [("fade.rs", FADE, "PWM"), ("ico_hdmi.rs", ICO_HDMI, "VIDEO")]
    {
        let addr = addr_of(src, name);
        assert_eq!(
            addr & 0xffff_f000,
            UART_BASE,
            "{file}'s `{name}` is outside the peripheral page"
        );
        assert_eq!(addr & 0xff, 0, "{file}'s `{name}` is not on a slot");
    }
}

/// The Ethernet port's registers are on a slot the page decodes, and
/// its buffers are in the memory the router answers for.
///
/// Issue 415 was an address that decoded to nothing, and the symptom
/// was a board that printed its header and then stopped for ever. A
/// peripheral base is the same kind of constant and fails the same
/// silent way, so it is checked the same way.
#[test]
fn the_ethernet_port_is_somewhere_the_board_answers() {
    // On the peripheral page, which the router gives 4 KiB.
    assert_eq!(
        decoded_by(ETH_BASE),
        Some("the peripheral page"),
        "the Ethernet registers"
    );

    // The page holds sixteen slots of 256 bytes and this is the
    // fifth, after the serial port, the modulator, the video slot and
    // the remote peripheral. Nothing before it moved to make room.
    assert_eq!(ETH_BASE & 0xff, 0, "a slot starts on a 256 byte boundary");
    assert_eq!(ETH_BASE, UART_BASE + 0x400, "the fifth slot");
    assert_ne!(ETH_BASE, UART_BASE, "and not the serial port's");

    // The buffers are in the DDR3 rather than inside the peripheral,
    // so the engines reach them over the bus like any other memory.
    assert_eq!(decoded_by(ETH_BUF_BASE), Some("the DDR3"), "the buffers");

    // 1 KiB aligned, which is what lets a 256 beat burst of words run
    // without crossing AXI4's 4 KiB boundary.
    assert_eq!(ETH_BUF_BASE & 0x3ff, 0, "and aligned for a full burst");

    // Clear of where a program is loaded, by a stated margin rather
    // than a vague one: sixteen megabytes above the memory's base,
    // where an image of a few tens of kilobytes goes.
    assert_eq!(
        (ETH_BUF_BASE - 0x4000_0000) >> 20,
        16,
        "megabytes above the address a program loads at"
    );
}

/// The map above has a port for every port the router has.
///
/// It cannot compare itself with `BoardRouter` directly, since that is
/// a type and its bases are const parameters. What it can do is read
/// the source and count, which is enough to catch the failure that
/// actually happened: a port added to the router and not added here.
#[test]
fn the_map_has_every_port_the_router_has() {
    const BOARD: &str = include_str!("../src/board.rs");

    let at = BOARD
        .find("pub type BoardRouter = Router")
        .expect("no `BoardRouter` in the board");
    let tail = &BOARD[at + "pub type BoardRouter = Router".len()..];
    let end = tail.find('<').expect("a router that never opens");
    let ports: usize = tail[..end].parse().expect("a router of no count");

    assert_eq!(
        MAP.len(),
        ports,
        "`BoardRouter` is a Router{ports} and the map here has \
         {} ports; a port added to the router is added here in the \
         same change",
        MAP.len()
    );
}
