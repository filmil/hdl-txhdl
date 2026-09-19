// SPDX-License-Identifier: Apache-2.0
//! Where the flagship's subsystems landed on the part, from the grid
//! `//flagship:place_report` read off the routed checkpoint.
//!
//! A utilisation report counts and does not say where. The question a
//! reader has about a design that is four subsystems sharing a part is
//! whether they are four regions or one mixture, and which of them the
//! pins pulled to an edge. That is a picture, and the data behind it is
//! a count of cells per subsystem per slice column and row.
//!
//! The map bins those coordinates and colours each bin by whichever
//! subsystem holds the most cells in it, with the shade saying how full
//! the bin is. Bins that no subsystem reaches are left as the die.
//!
//! Usage:
//!
//! ```text
//! fpgamap docs/flagship_place.tsv > flagshipmap.tex
//! fpgamap --totals docs/flagship_place.tsv > flagshiptotals.tex
//! ```
use std::collections::HashMap;
use std::fmt::Write as _;

/// A subsystem's totals, as the extraction wrote them.
struct Total {
    who: String,
    cells: u32,
    brams: u32,
    dsps: u32,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
}

/// The whole file: the part, the extent of the slice grid, the totals,
/// and a count of cells per subsystem per slice.
struct Place {
    part: String,
    gx: i32,
    gy: i32,
    totals: Vec<Total>,
    cells: HashMap<(String, i32, i32), u32>,
}

/// How many slices go into one bin of the map, on each axis. The part
/// is about 164 slices across and 203 up, so eight gives a grid of
/// about 21 by 26, which is a picture rather than a mosaic.
const BIN: i32 = 8;

/// The colour each subsystem is drawn in. Named rather than computed,
/// so that the same subsystem is the same colour in every build, and
/// chosen to be told apart in print as well as on a screen.
fn colour(who: &str) -> &'static str {
    match who {
        "core" => "flagcore",
        "memory" => "flagmem",
        "ethernet" => "flageth",
        "video" => "flagvid",
        "crossing" => "flagcdc",
        _ => "flagtop",
    }
}

/// What each subsystem is called in the figure, which is not what the
/// extraction calls it: the extraction's names are hierarchy prefixes.
fn label(who: &str) -> &'static str {
    match who {
        "core" => "Core and bus",
        "memory" => "DDR3 controller",
        "ethernet" => "Ethernet",
        "video" => "Video and I2C",
        "crossing" => "Crossing",
        _ => "Top level",
    }
}

fn read(text: &str) -> Place {
    let mut p = Place {
        part: String::new(),
        gx: 0,
        gy: 0,
        totals: Vec::new(),
        cells: HashMap::new(),
    };
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.first().copied() {
            Some("# part") if f.len() > 1 => p.part = f[1].to_string(),
            Some("# grid") if f.len() > 2 => {
                p.gx = f[1].parse().unwrap_or(0);
                p.gy = f[2].parse().unwrap_or(0);
            }
            Some("total") if f.len() > 8 => p.totals.push(Total {
                who: f[1].to_string(),
                cells: f[2].parse().unwrap_or(0),
                brams: f[3].parse().unwrap_or(0),
                dsps: f[4].parse().unwrap_or(0),
                x0: f[5].parse().unwrap_or(0),
                x1: f[6].parse().unwrap_or(0),
                y0: f[7].parse().unwrap_or(0),
                y1: f[8].parse().unwrap_or(0),
            }),
            Some("cell") if f.len() > 4 => {
                let who = f[1].to_string();
                let x: i32 = f[2].parse().unwrap_or(0);
                let y: i32 = f[3].parse().unwrap_or(0);
                let n: u32 = f[4].parse().unwrap_or(0);
                *p.cells.entry((who, x, y)).or_insert(0) += n;
            }
            _ => {}
        }
    }
    p
}

/// The map: one TikZ picture, a bin per square, the subsystem that
/// holds the most cells in a bin giving the colour and how full the
/// bin is giving the shade.
fn map(p: &Place) -> String {
    let bx = (p.gx + BIN - 1) / BIN;
    let by = (p.gy + BIN - 1) / BIN;
    // Per bin, the cells each subsystem has in it.
    let mut bins: HashMap<(i32, i32), HashMap<&str, u32>> = HashMap::new();
    let mut fullest = 1u32;
    for ((who, x, y), n) in &p.cells {
        let e = bins.entry((x / BIN, y / BIN)).or_default();
        *e.entry(who.as_str()).or_insert(0) += n;
    }
    for counts in bins.values() {
        let total: u32 = counts.values().sum();
        if total > fullest {
            fullest = total;
        }
    }

    // A quarter of an inch a bin would be a page wide, so the picture
    // is scaled to the column and the aspect kept.
    let unit = 2.6 / bx as f64;
    let mut s = String::new();
    let _ = writeln!(s, "\\begin{{tikzpicture}}[x={unit}in,y={unit}in]");
    let _ = writeln!(s, "  \\fill[flagdie] (0,0) rectangle ({bx},{by});");
    for ((cx, cy), counts) in &bins {
        let total: u32 = counts.values().sum();
        let (who, _) = counts
            .iter()
            .max_by_key(|(who, n)| (**n, *who))
            .map(|(w, n)| (*w, *n))
            .unwrap_or(("top", 0));
        // The shade runs from a quarter to full, so a bin with a
        // handful of cells is still visible against the die.
        let fill = 25.0 + 75.0 * (total as f64 / fullest as f64);
        let _ = writeln!(
            s,
            "  \\fill[{}!{:.0}!flagdie] ({},{}) rectangle ({},{});",
            colour(who),
            fill,
            cx,
            cy,
            cx + 1,
            cy + 1
        );
    }
    let _ = writeln!(s, "  \\draw[black!50] (0,0) rectangle ({bx},{by});");
    // The axes say what the numbers are, since a slice coordinate is
    // not a length.
    let _ = writeln!(
        s,
        "  \\node[below,font=\\footnotesize] at ({:.1},-0.15) \
         {{slice column, {} to {}}};",
        bx as f64 / 2.0,
        0,
        p.gx - 1
    );
    let _ = writeln!(
        s,
        "  \\node[rotate=90,above,font=\\footnotesize] at (-0.15,{:.1}) \
         {{slice row, {} to {}}};",
        by as f64 / 2.0,
        0,
        p.gy - 1
    );
    let _ = writeln!(s, "\\end{{tikzpicture}}");
    s
}

/// The legend and the totals, as a table, so the figure's colours have
/// names and the numbers behind them are on the same page.
fn totals(p: &Place) -> String {
    let mut rows: Vec<&Total> = p.totals.iter().collect();
    rows.sort_by_key(|t| std::cmp::Reverse(t.cells));
    let mut s = String::new();
    let _ = writeln!(s, "\\begin{{tabular}}{{@{{}}llrrrll@{{}}}}");
    let _ = writeln!(s, "\\toprule");
    let _ =
        writeln!(s, "& Subsystem & Cells & BRAM & DSP & Columns & Rows \\\\");
    let _ = writeln!(s, "\\midrule");
    for t in rows {
        let _ = writeln!(
            s,
            "\\textcolor{{{}}}{{$\\blacksquare$}} & {} & {} & {} & {} \
             & {}--{} & {}--{} \\\\",
            colour(&t.who),
            label(&t.who),
            group(t.cells),
            t.brams,
            t.dsps,
            t.x0,
            t.x1,
            t.y0,
            t.y1
        );
    }
    let _ = writeln!(s, "\\bottomrule");
    let _ = writeln!(s, "\\end{{tabular}}");
    s
}

/// A thousands separator that LaTeX sets as a thin space.
fn group(n: u32) -> String {
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
    let (want_totals, path) = match args.len() {
        3 if args[1] == "--totals" => (true, args[2].clone()),
        2 => (false, args[1].clone()),
        _ => {
            eprintln!("usage: fpgamap [--totals] PLACE.tsv");
            std::process::exit(2);
        }
    };
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    });
    let p = read(&text);
    if want_totals {
        print!("{}", totals(&p));
    } else {
        print!("{}", map(&p));
    }
}
