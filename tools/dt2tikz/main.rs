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
//! Usage: dt2tikz IN.dt [--order a,b,..] [--color] [--width CM]
//!        [--until TICKS] [--names FILE --signals 'path=>alias,..']
//!        > OUT.tex
//!
//! `--until` cuts the diagram at that tick: a long run, a processor's
//! program, shows its opening cycles and not the whole.
//!
//! `--names` is the sidecar the FST writer leaves beside its file: one
//! line per enum-valued signal, its path, a tab, and its variants by
//! index; `--signals` says which alias each path has here, so such a
//! signal is labelled by name rather than by number.
//!
//! Rows appear in the order the signals first appear in the text, or in
//! the order given, which is how the document asks for them.
use std::collections::BTreeMap;
use std::fmt::Write;

const XS_MIN: f64 = 0.30; // cm per tick, at least
const X_MAX: f64 = 16.0; // cm, the widest a diagram may be
const CHAR: f64 = 0.17; // cm per character of a bus label
const PAD: f64 = 0.45; // cm around a bus label, crossovers included
const H: f64 = 0.55; // signal height
const PITCH: f64 = 0.95; // row pitch

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().expect("usage: dt2tikz IN.dt [--order a,b]");
    let color = args.iter().any(|a| a == "--color");
    let x_max: f64 = match args.iter().position(|a| a == "--width") {
        Some(p) => args[p + 1].parse().expect("--width needs centimetres"),
        None => X_MAX,
    };
    let until: Option<u64> = args
        .iter()
        .position(|a| a == "--until")
        .map(|p| args[p + 1].parse().expect("--until needs ticks"));
    let wanted: Vec<String> = match args.iter().position(|a| a == "--order") {
        Some(p) => args
            .get(p + 1)
            .expect("--order needs a list")
            .split(',')
            .map(|s| s.to_string())
            .collect(),
        None => Vec::new(),
    };
    let text = std::fs::read_to_string(path).expect("read");
    // Alias -> variant names, from the sidecar and the signal list.
    let mut enum_names: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    if let Some(p) = args.iter().position(|a| a == "--names") {
        let sidecar = std::fs::read_to_string(&args[p + 1]).unwrap_or_default();
        let sigs = args
            .iter()
            .position(|a| a == "--signals")
            .map(|q| args[q + 1].clone())
            .unwrap_or_default();
        let alias_of: std::collections::HashMap<String, String> = sigs
            .split(',')
            .filter_map(|s| s.split_once("=>"))
            .map(|(p, a)| {
                (p.trim_start_matches('/').replace('/', "."), a.to_string())
            })
            .collect();
        for line in sidecar.lines() {
            let Some((path, names)) = line.split_once('\t') else {
                continue;
            };
            if let Some(alias) = alias_of.get(path) {
                enum_names.insert(
                    alias.clone(),
                    names.split(',').map(|s| s.to_string()).collect(),
                );
            }
        }
    }
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
    if let Some(u) = until {
        for h in hist.values_mut() {
            h.retain(|(t0, _)| *t0 < u);
        }
        t = t.min(u);
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
    // Spread the ticks so the widest bus label fits its shortest
    // interval, within the width of the page; and draw one interval
    // past the last change, so the last value has room too.
    let mut widest = 0.0f64;
    let mut shortest = u64::MAX;
    for (name, h) in &hist {
        if h.iter().all(|(_, v)| v.len() == 1) {
            continue;
        }
        for (k, (t0, v)) in h.iter().enumerate() {
            let t1 = h.get(k + 1).map(|(t1, _)| *t1).unwrap_or(t);
            if t1 > *t0 && k + 1 < h.len() {
                shortest = shortest.min(t1 - t0);
            }
            widest = widest.max(label_for(name, v, &enum_names).len() as f64);
        }
    }
    let tail = if shortest == u64::MAX { 1 } else { shortest };
    let end = (t + tail).max(1) as f64;
    let mut xs = XS_MIN;
    if shortest != u64::MAX && shortest > 0 {
        xs = xs.max((CHAR * widest + PAD) / shortest as f64);
    }
    if end * xs > x_max {
        xs = x_max / end;
    }
    let mut o = String::new();
    writeln!(
        o,
        "\\begin{{tikzpicture}}[font=\\scriptsize\\ttfamily, line width=0.5pt]"
    )
    .unwrap();
    let palette = [
        "blue!70!black",
        "red!70!black",
        "green!50!black",
        "orange!80!black",
        "violet",
        "teal",
        "brown",
    ];
    for (i, name) in order.iter().enumerate() {
        let y0 = -(i as f64) * PITCH;
        let col = if color {
            palette[i % palette.len()]
        } else {
            "black"
        };
        let lw = if color { "line width=0.9pt, " } else { "" };
        let h = &hist[name];
        writeln!(
            o,
            "\\node[anchor=east, font=\\scriptsize\\bfseries, {col}] \
             at (-0.15, {:.2}) {{{}}};",
            y0 + H / 2.0,
            tex(name)
        )
        .unwrap();
        let onebit = h.iter().all(|(_, v)| v.len() == 1);
        let mut prev_y: Option<f64> = None;
        for (k, (t0, v)) in h.iter().enumerate() {
            let x0 = *t0 as f64 * xs;
            let x1 = h
                .get(k + 1)
                .map(|(t1, _)| *t1 as f64 * xs)
                .unwrap_or(end * xs);
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
                            "\\draw[{lw}{col}] ({x0:.2},{py:.2}) \
                             -- ({x0:.2},{y:.2});"
                        )
                        .unwrap();
                    }
                }
                let style = if dashed { "[dashed]" } else { "" };
                writeln!(
                    o,
                    "\\draw[{lw}{col}]{style} ({x0:.2},{y:.2}) \
                     -- ({x1:.2},{y:.2});"
                )
                .unwrap();
                prev_y = Some(y);
            } else {
                let d = 0.07;
                let (a, b) = (
                    if k == 0 { x0 } else { x0 + d },
                    x1 - if k + 1 < h.len() { d } else { 0.0 },
                );
                if color {
                    writeln!(
                        o,
                        "\\fill[{col}!12] ({a:.2},{y0:.2}) \
                         rectangle ({b:.2},{:.2});",
                        y0 + H
                    )
                    .unwrap();
                }
                writeln!(
                    o,
                    "\\draw[{lw}{col}] ({a:.2},{y0:.2}) -- ({b:.2},{y0:.2});"
                )
                .unwrap();
                writeln!(
                    o,
                    "\\draw[{lw}{col}] ({a:.2},{:.2}) -- ({b:.2},{:.2});",
                    y0 + H,
                    y0 + H
                )
                .unwrap();
                if k > 0 {
                    writeln!(
                        o,
                        "\\draw[{lw}{col}] ({:.2},{y0:.2}) -- ({a:.2},{:.2});",
                        x0 - d,
                        y0 + H
                    )
                    .unwrap();
                    writeln!(
                        o,
                        "\\draw[{lw}{col}] ({:.2},{:.2}) -- ({a:.2},{y0:.2});",
                        x0 - d,
                        y0 + H
                    )
                    .unwrap();
                }
                let label = label_for(name, v, &enum_names);
                writeln!(
                    o,
                    "\\node at ({:.2},{:.2}) {{{}}};",
                    (a + b) / 2.0,
                    y0 + H / 2.0,
                    tex(&label)
                )
                .unwrap();
            }
        }
    }
    // The time axis, in ticks.
    let yb = -(order.len() as f64) * PITCH + PITCH - H - 0.15;
    let mut tick = 0u64;
    while (tick as f64) <= end {
        let x = tick as f64 * xs;
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
        end * xs
    )
    .unwrap();
    writeln!(o, "\\end{{tikzpicture}}").unwrap();
    print!("{o}");
}

/// A bus value's label: its variant's name when it has one, else its
/// bits or hex.
fn label_for(
    name: &str,
    v: &str,
    enum_names: &std::collections::HashMap<String, Vec<String>>,
) -> String {
    match enum_names.get(name) {
        Some(ns) => usize::from_str_radix(v, 2)
            .ok()
            .and_then(|i| ns.get(i))
            .cloned()
            .unwrap_or_else(|| pretty(v)),
        None => pretty(v),
    }
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
