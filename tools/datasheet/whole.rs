// SPDX-License-Identifier: Apache-2.0
//! Is every row of every generated table on a page of the datasheets?
//!
//! `//tools/datasheet` writes each component's ports, state, children
//! and registers as tables. A table LaTeX cannot break runs off the
//! bottom of its page, and the rows past the edge are set on no page at
//! all, so `//docs:edge_test`, which reads the words that are on a page,
//! has nothing to see. Razboj's rasteriser lost every field after `zon`
//! that way (issue 1614).
//!
//! So this reads the tables the generator wrote, takes each row's first
//! word, and looks for the rows in the PDF's text in order, each within
//! a few lines of the one before, which leaves room for a page's
//! running head and a table's heading repeated on the next page. It
//! fails naming the component, the table and the first row it could
//! not find.
//!
//! Usage:
//!
//! ```text
//! whole --pdftotext TREE/usr/bin/pdftotext ds_tables.tex datasheets.pdf
//! ```

use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};

/// How many lines may lie between two rows of one table in the text:
/// a wrapped cell, a page's running head and number, and the table's
/// heading repeated on the next page.
const GAP: usize = 16;

/// One generated table: whose it is, which, and its rows' first words.
#[derive(Debug, PartialEq, Eq)]
pub struct Table {
    pub key: String,
    pub what: String,
    pub rows: Vec<String>,
}

/// A cell's text as the PDF shows it: the TeX commands taken out, their
/// arguments kept, and the escapes undone.
pub fn detex(cell: &str) -> String {
    let c: Vec<char> = cell.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' if i + 1 < c.len() && !c[i + 1].is_ascii_alphabetic() => {
                // An escaped character stands for itself, and a thin
                // space for nothing.
                if c[i + 1] != ',' {
                    out.push(c[i + 1]);
                }
                i += 2;
            }
            '\\' => {
                i += 1;
                while i < c.len() && c[i].is_ascii_alphabetic() {
                    i += 1;
                }
            }
            '{' | '}' | '$' => i += 1,
            '~' => {
                out.push(' ');
                i += 1;
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out.trim().to_string()
}

/// A row's first word: of its first cell that says anything.
fn first_word(row: &str) -> Option<String> {
    let row = row.trim_end().trim_end_matches("\\\\");
    row.split('&')
        .map(detex)
        .find(|c| !c.is_empty())
        .and_then(|c| c.split_whitespace().next().map(str::to_string))
}

/// The tables in the generator's output, with their rows' first words.
pub fn tables(tex: &str) -> Vec<Table> {
    let mut out = Vec::new();
    let mut cur: Option<(String, String)> = None;
    let mut rows = Vec::new();
    let mut in_rows = false;
    for line in tex.lines() {
        if let Some(rest) = line.strip_prefix("\\expandafter\\def\\csname ds@")
        {
            let name = rest.split('\\').next().unwrap_or("");
            let (what, key) = name.split_once('@').unwrap_or((name, ""));
            cur = Some((what.to_string(), key.to_string()));
            rows.clear();
            in_rows = false;
            continue;
        }
        let Some((what, key)) = &cur else { continue };
        if !matches!(what.as_str(), "ports" | "state" | "children" | "regs") {
            continue;
        }
        // The rows follow the heading's rule, in a `tabular`, or the
        // foot's definition, in a `longtable`, whose heading and foot
        // come first and hold no rows.
        let t = line.trim();
        if t == "\\midrule" || t == "\\endfoot" {
            in_rows = true;
        } else if t == "\\endhead" {
            in_rows = false;
        } else if t.starts_with("\\end{longtable}") || t == "\\bottomrule" {
            if in_rows && !rows.is_empty() {
                out.push(Table {
                    key: key.clone(),
                    what: what.clone(),
                    rows: std::mem::take(&mut rows),
                });
            }
            in_rows = false;
        } else if in_rows && t.ends_with("\\\\") {
            if let Some(w) = first_word(t) {
                rows.push(w);
            }
        }
    }
    out
}

/// The first word of each line of the PDF's text.
pub fn heads(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
        .collect()
}

/// How many of a table's rows are found in order, from the best start.
pub fn found(rows: &[String], heads: &[String]) -> usize {
    let mut best = 0;
    for (start, h) in heads.iter().enumerate() {
        if rows.first() != Some(h) {
            continue;
        }
        let (mut at, mut n) = (start, 1);
        for want in &rows[1..] {
            let end = (at + 1 + GAP).min(heads.len());
            match (at + 1..end).find(|&j| &heads[j] == want) {
                Some(j) => {
                    at = j;
                    n += 1;
                }
                None => break,
            }
        }
        best = best.max(n);
        if best == rows.len() {
            break;
        }
    }
    best
}

/// Runs `pdftotext` from the pinned tree, through the tree's own loader
/// and libraries, as //tools/pdfedge does.
fn pdftotext(tool: &Path, pdf: &Path) -> Result<String, String> {
    let tree = tool
        .to_str()
        .and_then(|p| p.strip_suffix("/usr/bin/pdftotext"))
        .ok_or_else(|| {
            format!("{}: not a pinned tree's tool", tool.display())
        })?;
    let out = Command::new(format!("{tree}/usr/lib64/ld-linux-x86-64.so.2"))
        .arg("--library-path")
        .arg(format!("{tree}/usr/lib/x86_64-linux-gnu"))
        .arg(tool)
        .arg("-raw")
        .arg(pdf)
        .arg("-")
        .output()
        .map_err(|e| format!("running {}: {e}", tool.display()))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [flag, tool, tex, pdf] = args.as_slice() else {
        eprintln!("usage: whole --pdftotext PDFTOTEXT ds_tables.tex PDF");
        return ExitCode::FAILURE;
    };
    if flag != "--pdftotext" {
        eprintln!("usage: whole --pdftotext PDFTOTEXT ds_tables.tex PDF");
        return ExitCode::FAILURE;
    }
    let tex = match fs::read_to_string(tex) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{tex}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let text = match pdftotext(Path::new(tool), Path::new(pdf)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{pdf}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let heads = heads(&text);
    let all = tables(&tex);
    let mut bad = 0;
    for t in &all {
        let n = found(&t.rows, &heads);
        if n < t.rows.len() {
            eprintln!(
                "{}'s {} table: {} of {} rows found in order; \
                 not found: `{}`",
                t.key,
                t.what,
                n,
                t.rows.len(),
                t.rows[n]
            );
            bad += 1;
        }
    }
    if bad > 0 {
        eprintln!("{bad} of {} tables are not whole on the page", all.len());
        return ExitCode::FAILURE;
    }
    println!("all {} generated tables are whole", all.len());
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEX: &str = "\\expandafter\\def\\csname ds@state@Buf\\endcsname{%\n\
        {\\footnotesize\n\\begin{longtable}{@{}llrr@{}}\n\\toprule\n\
        Field & What & Width & Words \\\\\n\\midrule\n\\endhead\n\
        \\bottomrule\n\\endfoot\n\
        \\code{head\\_full} & register & 1 &  \\\\\n\
        \\code{tail} & register & 8 &  \\\\\n\
        \\end{longtable}}\n}\n\
        \\expandafter\\def\\csname ds@figure@Buf\\endcsname{%\n\
        \\code{x} & y \\\\\n}\n";

    #[test]
    fn a_tables_rows_are_read_by_their_first_word() {
        assert_eq!(
            tables(TEX),
            vec![Table {
                key: "Buf".into(),
                what: "state".into(),
                rows: vec!["head_full".into(), "tail".into()],
            }]
        );
    }

    #[test]
    fn a_tabulars_rows_are_read_as_well() {
        let tex = "\\expandafter\\def\\csname ds@ports@Buf\\endcsname{%\n\
            \\begin{center}\\footnotesize\n\\begin{tabular}{@{}lll@{}}\n\
            \\toprule\nPort & What & Width \\\\\n\\midrule\n\
            \\code{tx\\_data} & in, wire & 8 \\\\\n\
            \\bottomrule\n\\end{tabular}\n\\end{center}\n}\n";
        assert_eq!(tables(tex)[0].rows, vec!["tx_data".to_string()]);
    }

    #[test]
    fn a_register_rows_first_cell_may_be_empty() {
        assert_eq!(
            first_word(" & \\code{en\\_irq} & rw & A bit. \\\\").as_deref(),
            Some("en_irq")
        );
        assert_eq!(detex("10\\,719"), "10719");
    }

    #[test]
    fn rows_across_a_page_break_are_found() {
        let text = "head_full register 1\nTXHDL DATASHEETS 190\n\
                    Field What Width Words\ntail register 8\n";
        let rows = vec!["head_full".to_string(), "tail".to_string()];
        assert_eq!(found(&rows, &heads(text)), 2);
    }

    #[test]
    fn rows_cut_off_the_page_are_missed() {
        let text = "head_full register 1\nSome prose.\n";
        let rows = vec!["head_full".to_string(), "tail".to_string()];
        assert_eq!(found(&rows, &heads(text)), 1);
    }
}
