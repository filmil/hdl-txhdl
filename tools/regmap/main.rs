// SPDX-License-Identifier: Apache-2.0
//! A register map, as declared once in the parts, written for the
//! software: a C header a driver includes, or a device tree node at
//! a base (issue 499).
//!
//! ```text
//! regmap sd c                                  # the header
//! regmap sd rs                                 # a Rust module
//! regmap all rs                                # every map, one crate
//! regmap sd dts 0x3600 hdlfactory,vreteno-sd   # the node
//! regmap list                                  # the maps there are
//! ```
//!
//! A map is added here when its peripheral declares one, so that a
//! header exists for every map and nobody types an offset twice.
use std::fmt::Write;
use std::process::exit;
use txhdl::regmap::RegMap;

/// Every map the parts declare, by the name its peripheral is known
/// by, one to a line and in alphabetical order, so that two changes
/// adding a map each touch different lines.
fn maps() -> Vec<(&'static str, &'static RegMap)> {
    vec![
        ("eth", &txhdl_parts::eth::regs::MAP),
        ("ethslots", &txhdl_parts::ethslots::regs::MAP),
        ("gpio", &txhdl_parts::gpio::regs::MAP),
        ("hdmi", &txhdl_parts::hdmi::regs::MAP),
        ("sd", &txhdl_parts::sd::regs::MAP),
        ("syscon", &txhdl_parts::syscon::regs::MAP),
        ("timer", &vreteno32::timer::clint::MAP),
        ("tracer", &txhdl_parts::tracer::regs::MAP),
        ("trng", &txhdl_parts::trng::regs::MAP),
        ("uart", &vreteno32::uart::serial::MAP),
        ("wdog", &txhdl_parts::wdog::regs::MAP),
    ]
}


/// The map as a Rust module for software that cannot depend on the
/// parts: the bare-metal firmware under `cpu/vreteno/rust`, which is
/// built for the core with no standard library (issue 709). A register
/// is its byte offset from the peripheral's base, a `usize` as an
/// address sum wants it; a field is its shift, its mask in place, its
/// width and its reset, as `u32`s a word is tested and built with.
fn rust_module(name: &str, map: &RegMap) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "/// The {name} register map.");
    let _ = writeln!(s, "pub mod {name} {{");
    let _ = writeln!(s, "    /// Bytes the map spans.");
    let _ = writeln!(s, "    pub const SPAN: usize = 0x{:x};", map.span());
    for r in map.regs {
        let reg = r.name.to_uppercase();
        let _ = writeln!(s, "    /// {}: {}.", r.access.as_str(), r.doc);
        let _ =
            writeln!(s, "    pub const {reg}: usize = 0x{:02x};", r.offset());
        for f in r.fields {
            let fname = format!("{reg}_{}", f.name.to_uppercase());
            let _ = writeln!(s, "    /// {}: {}.", f.access.as_str(), f.doc);
            let _ = writeln!(
                s,
                "    pub const {fname}_SHIFT: u32 = {};",
                f.field.shift
            );
            let _ = writeln!(
                s,
                "    pub const {fname}_MASK: u32 = 0x{:x};",
                f.field.mask()
            );
            let _ = writeln!(
                s,
                "    pub const {fname}_WIDTH: u32 = {};",
                f.field.width
            );
            let _ = writeln!(
                s,
                "    pub const {fname}_RESET: u32 = 0x{:x};",
                f.field.reset
            );
        }
    }
    let _ = writeln!(s, "}}");
    s
}

/// Every map as one Rust file, the module of each after the other.
fn rust_all() -> String {
    let mut s = String::new();
    let _ = writeln!(s, "//! Every register map the parts declare, written");
    let _ = writeln!(s, "//! by //tools/regmap from the declarations; edit");
    let _ = writeln!(s, "//! those and not this (issue 709).");
    let _ = writeln!(s, "#![no_std]");
    for (n, m) in maps() {
        let _ = writeln!(s);
        s.push_str(&rust_module(n, m));
    }
    s
}

fn usage() -> ! {
    eprintln!("usage: regmap <map> c");
    eprintln!("       regmap <map> rs");
    eprintln!("       regmap all rs");
    eprintln!("       regmap <map> dts <base> <compatible>");
    eprintln!("       regmap list");
    exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("list") {
        for (n, m) in maps() {
            println!("{n}: {} registers, {} bytes", m.regs.len(), m.span());
        }
        return;
    }
    if args.first().map(String::as_str) == Some("all")
        && args.get(1).map(String::as_str) == Some("rs")
    {
        print!("{}", rust_all());
        return;
    }
    let (Some(name), Some(what)) = (args.first(), args.get(1)) else {
        usage()
    };
    let Some((_, map)) = maps().into_iter().find(|(n, _)| n == name) else {
        eprintln!("regmap: no map named `{name}`; `regmap list` names them");
        exit(1)
    };
    match what.as_str() {
        "c" => print!("{}", map.c_header(name)),
        "rs" => print!("{}", rust_module(name, map)),
        "dts" => {
            let (Some(base), Some(compat)) = (args.get(2), args.get(3)) else {
                usage()
            };
            let base = base.trim_start_matches("0x");
            let Ok(base) = u32::from_str_radix(base, 16) else {
                eprintln!("regmap: the base is a hex number");
                exit(1)
            };
            print!("{}", map.dts_node(name, base, compat));
        }
        _ => usage(),
    }
}
