// SPDX-License-Identifier: Apache-2.0
//! The boards' memory maps, as LaTeX (issue 444).
//!
//! A board document lists the map its design decodes, and this writes
//! it from the design rather than from a copy: the router's ranges
//! and names are `BoardMap`'s, the peripheral page's are `SlotMap`'s,
//! and a peripheral's registers are the map its `regmap!` declares.
//! Each table is a definition, `\mm@<what>@<key>`, which the documents
//! read through `\mmget` in `docs/memmap.tex`; asking for one this
//! does not write stops the document, so a table cannot quietly go.
use txhdl::map::AddrMap;
use txhdl::regmap::RegMap;
use vreteno32::board::{BoardMap, SlotMap};

/// One LaTeX definition, `\mm@<what>@<key>`, holding `body`.
fn define(what: &str, key: &str, body: &str) {
    println!(
        "\\expandafter\\def\\csname mm@{what}@{key}\\endcsname{{%\n{body}}}"
    );
}

/// A table from a column spec, a header and rows.
fn table(cols: &str, head: &str, rows: &[String]) -> String {
    format!(
        "\\begin{{center}}\\footnotesize\n\\begin{{tabular}}{{@{{}}{cols}@{{}}}}\n\
         \\toprule\n{head} \\\\\n\\midrule\n{}\n\\bottomrule\n\
         \\end{{tabular}}\n\\end{{center}}\n",
        rows.join("\n")
    )
}

/// An address as the documents write one: `0x3000`, or in groups of
/// four digits past sixteen bits, `0x4000\_0000`.
fn addr(a: usize) -> String {
    let h = format!("{a:x}");
    if h.len() <= 4 {
        return format!("\\code{{0x{h}}}");
    }
    let h = format!("{a:08x}");
    format!("\\code{{0x{}\\_{}}}", &h[..4], &h[4..])
}

/// A size in the largest unit it is a whole number of.
fn size(n: usize) -> String {
    for (u, s) in [(1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")] {
        if n >= u && n.is_multiple_of(u) {
            return format!("{}~{s}", n / u);
        }
    }
    format!("{n}~bytes")
}

/// The ranges of a map, lowest first: the base, the last address, the
/// size and the name, with `names` in place of the map's own where
/// it gives one.
fn ranges<const N: usize, M: AddrMap<N>>(
    names: &[(usize, &str)],
) -> Vec<String> {
    let mut rs: Vec<(usize, usize, &str)> = (0..N)
        .map(|i| {
            let (base, mask) = M::RANGES[i];
            let n = (!mask & 0xffff_ffff) + 1;
            let name = names
                .iter()
                .find(|(k, _)| *k == i)
                .map_or(M::NAMES[i], |(_, s)| *s);
            (base, n, name)
        })
        .collect();
    rs.sort();
    rs.iter()
        .map(|(b, n, name)| {
            format!(
                "{} & {} & {} & {name} \\\\",
                addr(*b),
                addr(b + n - 1),
                size(*n)
            )
        })
        .collect()
}

/// The map of a board, keyed `key`, with `names` naming ranges of the
/// peripheral page differently from `SlotMap`, as a board that puts
/// something on the third slot does.
fn board(key: &str, slots: &[(usize, &str)]) {
    let cols = "lll>{\\raggedright\\arraybackslash}p{0.4\\linewidth}";
    let head = "Base & Last & Size & What is there";
    define(
        "board",
        key,
        &table(cols, head, &ranges::<8, BoardMap>(&[])),
    );
    define(
        "page",
        key,
        &table(cols, head, &ranges::<7, SlotMap>(slots)),
    );
}

/// A peripheral's registers, from the base the board gives it: the
/// rows its map writes for a datasheet, offsets from that base, and
/// its fields under each word.
fn regs(key: &str, base: usize, map: &RegMap) {
    define(
        "regs",
        key,
        &table(
            "ll>{\\raggedright\\arraybackslash}p{0.18\\linewidth}\
             >{\\raggedright\\arraybackslash}p{0.4\\linewidth}",
            &format!("From {} & Register & Access & What it is", addr(base)),
            &map.tex_rows(),
        ),
    );
}

/// The board's peripherals that declare their registers with
/// `regmap!`, each with the key a document asks for, since most maps
/// are called `regs`, and its base: a router port's or a slot of the
/// page's. One to a line, and one more as each declaration lands.
fn reg_maps() -> Vec<(&'static str, usize, &'static RegMap)> {
    let port = |i: usize| <BoardMap as AddrMap<8>>::RANGES[i].0;
    let slot = |i: usize| <SlotMap as AddrMap<7>>::RANGES[i].0;
    vec![
        ("timer", port(1), &vreteno32::timer::clint::MAP),
        ("uart", slot(0), &vreteno32::uart::serial::MAP),
        ("pwm", slot(1), &txhdl_parts::pwm::regs::MAP),
        ("hdmi", slot(2), &txhdl_parts::hdmi::regs::MAP),
        ("ethslots", slot(4), &txhdl_parts::ethslots::regs::MAP),
        ("trng", slot(5), &txhdl_parts::trng::regs::MAP),
        ("spi", slot(6), &txhdl_parts::spi::regs::MAP),
    ]
}

/// The system on a lattice's map, keyed `soc`: what the program and
/// the rasteriser keep where, and the corner a host bridge sends each
/// range to, which is the serial port's for its page and the memory's
/// for the rest. Each base is also an `at`, for the prose to name.
fn soc() {
    let corner = |base: usize| {
        let (x, y) = if base & soc::PAGE_MASK == soc::SERIAL_BASE {
            soc::SERIAL_NODE
        } else {
            soc::MEMORY_NODE
        };
        format!("{x},{y}")
    };
    let rows: Vec<String> = soc::REGIONS
        .iter()
        .map(|(b, n, what)| {
            format!(
                "{} & {} & {} & {} & {what} \\\\",
                addr(*b),
                addr(b + n - 1),
                size(*n),
                corner(*b)
            )
        })
        .collect();
    define(
        "board",
        "soc",
        &table(
            "llll>{\\raggedright\\arraybackslash}p{0.3\\linewidth}",
            "Base & Last & Size & Corner & What is there",
            &rows,
        ),
    );
    for (key, base) in [
        ("soc_data", soc::DATA_BASE),
        ("soc_dl", soc::DL_BASE),
        ("soc_serial", soc::SERIAL_BASE),
        ("soc_count", soc::DL_CTRL),
        ("soc_fb", soc::FB_BASE),
    ] {
        define("at", key, &addr(base));
    }
    // The first framebuffer word past 64 KiB whose page digit is the
    // serial port's: where a mask of `0xf000` sent the picture.
    let fb_end = soc::RAM_WORDS * 4;
    let alias = (soc::FB_BASE..fb_end)
        .step_by(0x1000)
        .find(|a| *a >= 0x1_0000 && a & 0xf000 == soc::SERIAL_BASE)
        .expect("the framebuffer reaches a page that aliases the port");
    define("at", "soc_alias", &addr(alias));
}

/// The words behind BAR1, keyed `bar`: 64-bit words, so the offsets go
/// in eights, which is why they are not a `regmap!`.
fn bar() {
    let rows: Vec<String> = pcie::bar::WORDS
        .iter()
        .map(|(off, name, access, what)| {
            format!("\\code{{0x{off:02x}}} & {name} & {access} & {what} \\\\")
        })
        .collect();
    define(
        "regs",
        "bar",
        &table(
            "ll>{\\raggedright\\arraybackslash}p{0.18\\linewidth}\
             >{\\raggedright\\arraybackslash}p{0.4\\linewidth}",
            "Offset & Word & Access & What it is",
            &rows,
        ),
    );
    // The identifier word's value, in groups of four digits.
    let h = format!("{:016x}", pcie::bar::IDENT);
    let groups: Vec<&str> = (0..4).map(|i| &h[i * 4..i * 4 + 4]).collect();
    define(
        "ident",
        "bar",
        &format!("\\code{{0x{}}}", groups.join("\\_")),
    );
}

fn main() {
    println!("% Generated by //tools/memmap. Do not edit.");
    soc();
    bar();
    board("vreteno", &[]);
    board(
        "flagship",
        &[(2, "the video peripheral, on the pixel clock")],
    );
    for (key, base, map) in reg_maps() {
        // Where the board puts it, for a document about the peripheral
        // rather than the board to say so without typing it (#444).
        define("at", key, &addr(base));
        regs(key, base, map);
    }
}
