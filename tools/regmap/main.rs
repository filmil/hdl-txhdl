// SPDX-License-Identifier: Apache-2.0
//! A register map, as declared once in the parts, written for the
//! software: a C header a driver includes, or a device tree node at
//! a base (issue 499).
//!
//! ```text
//! regmap sd c                                  # the header
//! regmap sd dts 0x3600 hdlfactory,vreteno-sd   # the node
//! regmap list                                  # the maps there are
//! ```
//!
//! A map is added here when its peripheral declares one, so that a
//! header exists for every map and nobody types an offset twice.
use std::process::exit;
use txhdl::regmap::RegMap;

/// Every map the parts declare, by the name its peripheral is known
/// by.
fn maps() -> Vec<(&'static str, &'static RegMap)> {
    vec![("sd", &txhdl_parts::sd::regs::MAP)]
}

fn usage() -> ! {
    eprintln!("usage: regmap <map> c");
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
    let (Some(name), Some(what)) = (args.first(), args.get(1)) else {
        usage()
    };
    let Some((_, map)) = maps().into_iter().find(|(n, _)| n == name) else {
        eprintln!("regmap: no map named `{name}`; `regmap list` names them");
        exit(1)
    };
    match what.as_str() {
        "c" => print!("{}", map.c_header(name)),
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
