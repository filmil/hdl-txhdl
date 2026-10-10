// SPDX-License-Identifier: Apache-2.0
//! Writes the Utah teapot's control points as Rust (#1592), from
//! freeglut's `src/fg_teapot_data.h`, which MODULE.bazel fetches by
//! checksum:
//!
//!   teapot2rs <fg_teapot_data.h> <teapot_data.rs>
//!
//! The header holds Newell's teapot as ten input patches of sixteen
//! control point indices each, and the 129 distinct control points
//! they index, with z up. The first six patches, the rim, the body in
//! two, the lid in two and the bottom, are a quarter of a surface of
//! revolution and are turned to the other three quarters; the other
//! four, the handle in two and the spout in two, are halves mirrored
//! across the plane through the axis. That makes Newell's 32 patches.
//!
//! The output is a `no_std` crate of the two tables and nothing else,
//! with the header's copyright and permission notice carried over in
//! full, as its licence asks. The numbers are copied as the header
//! spells them, so the crate holds exactly the header's values.
use std::fs;
use std::process::exit;

/// The input patches and the control points the header has.
const PATCHES: usize = 10;
const POINTS: usize = 129;

fn fail(why: &str) -> ! {
    eprintln!("teapot2rs: {why}");
    exit(1)
}

/// The text between the braces that open after `name` and the brace
/// that closes them.
fn table<'a>(text: &'a str, name: &str) -> &'a str {
    let at = text
        .find(name)
        .unwrap_or_else(|| fail(&format!("no {name}")));
    let rest = &text[at..];
    let open = rest.find('{').unwrap_or_else(|| fail("no table"));
    let mut depth = 0;
    for (i, c) in rest[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[open + 1..open + i];
                }
            }
            _ => {}
        }
    }
    fail("an open table")
}

/// The rows of a table: the text inside each inner pair of braces.
fn rows(table: &str) -> Vec<&str> {
    table
        .split('{')
        .skip(1)
        .map(|r| r.split('}').next().unwrap_or(""))
        .collect()
}

/// The notice at the head of the header, its first comment up to the
/// paragraph on where Juhana Kouhia had the models from, as `//` lines.
fn notice(text: &str) -> Vec<String> {
    let end = text
        .find("Juhana Kouhia received")
        .unwrap_or_else(|| fail("no notice's end"));
    text[..end]
        .lines()
        .map(|l| {
            let l = l.trim_end();
            let l = l.strip_prefix("/*").unwrap_or(l);
            let l = l.trim_start().strip_prefix('*').unwrap_or(l.trim_start());
            format!(
                "//{}",
                if l.is_empty() {
                    String::new()
                } else {
                    format!(" {}", l.trim_start())
                }
            )
        })
        .map(|l| l.trim_end().to_string())
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        fail("usage: teapot2rs <fg_teapot_data.h> <teapot_data.rs>");
    }
    let text = fs::read_to_string(&args[1])
        .unwrap_or_else(|e| fail(&format!("{}: {e}", args[1])))
        .replace('\r', "");

    let patches: Vec<Vec<u32>> = rows(table(&text, "patchdata_teapot["))
        .iter()
        .map(|r| {
            r.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    s.parse().unwrap_or_else(|_| fail(&format!("index {s}")))
                })
                .collect()
        })
        .collect();
    if patches.len() != PATCHES || patches.iter().any(|p| p.len() != 16) {
        fail("not ten patches of sixteen");
    }
    if patches.iter().flatten().any(|&i| i as usize >= POINTS) {
        fail("an index past the points");
    }
    let points: Vec<Vec<String>> = rows(table(&text, "cpdata_teapot["))
        .iter()
        .map(|r| {
            r.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    let n = s.strip_suffix('f').unwrap_or(s);
                    n.parse::<f32>()
                        .unwrap_or_else(|_| fail(&format!("number {s}")));
                    n.to_string()
                })
                .collect()
        })
        .collect();
    if points.len() != POINTS || points.iter().any(|p| p.len() != 3) {
        fail("not 129 points of three");
    }

    let mut out = String::new();
    out.push_str(
        "// Written by //tools/teapot2rs from freeglut's src/fg_teapot_data.h,\n\
         // which MODULE.bazel fetches by checksum (#1592). Do not edit. The\n\
         // header's notice, which its licence asks to be kept:\n//\n",
    );
    for l in notice(&text) {
        out.push_str(&l);
        out.push('\n');
    }
    out.push_str(
        "\n//! The Utah teapot's control points, as freeglut's header holds them.\n\
         #![no_std]\n\n\
         /// The ten input patches, each sixteen indices into [`POINTS`], four\n\
         /// rows of four. The first six are turned about the axis to all four\n\
         /// quarters; the other four are mirrored across the plane y = 0.\n",
    );
    out.push_str(&format!("pub static PATCHES: [[u8; 16]; {PATCHES}] = [\n"));
    for p in &patches {
        let p: Vec<String> = p.iter().map(u32::to_string).collect();
        out.push_str(&format!("    [\n        {},\n    ],\n", p.join(", ")));
    }
    out.push_str("];\n\n/// The control points, with z up.\n");
    out.push_str(&format!("pub static POINTS: [[f32; 3]; {POINTS}] = [\n"));
    for p in &points {
        out.push_str(&format!("    [{}],\n", p.join(", ")));
    }
    out.push_str("];\n");
    fs::write(&args[2], out)
        .unwrap_or_else(|e| fail(&format!("{}: {e}", args[2])));
}
