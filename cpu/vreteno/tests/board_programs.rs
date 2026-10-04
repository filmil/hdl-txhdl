// SPDX-License-Identifier: Apache-2.0
//! The board programs against the map of the board they run on.
//!
//! Every program under `cpu/vreteno/rust/` reaches its peripherals
//! through an address written by hand, and nothing else checks those.
//! The programs that matter are built `#[cfg(board_run)]`, so the
//! compiler never sees their constants in a normal build. A wrong
//! address shows only when the program runs: the router answers
//! `DecErr` and the core takes a bus error trap (issue 417).
//!
//! So a mistake here is silent at build time, and found late.
//! `ddr3_board` read the timer at `0x2000` from the day it was
//! written until issue 415, and the only symptom was a board that
//! printed its header and then nothing at all.
//!
//! This test is what stands in the way of the next one.
use txhdl::map::AddrMap;
use vreteno32::board::{BoardMap, SlotMap};
use vreteno32::isa::{
    CLINT_BASE, ETH_BASE, ETH_BUF_BASE, MTIME_OFF, UART_BASE,
};

const BOOT: &str = include_str!("../rust/boot.rs");
const DDR3: &str = include_str!("../rust/ddr3.rs");
const FADE: &str = include_str!("../rust/fade.rs");
const ICO_HDMI: &str = include_str!("../rust/ico_hdmi.rs");
const HAL: &str = include_str!("../rust/hal/lib.rs");

/// A register's byte offset, as `vreteno_regs::<map>::<REG>` names it
/// in a program: the map the peripheral declares with `regmap!`, which
/// is where the generated crate takes it from (issue 709).
fn reg_offset(map: &str, reg: &str) -> u32 {
    let m = match map {
        "timer" => &vreteno32::timer::clint::MAP,
        "uart" => &vreteno32::uart::serial::MAP,
        _ => panic!("no map `{map}` here"),
    };
    m.regs
        .iter()
        .find(|r| r.name.to_uppercase() == reg)
        .unwrap_or_else(|| panic!("the map `{map}` has no `{reg}`"))
        .offset()
}

/// The value of a `const NAME: *mut u32 = ...` in a program: a hex
/// address, or a sum of them and registers of the maps, as
/// `(0x0200_0000 + vreteno_regs::timer::MTIME_LO) as *mut u32`.
fn addr_of(src: &str, name: &str) -> u32 {
    let pat = format!("const {name}: *mut u32 =");
    let at = src
        .find(&pat)
        .unwrap_or_else(|| panic!("no `{name}` in the program"));
    let tail = &src[at + pat.len()..];
    let end = tail.find(';').expect("a constant that never ends");
    let expr = tail[..end].replace(" as *mut u32", "");
    expr.split('+')
        .map(|t| t.trim().trim_matches(|c| c == '(' || c == ')').trim())
        .map(|t| match t.strip_prefix("vreteno_regs::") {
            Some(path) => {
                let (map, reg) = path.split_once("::").expect("map::REG");
                reg_offset(map, reg)
            }
            None => {
                let hex = t.trim_start_matches("0x").replace('_', "");
                u32::from_str_radix(&hex, 16)
                    .unwrap_or_else(|_| panic!("`{t}` is not an address"))
            }
        })
        .sum()
}

/// Which port of the board decodes an address, if any, by its name.
///
/// The map is `BoardMap`, the one `BoardRouter` decodes, read rather
/// than copied: a copy written against a six-port router stayed at six
/// when the debug module took a seventh (issue 154), and nothing
/// noticed (#444).
fn decoded_by(addr: u32) -> Option<&'static str> {
    let a = addr as usize;
    (0..BoardMap::RANGES.len())
        .find(|&i| a & BoardMap::RANGES[i].1 == BoardMap::RANGES[i].0)
        .map(|i| BoardMap::NAMES[i])
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
        Some(BoardMap::NAMES[2]),
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
    assert_eq!(
        decoded_by(ETH_BUF_BASE),
        Some(BoardMap::NAMES[3]),
        "the buffers"
    );

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

/// The HAL's `map` module: every `pub const NAME: usize = ...;` in it,
/// with its value. A value is a hex literal, or another name of the
/// module plus one, as `FLASH + 0x00A0_0000`.
fn hal_map() -> Vec<(String, usize)> {
    let start = HAL.find("pub mod map {").expect("the HAL's map");
    let body = &HAL[start..];
    let body = &body[..body.find("\n}\n").expect("the map's end")];
    let mut out: Vec<(String, usize)> = Vec::new();
    for line in body.lines() {
        let Some(rest) = line.trim().strip_prefix("pub const ") else {
            continue;
        };
        let (name, value) = rest.split_once(": usize = ").expect("a base");
        let value = value.trim_end_matches(';');
        let v = value
            .split('+')
            .map(str::trim)
            .map(|t| match t.strip_prefix("0x") {
                Some(h) => usize::from_str_radix(&h.replace('_', ""), 16)
                    .expect("a hex base"),
                None => {
                    out.iter()
                        .find(|(n, _)| n == t)
                        .unwrap_or_else(|| {
                            panic!("`{t}` is not in the map yet")
                        })
                        .1
                }
            })
            .sum();
        out.push((name.to_string(), v));
    }
    out
}

/// Every base the HAL types by hand is where the router puts it: a
/// memory or controller at its port's base in `BoardMap`, a small
/// peripheral at its slot's base in `SlotMap` (issue 709).
///
/// The registers inside each peripheral come from the maps through
/// `vreteno_regs`, but the bases are typed in the HAL, and a router
/// that moved one would leave every program that uses the HAL reading
/// zero without a word, as issue 417 says. So a base the HAL adds must
/// be added here too, or this fails.
#[test]
fn every_base_in_the_hal_is_where_the_router_puts_it() {
    let board = |i: usize| (BoardMap::RANGES[i].0, BoardMap::NAMES[i]);
    let slot = |i: usize| (SlotMap::RANGES[i].0, SlotMap::NAMES[i]);
    let want: [(&str, (usize, &str)); 14] = [
        ("ROM", board(5)),
        ("DMEM", board(0)),
        ("CLINT", board(1)),
        ("UART", slot(0)),
        ("PWM", slot(1)),
        ("VIDEO", slot(2)),
        ("REMOTE", slot(3)),
        ("TRNG", slot(5)),
        ("SPI", slot(6)),
        ("MDIO", slot(7)),
        ("SD", slot(8)),
        ("FLASH", board(7)),
        ("PLIC", board(4)),
        ("DDR3", board(3)),
    ];
    let hal = hal_map();
    for (name, value) in &hal {
        if name == "FLASH_PROGRAMS" {
            let flash = BoardMap::RANGES[7].0;
            assert_eq!(
                *value,
                flash + vreteno32::isa::FLASH_PROGRAMS as usize,
                "the HAL's FLASH_PROGRAMS is not the layout's offset"
            );
            continue;
        }
        let (_, (at, what)) = want
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| {
                panic!(
                    "the HAL's map has `{name}`, which this test does \
                     not hold to a port"
                )
            });
        assert_eq!(
            *value, *at,
            "the HAL's `{name}` is {value:#x}, and {what} is at {at:#x}"
        );
    }
    for (name, _) in &want {
        assert!(
            hal.iter().any(|(n, _)| n == name),
            "the HAL's map has no `{name}`"
        );
    }
}
