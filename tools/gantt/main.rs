// SPDX-License-Identifier: Apache-2.0
//! The project's timeline as a TikZ picture: a band per part of the
//! work, a column per day the project was worked on, and an arrow
//! from each band to what it needed before it could start.
//!
//! The axis is the days the project was worked on and not the
//! calendar. Between the specification and the implementation lie
//! four months in which nothing happened, and on a calendar axis
//! those four months would be most of the picture and the week that
//! holds almost every commit would be a smudge. The gap is said in
//! words instead, and marked on the axis where it falls.
//!
//! A band is drawn as a segment per day it was worked on, so a band
//! that stopped and started again reads as two blocks rather than one
//! long bar over a gap it was idle for. The shade of a block is how
//! many commits landed that day, so the picture carries the intensity
//! as well as the extent.
//!
//! Usage: `gantt docs/timeline.tsv > gantt.tex`
use std::collections::HashMap;
use std::fmt::Write as _;

/// One band: what it is called, what it needed first, and the days it
/// was worked on with the commits that landed on each.
struct Band {
    name: String,
    needs: Vec<String>,
    days: Vec<(String, usize)>,
}

/// The file as the extractor writes it: the axis, then the bands.
fn read(text: &str) -> (Vec<(String, usize)>, Vec<Band>) {
    let mut axis = Vec::new();
    let mut bands = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        match f.first() {
            Some(&"day") if f.len() >= 3 => {
                axis.push((f[1].to_string(), f[2].parse().unwrap_or(0)))
            }
            Some(&"task") if f.len() >= 4 => {
                let needs = if f[2].is_empty() {
                    Vec::new()
                } else {
                    f[2].split(';').map(|s| s.to_string()).collect()
                };
                let days = f[3]
                    .split(',')
                    .filter_map(|p| p.split_once(':'))
                    .map(|(d, n)| (d.to_string(), n.parse().unwrap_or(0)))
                    .collect();
                bands.push(Band {
                    name: f[1].to_string(),
                    needs,
                    days,
                })
            }
            _ => {}
        }
    }
    (axis, bands)
}

/// A label with the characters TeX reads as its own made safe.
fn tex(s: &str) -> String {
    s.replace('\\', "")
        .replace('&', "\\&")
        .replace('%', "\\%")
        .replace('_', "\\_")
        .replace('#', "\\#")
}

/// How dark a block is: more commits, more ink, so the picture says
/// where the work was as well as when.
fn shade(n: usize, most: usize) -> usize {
    if most == 0 {
        return 20;
    }
    // Between a fifth and full, so the lightest block is still seen.
    20 + (n * 65) / most
}

/// Days between two dates written `YYYY-MM-DD`, by the civil
/// calendar. Only ever asked about dates a few months apart.
fn days_between(a: &str, b: &str) -> i64 {
    fn to_days(s: &str) -> i64 {
        let (y, rest) = s.split_at(4);
        let m: i64 = rest[1..3].parse().unwrap_or(1);
        let d: i64 = rest[4..6].parse().unwrap_or(1);
        let mut y: i64 = y.parse().unwrap_or(0);
        let mut m = m;
        // Howard Hinnant's days-from-civil.
        y -= i64::from(m <= 2);
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        if m <= 2 {
            m += 12
        }
        let doy = (153 * (m - 3) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }
    to_days(b) - to_days(a)
}

/// Where the project stopped and started again: the column after the
/// longest stretch with nothing committed, and how many days that
/// stretch was. Found rather than stated, so a project with a
/// different history gets its own answer.
fn longest_gap(axis: &[(String, usize)]) -> (usize, i64) {
    let mut gap = 0i64;
    let mut at = 0usize;
    for i in 1..axis.len() {
        let d = days_between(&axis[i - 1].0, &axis[i].0);
        if d > gap {
            gap = d;
            at = i;
        }
    }
    (at, gap)
}

/// The numbers the document quotes, as TeX macros, so its prose
/// cannot drift from the chart beside it.
fn counts(axis: &[(String, usize)], bands: &[Band]) -> String {
    let total: usize = axis.iter().map(|(_, n)| n).sum();
    let (at, gap) = longest_gap(axis);
    let before: usize = axis[..at].iter().map(|(_, n)| n).sum();
    let after: usize = axis[at..].iter().map(|(_, n)| n).sum();
    let mut o = String::new();
    writeln!(o, "% Written by //tools/gantt. Do not edit.").unwrap();
    writeln!(o, "\\newcommand{{\\ganttcommits}}{{{total}}}").unwrap();
    writeln!(o, "\\newcommand{{\\ganttdays}}{{{}}}", axis.len()).unwrap();
    writeln!(o, "\\newcommand{{\\ganttbefore}}{{{before}}}").unwrap();
    writeln!(o, "\\newcommand{{\\ganttafter}}{{{after}}}").unwrap();
    writeln!(o, "\\newcommand{{\\ganttbeforedays}}{{{at}}}").unwrap();
    writeln!(o, "\\newcommand{{\\ganttafterdays}}{{{}}}", axis.len() - at)
        .unwrap();
    writeln!(o, "\\newcommand{{\\ganttgapdays}}{{{gap}}}").unwrap();
    let docs = bands
        .iter()
        .find(|b| b.name == "The documents")
        .map(|b| b.days.iter().map(|(_, n)| n).sum::<usize>())
        .unwrap_or(0);
    writeln!(o, "\\newcommand{{\\ganttdocs}}{{{docs}}}").unwrap();
    o
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let only_counts = args.iter().any(|a| a == "--counts");
    let path = match args.iter().skip(1).find(|a| !a.starts_with("--")) {
        Some(p) => p.clone(),
        None => {
            eprintln!("usage: gantt [--counts] TIMELINE.tsv");
            std::process::exit(1);
        }
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("gantt: {path}: {e}");
            std::process::exit(1);
        }
    };
    let (axis, bands) = read(&text);
    if axis.is_empty() || bands.is_empty() {
        eprintln!("gantt: {path} holds no timeline");
        std::process::exit(1);
    }
    if only_counts {
        print!("{}", counts(&axis, &bands));
        return;
    }
    let at: HashMap<&str, usize> = axis
        .iter()
        .enumerate()
        .map(|(i, (d, _))| (d.as_str(), i))
        .collect();
    let most = bands
        .iter()
        .flat_map(|b| b.days.iter().map(|(_, n)| *n))
        .max()
        .unwrap_or(1);
    // A column per day, a row per band, in centimetres.
    let col = 1.15f64;
    let row = 0.80f64;
    let left = 4.6f64;
    let height = row * bands.len() as f64;
    // The axis is working days, so a stretch with nothing committed
    // takes no room at all and two columns four months apart sit
    // side by side. A long one is marked rather than left silent: the
    // columns are pushed apart and the break is drawn and counted, so
    // a reader sees that the axis was cut and by how much.
    let (brk, gap_days) = longest_gap(&axis);
    let marked = gap_days > 7 && brk > 0;
    let gap_w = if marked { 0.85 } else { 0.0 };
    let xs: Vec<f64> = (0..axis.len())
        .map(|i| {
            left + col * i as f64 + if marked && i >= brk { gap_w } else { 0.0 }
        })
        .collect();

    let mut o = String::new();
    writeln!(o, "% Written by //tools/gantt from docs/timeline.tsv.").unwrap();
    writeln!(o, "% Do not edit: it is a build output.").unwrap();
    writeln!(o, "\\begin{{tikzpicture}}[x=1cm, y=1cm]").unwrap();
    writeln!(
        o,
        "\\tikzset{{\n  \
         band/.style={{font=\\footnotesize, anchor=east}},\n  \
         daylab/.style={{font=\\scriptsize, rotate=90, anchor=west}},\n  \
         blk/.style={{draw=black!55, line width=0.2pt}},\n  \
         dep/.style={{-{{Stealth[length=1.4mm]}}, draw=black!60, \
         line width=0.4pt}},\n}}"
    )
    .unwrap();

    // The columns, faintly, so a block can be read back to its day.
    for (i, (d, n)) in axis.iter().enumerate() {
        let x = xs[i] + col / 2.0;
        writeln!(
            o,
            "\\draw[black!12, line width=0.2pt] ({x:.2},0) -- ({x:.2},{:.2});",
            -height
        )
        .unwrap();
        writeln!(
            o,
            "\\node[daylab] at ({x:.2},0.12) {{{} \\textcolor{{black!55}}{{({n})}}}};",
            tex(&d[5..])
        )
        .unwrap();
    }

    // The cut in the axis, drawn where it falls: a white band to part
    // the two lives, two slanted strokes as a break is conventionally
    // drawn, and the days it stands for, so the compression is stated
    // rather than left for a reader to notice from the dates.
    if marked {
        let x = xs[brk] - gap_w / 2.0;
        writeln!(
            o,
            "\\fill[white] ({:.2},0.05) rectangle ({:.2},{:.2});",
            x - gap_w / 2.0 + 0.06,
            x + gap_w / 2.0 - 0.06,
            -height - 0.05
        )
        .unwrap();
        for d in [-0.11f64, 0.11] {
            writeln!(
                o,
                "\\draw[black!45, line width=0.5pt] ({:.2},{:.2}) -- \
                 ({:.2},0.05);",
                x + d - 0.13,
                -height - 0.05,
                x + d + 0.13
            )
            .unwrap();
        }
        writeln!(
            o,
            "\\node[font=\\scriptsize, rotate=90, anchor=center, \
             fill=white, inner sep=1pt] at ({:.2},{:.2}) \
             {{{gap_days} days}};",
            x,
            -height / 2.0
        )
        .unwrap();
    }

    // Where each band sits, so an arrow can find it.
    let ypos: HashMap<&str, f64> = bands
        .iter()
        .enumerate()
        .map(|(i, b)| (b.name.as_str(), -row * (i as f64 + 0.5)))
        .collect();

    for (i, b) in bands.iter().enumerate() {
        let y = -row * (i as f64 + 0.5);
        writeln!(
            o,
            "\\node[band] at ({:.2},{y:.2}) {{{}}};",
            left - 0.25,
            tex(&b.name)
        )
        .unwrap();
        for (d, n) in &b.days {
            let Some(&i) = at.get(d.as_str()) else {
                continue;
            };
            let x = xs[i] + 0.12;
            writeln!(
                o,
                "\\filldraw[blk, fill=black!{}] ({x:.2},{:.2}) rectangle \
                 ({:.2},{:.2});",
                shade(*n, most),
                y - row * 0.32,
                x + col - 0.24,
                y + row * 0.32
            )
            .unwrap();
        }
    }

    // An arrow from what a band needed to the band itself, drawn from
    // the end of the earlier work to the start of the later.
    for b in &bands {
        let Some(&to_y) = ypos.get(b.name.as_str()) else {
            continue;
        };
        let Some(first) = b.days.first() else {
            continue;
        };
        let Some(&to_i) = at.get(first.0.as_str()) else {
            continue;
        };
        for need in &b.needs {
            let Some(&from_y) = ypos.get(need.as_str()) else {
                continue;
            };
            let from = bands.iter().find(|o| &o.name == need);
            let Some(from) = from else { continue };
            let Some(last) = from
                .days
                .iter()
                .filter(|(d, _)| at.get(d.as_str()) <= Some(&to_i))
                .next_back()
            else {
                continue;
            };
            let Some(&from_i) = at.get(last.0.as_str()) else {
                continue;
            };
            let fx = xs[from_i] + col - 0.12;
            let tx = xs[to_i] + 0.12;
            writeln!(
                o,
                "\\draw[dep] ({fx:.2},{from_y:.2}) .. controls \
                 ({:.2},{from_y:.2}) and ({:.2},{to_y:.2}) .. \
                 ({tx:.2},{to_y:.2});",
                fx + 0.35,
                tx - 0.35
            )
            .unwrap();
        }
    }

    writeln!(o, "\\end{{tikzpicture}}").unwrap();
    print!("{o}");
}
