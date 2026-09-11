// SPDX-License-Identifier: Apache-2.0
//! Draw drawtiming text as a TikZ picture: one row per signal, time
//! left to right, a one-bit signal as a line and a wider one as a bus
//! with its value written in each stable interval.
//!
//! The text is the subset `sqlite2drawtiming` writes: a line of dots
//! advances time by one tick per dot, a line starting with `#` is a
//! comment, and any other line is `name=value;name=value...` ending in
//! a period, all at the current time.
//!
//! Usage: dt2tikz IN.dt [--order name,name,...] > OUT.tex
//!
//! Rows appear in the order the signals first appear in the text, or in
//! the order given, which is how the document asks for them.
use std::collections::BTreeMap;
use std::fmt::Write;

const XS: f64 = 0.30; // cm per tick
const H: f64 = 0.55; // signal height
const PITCH: f64 = 0.95; // row pitch

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().expect("usage: dt2tikz IN.dt [--order a,b]");
    let wanted: Vec<String> = match args.get(1).map(|s| s.as_str()) {
        Some("--order") => args
            .get(2)
            .expect("--order needs a list")
            .split(',')
            .map(|s| s.to_string())
            .collect(),
        _ => Vec::new(),
    };
    let text = std::fs::read_to_string(path).expect("read");
    let mut order: Vec<String> = Vec::new();
    let mut hist: BTreeMap<String, Vec<(u64, String)>> = BTreeMap::new();
    let mut t: u64 = 0;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.chars().all(|c| c == '.') {
            t += l.len() as u64;
            continue;
        }
        for stanza in l.trim_end_matches('.').split(';') {
            let Some((name, value)) = stanza.split_once('=') else {
                continue;
            };
            let (name, value) =
                (name.trim().to_string(), value.trim().to_string());
            if !hist.contains_key(&name) {
                order.push(name.clone());
            }
            let h = hist.entry(name).or_default();
            match h.last_mut() {
                Some(last) if last.0 == t => last.1 = value,
                _ => h.push((t, value)),
            }
        }
    }
    if !wanted.is_empty() {
        let mut rest: Vec<String> = order
            .iter()
            .filter(|n| !wanted.contains(n))
            .cloned()
            .collect();
        order = wanted
            .into_iter()
            .filter(|n| hist.contains_key(n))
            .collect();
        order.append(&mut rest);
    }
    let end = t.max(1) as f64;
    let mut o = String::new();
    writeln!(
        o,
        "\\begin{{tikzpicture}}[font=\\scriptsize\\ttfamily, line width=0.5pt]"
    )
    .unwrap();
    for (i, name) in order.iter().enumerate() {
        let y0 = -(i as f64) * PITCH;
        let h = &hist[name];
        writeln!(
            o,
            "\\node[anchor=east, font=\\scriptsize] at (-0.15, {:.2}) {{{}}};",
            y0 + H / 2.0,
            tex(name)
        )
        .unwrap();
        let onebit = h.iter().all(|(_, v)| v.len() == 1);
        let mut prev_y: Option<f64> = None;
        for (k, (t0, v)) in h.iter().enumerate() {
            let x0 = *t0 as f64 * XS;
            let x1 = h
                .get(k + 1)
                .map(|(t1, _)| *t1 as f64 * XS)
                .unwrap_or(end * XS);
            if onebit {
                let (y, dashed) = match v.as_str() {
                    "1" => (y0 + H, false),
                    "0" => (y0, false),
                    _ => (y0 + H / 2.0, true),
                };
                if let Some(py) = prev_y {
                    if (py - y).abs() > 1e-9 {
                        writeln!(
                            o,
                            "\\draw ({x0:.2},{py:.2}) -- ({x0:.2},{y:.2});"
                        )
                        .unwrap();
                    }
                }
                let style = if dashed { "[dashed]" } else { "" };
                writeln!(
                    o,
                    "\\draw{style} ({x0:.2},{y:.2}) -- ({x1:.2},{y:.2});"
                )
                .unwrap();
                prev_y = Some(y);
            } else {
                let d = 0.07;
                let (a, b) = (
                    if k == 0 { x0 } else { x0 + d },
                    x1 - if k + 1 < h.len() { d } else { 0.0 },
                );
                writeln!(o, "\\draw ({a:.2},{y0:.2}) -- ({b:.2},{y0:.2});")
                    .unwrap();
                writeln!(
                    o,
                    "\\draw ({a:.2},{:.2}) -- ({b:.2},{:.2});",
                    y0 + H,
                    y0 + H
                )
                .unwrap();
                if k > 0 {
                    writeln!(
                        o,
                        "\\draw ({:.2},{y0:.2}) -- ({a:.2},{:.2});",
                        x0 - d,
                        y0 + H
                    )
                    .unwrap();
                    writeln!(
                        o,
                        "\\draw ({:.2},{:.2}) -- ({a:.2},{y0:.2});",
                        x0 - d,
                        y0 + H
                    )
                    .unwrap();
                }
                writeln!(
                    o,
                    "\\node at ({:.2},{:.2}) {{{}}};",
                    (a + b) / 2.0,
                    y0 + H / 2.0,
                    tex(&pretty(v))
                )
                .unwrap();
            }
        }
    }
    // The time axis, in ticks.
    let yb = -(order.len() as f64) * PITCH + PITCH - H - 0.15;
    let mut tick = 0u64;
    while (tick as f64) <= end {
        let x = tick as f64 * XS;
        writeln!(
            o,
            "\\draw[gray!50] ({x:.2},{:.2}) -- ({x:.2},{:.2});",
            yb,
            yb - 0.1
        )
        .unwrap();
        writeln!(
            o,
            "\\node[anchor=north, font=\\tiny] at ({x:.2},{:.2}) {{{tick}}};",
            yb - 0.1
        )
        .unwrap();
        tick += 2;
    }
    writeln!(
        o,
        "\\draw[gray!50] (0,{yb:.2}) -- ({:.2},{yb:.2});",
        end * XS
    )
    .unwrap();
    writeln!(o, "\\end{{tikzpicture}}").unwrap();
    print!("{o}");
}

/// A bus value: hex when it is binary and long enough to be unreadable.
fn pretty(v: &str) -> String {
    if v.len() > 4 && v.chars().all(|c| c == '0' || c == '1') {
        let n = u128::from_str_radix(v, 2).unwrap_or(0);
        format!("{n:x}")
    } else {
        v.to_string()
    }
}

fn tex(s: &str) -> String {
    s.replace('_', "\\_").replace('#', "\\#")
}
