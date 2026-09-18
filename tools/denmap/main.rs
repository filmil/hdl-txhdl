// SPDX-License-Identifier: Apache-2.0
//! The placed core as a picture, from the grid OpenROAD wrote.
//!
//! A routed design is forty megabytes of DEF and no document can hold
//! it. What a reader wants from a picture of a layout is where the
//! cells went, and that fits in a grid: the flow bins the die into
//! sixty four squares a side and writes, for each, how much of it is
//! filled by cells of any kind and how much by flip flops. This draws
//! those two grids side by side.
//!
//! The flip flops are drawn on their own because they are what the
//! design's state is, and where they sit is the one thing in a layout
//! that a reader can tie back to the source: the register file and
//! the pipeline registers are a shape, and the instruction memory,
//! which is a mask ROM and therefore combinational, is a hole in it.
//!
//! Usage:
//!
//! ```text
//! denmap docs/asic_map.tsv > asicmap.tex
//! denmap --counts docs/asic_map.tsv docs/asic.tsv > asic_counts.tex
//! ```
use std::collections::HashMap;
use std::fmt::Write as _;

/// The grid as the flow wrote it: the die in microns, the bins a side,
/// how many instances were placed and how many of them are flip flops,
/// and the two grids themselves, indexed by row and then column.
struct Map {
    die_w: f64,
    die_h: f64,
    bins: usize,
    instances: usize,
    flops: usize,
    all: Vec<Vec<f64>>,
    flop: Vec<Vec<f64>>,
}

fn read_map(text: &str) -> Map {
    let mut m = Map {
        die_w: 0.0,
        die_h: 0.0,
        bins: 0,
        instances: 0,
        flops: 0,
        all: Vec::new(),
        flop: Vec::new(),
    };
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.first() {
            Some(&"# die") if f.len() >= 3 => {
                m.die_w = f[1].parse().unwrap_or(0.0);
                m.die_h = f[2].parse().unwrap_or(0.0);
            }
            Some(&"# bins") if f.len() >= 2 => {
                m.bins = f[1].parse().unwrap_or(0);
            }
            Some(&"# instances") if f.len() >= 3 => {
                m.instances = f[1].parse().unwrap_or(0);
                m.flops = f[2].parse().unwrap_or(0);
            }
            Some(&"all") | Some(&"ff") if f.len() >= 3 => {
                let row: Vec<f64> =
                    f[2].split(',').map(|v| v.parse().unwrap_or(0.0)).collect();
                if f[0] == "all" {
                    m.all.push(row);
                } else {
                    m.flop.push(row);
                }
            }
            _ => {}
        }
    }
    if m.bins == 0 || m.all.len() != m.bins || m.flop.len() != m.bins {
        panic!(
            "the map says {} bins a side and has {} and {} rows",
            m.bins,
            m.all.len(),
            m.flop.len()
        );
    }
    m
}

/// The metrics file: one name and one value per line.
fn read_metrics(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once('\t') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

/// The side of one panel and the gap between the two, in centimetres.
const SIDE: f64 = 7.4;
const GAP: f64 = 1.1;

/// One grid as shaded squares. A square that holds nothing is not
/// drawn at all, which is most of the second panel.
fn panel(
    out: &mut String,
    m: &Map,
    grid: &[Vec<f64>],
    x0: f64,
    colour: &str,
    scale: f64,
) {
    let step = SIDE / m.bins as f64;
    for (j, row) in grid.iter().enumerate() {
        for (i, v) in row.iter().enumerate() {
            let p = (v * scale * 100.0).round().clamp(0.0, 100.0) as i32;
            if p == 0 {
                continue;
            }
            let x = x0 + i as f64 * step;
            let y = j as f64 * step;
            let _ = writeln!(
                out,
                "  \\fill[{colour}!{p}!white] ({x:.3},{y:.3}) \
                 rectangle ({:.3},{:.3});",
                x + step,
                y + step
            );
        }
    }
    let _ = writeln!(
        out,
        "  \\draw[framegray] ({x0:.3},0) rectangle ({:.3},{SIDE:.3});",
        x0 + SIDE
    );
}

/// The key: ten steps from empty to full, with what the full end means.
fn key(out: &mut String, x0: f64, colour: &str, says: &str) {
    let w = 2.4;
    let h = 0.22;
    let y = -0.75;
    for k in 0..10 {
        let p = (k + 1) * 10;
        let x = x0 + k as f64 * w / 10.0;
        let _ = writeln!(
            out,
            "  \\fill[{colour}!{p}!white] ({x:.3},{y:.3}) \
             rectangle ({:.3},{:.3});",
            x + w / 10.0,
            y + h
        );
    }
    let _ = writeln!(
        out,
        "  \\draw[framegray] ({x0:.3},{y:.3}) rectangle ({:.3},{:.3});",
        x0 + w,
        y + h
    );
    let _ = writeln!(
        out,
        "  \\node[lbl, anchor=west] at ({:.3},{:.3}) {{{says}}};",
        x0 + w + 0.15,
        y + h / 2.0
    );
}

/// The fullest square of a grid, which is what its shading is against.
fn peak(grid: &[Vec<f64>]) -> f64 {
    let mut top: f64 = 0.0;
    for row in grid {
        for v in row {
            top = top.max(*v);
        }
    }
    top
}

fn picture(m: &Map) -> String {
    let mut out = String::new();
    // The first panel is shaded against a square that cells fill
    // entirely, which is what a placer is aiming at. The second is
    // shaded against its own fullest square, because flip flops are a
    // tenth of the cells and against a full square the whole panel
    // would be one pale wash.
    let ffpeak = peak(&m.flop);
    let ffscale = if ffpeak > 0.0 { 1.0 / ffpeak } else { 1.0 };
    let _ = writeln!(out, "% Generated by //tools/denmap. Do not edit.");
    let _ = writeln!(out, "\\begin{{tikzpicture}}[x=1cm, y=1cm]");
    panel(&mut out, m, &m.all, 0.0, "black", 1.0);
    panel(&mut out, m, &m.flop, SIDE + GAP, "kwblue", ffscale);
    key(&mut out, 0.0, "black", "empty to full");
    key(&mut out, SIDE + GAP, "kwblue", "none to the fullest square");
    let _ = writeln!(
        out,
        "  \\node[lbl, anchor=south] at ({:.3},{:.3}) \
         {{every cell}};",
        SIDE / 2.0,
        SIDE + 0.12
    );
    let _ = writeln!(
        out,
        "  \\node[lbl, anchor=south] at ({:.3},{:.3}) \
         {{the flip flops alone}};",
        SIDE + GAP + SIDE / 2.0,
        SIDE + 0.12
    );
    // The die is square, so one number says how wide the picture is.
    let _ = writeln!(
        out,
        "  \\draw[|<->|, framegray] (0,-0.30) -- ({SIDE:.3},-0.30);"
    );
    let _ = writeln!(
        out,
        "  \\node[font=\\scriptsize, fill=white, inner sep=1pt] \
         at ({:.3},-0.30) {{{:.0}\\,$\\mu$m}};",
        SIDE / 2.0,
        m.die_w
    );
    let _ = writeln!(out, "\\end{{tikzpicture}}");
    out
}

/// The numbers the prose quotes, as macros, so it cannot drift from
/// the run that produced them.
fn counts(m: &Map, metrics: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "% Generated by //tools/denmap. Do not edit.");
    let mut def = |name: &str, value: String| {
        let _ = writeln!(out, "\\newcommand{{\\{name}}}{{{value}}}");
    };
    def("asicdie", format!("{:.0}", m.die_w));
    def("asicbins", format!("{}", m.bins));
    def("asicinstances", group(m.instances));
    def("asicflops", group(m.flops));
    def("asicffpeak", format!("{:.0}", peak(&m.flop) * 100.0));
    let get = |k: &str| metrics.get(k).cloned().unwrap_or_default();
    def("asiccells", group(get("cells").parse().unwrap_or(0)));
    def("asicnets", group(get("nets").parse().unwrap_or(0)));
    let area: f64 = get("design_area").parse().unwrap_or(0.0);
    def("asicarea", group(area.round() as usize));
    def("asicutil", get("utilization"));
    def("asicperiod", get("clock_period"));
    def(
        "asicfreq",
        format!(
            "{:.0}",
            1000.0 / get("clock_period").parse::<f64>().unwrap_or(1.0)
        ),
    );
    def("asicslack", get("worst_slack_max"));
    def("asicholdslack", get("worst_slack_min"));
    def("asictns", get("tns"));
    def("asicdrc", get("drc_violations"));
    out
}

/// A count with a thin space every three digits, as the documents set
/// numbers.
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push_str("\\,");
        }
        out.push(c);
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--counts" {
        let m = read_map(&std::fs::read_to_string(&args[2]).unwrap());
        let metrics = read_metrics(&std::fs::read_to_string(&args[3]).unwrap());
        print!("{}", counts(&m, &metrics));
    } else if args.len() >= 2 {
        let m = read_map(&std::fs::read_to_string(&args[1]).unwrap());
        print!("{}", picture(&m));
    } else {
        eprintln!("usage: denmap [--counts] MAP.tsv [METRICS.tsv]");
        std::process::exit(2);
    }
}
