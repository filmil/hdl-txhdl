// SPDX-License-Identifier: Apache-2.0
//! Does every word of a rendered document lie on its page?
//!
//! LaTeX sets a table wider than its column without a word: the build
//! is green, every reference resolves, and the page loses text off its
//! right edge. Both tables on the showcase's first page had, when issue
//! 457 was filed, and the only evidence was the rendered page.
//!
//! `pdftotext -bbox` writes every word of a PDF with its box in points,
//! and the page's own width beside it, so the check is a comparison:
//! a word whose box ends past the page's width, or starts before its
//! left edge, is text the reader cannot see. The tool runs `pdftotext`
//! from the pinned tree in `//third_party/poppler`, reads what it
//! writes, and fails naming the document, the page, the word and the
//! two numbers.
//!
//! Usage:
//!
//! ```text
//! pdfedge --pdftotext PATH [--slack PT] DOCUMENT.pdf...
//! ```
//!
//! `--slack` is how far past the edge a word may end before it counts,
//! in points; the default is nothing. A word cut by the edge is text
//! lost whatever its width, so the tool has no notion of a column: the
//! page is the one edge every document shares.
use std::path::Path;
use std::process::Command;

/// One word past an edge: where, what, and by how much.
#[derive(Debug, PartialEq)]
struct Overrun {
    page: usize,
    word: String,
    x_min: f64,
    x_max: f64,
    width: f64,
}

/// The value of `name="..."` in a tag, if it carries one.
fn attr(tag: &str, name: &str) -> Option<f64> {
    let key = format!("{}=\"", name);
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    tag[start..end].parse().ok()
}

/// The words past an edge in `pdftotext -bbox` output.
///
/// The output is XHTML, one `<page width= height=>` per page and one
/// `<word xMin= yMin= xMax= yMax=>text</word>` per word, each on a
/// line of its own, which is what this reads: nothing more of the
/// markup is needed, and the words are what the reader sees.
fn overruns(bbox: &str, slack: f64) -> Vec<Overrun> {
    let mut found = Vec::new();
    let mut page = 0;
    let mut width = 0.0;
    for line in bbox.lines() {
        let line = line.trim();
        if line.starts_with("<page ") {
            page += 1;
            width = attr(line, "width").unwrap_or(0.0);
        } else if line.starts_with("<word ") {
            let (x_min, x_max) = match (attr(line, "xMin"), attr(line, "xMax")) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };
            if x_max > width + slack || x_min < -slack {
                let text = line
                    .find('>')
                    .map(|i| &line[i + 1..])
                    .and_then(|s| s.find('<').map(|j| &s[..j]))
                    .unwrap_or("")
                    .to_string();
                found.push(Overrun {
                    page,
                    word: text,
                    x_min,
                    x_max,
                    width,
                });
            }
        }
    }
    found
}

/// Runs `pdftotext -bbox` from the pinned tree on one document.
///
/// The tree is the root the binary was unpacked into, found from the
/// binary's own path. The binary is run through the tree's own
/// dynamic loader with the tree's libraries, and not the machine's:
/// the tree carries Debian's C library, and the machine's loader with
/// that library under it does not get as far as `main`.
fn bbox(pdftotext: &Path, pdf: &Path) -> Result<String, String> {
    let tree = pdftotext
        .to_str()
        .and_then(|p| p.strip_suffix("/usr/bin/pdftotext"))
        .ok_or_else(|| format!("{}: not a pinned tree's pdftotext", pdftotext.display()))?;
    let out = Command::new(format!("{}/usr/lib64/ld-linux-x86-64.so.2", tree))
        .arg("--library-path")
        .arg(format!("{}/usr/lib/x86_64-linux-gnu", tree))
        .arg(pdftotext)
        .arg("-bbox")
        .arg(pdf)
        .arg("-")
        .output()
        .map_err(|e| format!("{}: running pdftotext: {}", pdf.display(), e))?;
    if !out.status.success() {
        return Err(format!(
            "{}: pdftotext failed: {}",
            pdf.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut pdftotext = None;
    let mut slack = 0.0;
    let mut pdfs = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--pdftotext" => pdftotext = args.next(),
            "--slack" => {
                slack = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| {
                        eprintln!("pdfedge: --slack wants a number of points");
                        std::process::exit(2)
                    })
            }
            _ => pdfs.push(a),
        }
    }
    let pdftotext = match pdftotext {
        Some(p) => p,
        None => {
            eprintln!("usage: pdfedge --pdftotext PATH [--slack PT] DOCUMENT.pdf...");
            std::process::exit(2)
        }
    };
    let mut bad = 0;
    for pdf in &pdfs {
        let text = match bbox(Path::new(&pdftotext), Path::new(pdf)) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("pdfedge: {}", e);
                std::process::exit(2)
            }
        };
        for o in overruns(&text, slack) {
            bad += 1;
            eprintln!(
                "{}: page {}: '{}' spans {:.1} to {:.1} pt on a page {:.0} pt wide",
                pdf, o.page, o.word, o.x_min, o.x_max, o.width
            );
        }
    }
    if bad > 0 {
        eprintln!(
            "pdfedge: {} word(s) past the page's edge: a table or a figure \
             wider than its column, which LaTeX does not report. Wrap the \
             column, shorten the text, or make the float page wide.",
            bad
        );
        std::process::exit(1)
    }
    println!("pdfedge: {} document(s), every word on its page", pdfs.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<html><body>
<doc>
<page width="612.000000" height="792.000000">
  <word xMin="54.000000" yMin="60.000000" xMax="100.000000" yMax="70.000000">fine</word>
  <word xMin="588.400000" yMin="179.000000" xMax="612.200000" yMax="186.000000">parent</word>
</page>
<page width="612.000000" height="792.000000">
  <word xMin="-2.500000" yMin="60.000000" xMax="40.000000" yMax="70.000000">left</word>
  <word xMin="600.900000" yMin="474.000000" xMax="612.443597" yMax="480.000000">fau</word>
</page>
</doc>
</body></html>
"#;

    #[test]
    fn a_word_past_either_edge_is_found_with_its_page() {
        let found = overruns(SAMPLE, 0.0);
        let words: Vec<(usize, &str)> =
            found.iter().map(|o| (o.page, o.word.as_str())).collect();
        assert_eq!(words, vec![(1, "parent"), (2, "left"), (2, "fau")]);
        assert_eq!(found[0].width, 612.0);
        assert!((found[0].x_max - 612.2).abs() < 1e-6);
    }

    #[test]
    fn slack_forgives_a_word_that_ends_within_it() {
        let found = overruns(SAMPLE, 0.5);
        let words: Vec<&str> = found.iter().map(|o| o.word.as_str()).collect();
        // `parent` ends 0.2 pt past the edge; `left` and `fau` are
        // 2.5 and 0.44 past theirs, and only 'left' exceeds 0.5.
        assert_eq!(words, vec!["left"]);
    }

    #[test]
    fn a_page_with_every_word_on_it_reports_nothing() {
        let clean = r#"<page width="612" height="792">
<word xMin="54" yMin="1" xMax="558" yMax="2">edge</word>
</page>"#;
        assert!(overruns(clean, 0.0).is_empty());
    }

    #[test]
    fn a_tag_without_the_attribute_is_skipped() {
        assert_eq!(attr("<word yMin=\"1\">", "xMax"), None);
        assert_eq!(attr("<page width=\"612.5\">", "width"), Some(612.5));
    }
}
