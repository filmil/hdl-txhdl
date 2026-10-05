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
//!
//! With `--frames --pdftocairo PATH` it checks listings instead: every
//! word that starts inside a listing's frame ends inside it (issue
//! 1061). A listing that runs past its frame is still on the page, so
//! the page's edge cannot see it, and neither LaTeX nor rustfmt says a
//! word. The frame is read from the page itself: the house style fills
//! a listing's background with `bgshade`, 97 per cent grey, and the
//! listings package draws that fill a line at a time as three
//! rectangles, a margin, the text and a margin. `pdftocairo -svg` writes
//! each as a path, and the widest of a line's three is where its text
//! has to stop.
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
            let (x_min, x_max) = match (attr(line, "xMin"), attr(line, "xMax"))
            {
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

/// A rectangle on a page, in points from its top left: left, top,
/// right, bottom.
type Rect = (f64, f64, f64, f64);

/// A listing's text area a line at a time, from one page as
/// `pdftocairo -svg` writes it: each axis-aligned rectangle filled with
/// a grey within half a per cent of 97, grouped by the line it fills,
/// and of each line's pieces the widest, which is the text between the
/// two margins.
fn frames(svg: &str) -> Vec<Rect> {
    let mut lines: Vec<Rect> = Vec::new();
    for path in svg.split("<path ").skip(1) {
        let tag = &path[..path.find("/>").unwrap_or(path.len())];
        let Some(fill) = quoted(tag, "fill") else {
            continue;
        };
        let grey: Vec<f64> = fill
            .trim_start_matches("rgb(")
            .trim_end_matches(')')
            .split(',')
            .filter_map(|c| c.trim().trim_end_matches('%').parse().ok())
            .collect();
        let is_shade = grey.len() == 3
            && grey.iter().all(|g| (g - grey[0]).abs() < 1e-6)
            && (96.5..=97.5).contains(&grey[0]);
        if !is_shade || tag.contains("transform=") {
            continue;
        }
        let Some(rect) = quoted(tag, "d").and_then(|d| rectangle(&d)) else {
            continue;
        };
        let same = |r: &Rect| {
            (r.1 - rect.1).abs() < 0.05 && (r.3 - rect.3).abs() < 0.05
        };
        match lines.iter_mut().find(|r| same(r)) {
            Some(r) if rect.2 - rect.0 > r.2 - r.0 => *r = rect,
            Some(_) => {}
            None => lines.push(rect),
        }
    }
    lines
}

/// The value of `name="..."` in a tag, as text.
fn quoted(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {}=\"", name);
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

/// The rectangle a path draws, when it draws one: `M x y` and three
/// `L`s around four corners on two values of each axis, then `Z`.
fn rectangle(d: &str) -> Option<Rect> {
    let nums: Vec<f64> = d
        .split(|c: char| c.is_ascii_alphabetic() || c.is_whitespace())
        .filter_map(|t| t.parse().ok())
        .collect();
    let ops: String = d.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if !ops.starts_with("MLLLZ") || nums.len() < 8 {
        return None;
    }
    let (xs, ys): (Vec<f64>, Vec<f64>) =
        nums[..8].chunks(2).map(|p| (p[0], p[1])).unzip();
    let distinct = |v: &[f64]| {
        let mut v = v.to_vec();
        v.sort_by(|a, b| a.total_cmp(b));
        v.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        v
    };
    let (dx, dy) = (distinct(&xs), distinct(&ys));
    if dx.len() != 2 || dy.len() != 2 {
        return None;
    }
    Some((dx[0], dy[0], dx[1], dy[1]))
}

/// One word past its frame: where, what, and by how much.
#[derive(Debug, PartialEq)]
struct Overflow {
    page: usize,
    word: String,
    x_max: f64,
    edge: f64,
}

/// The words of `pdftotext -bbox` output, page by page, as their boxes
/// and their text.
fn words(bbox: &str) -> Vec<Vec<(Rect, String)>> {
    let mut pages: Vec<Vec<(Rect, String)>> = Vec::new();
    for line in bbox.lines() {
        let line = line.trim();
        if line.starts_with("<page ") {
            pages.push(Vec::new());
        } else if line.starts_with("<word ") {
            let b = ["xMin", "yMin", "xMax", "yMax"].map(|k| attr(line, k));
            let [Some(x0), Some(y0), Some(x1), Some(y1)] = b else {
                continue;
            };
            let text = line
                .find('>')
                .map(|i| &line[i + 1..])
                .and_then(|s| s.find('<').map(|j| &s[..j]))
                .unwrap_or("")
                .to_string();
            if let Some(p) = pages.last_mut() {
                p.push(((x0, y0, x1, y1), text));
            }
        }
    }
    pages
}

/// The words of one page that start inside a listing line's text and
/// end past it by more than `slack`. A word is on a line when its
/// middle, top to bottom, is.
fn overflows(
    page: usize,
    words: &[(Rect, String)],
    lines: &[Rect],
    slack: f64,
) -> Vec<Overflow> {
    let mut found = Vec::new();
    for ((x0, y0, x1, y1), text) in words {
        let mid = (y0 + y1) / 2.0;
        let on = lines.iter().find(|l| {
            l.1 <= mid && mid <= l.3 && l.0 - 0.5 <= *x0 && *x0 < l.2
        });
        if let Some(l) = on {
            if *x1 > l.2 + slack {
                found.push(Overflow {
                    page,
                    word: text.clone(),
                    x_max: *x1,
                    edge: l.2,
                });
            }
        }
    }
    found
}

/// Runs a tool from the pinned tree and returns what it wrote.
///
/// The tree is the root the binary was unpacked into, found from the
/// binary's own path less `suffix`. The binary is run through the
/// tree's own dynamic loader with the tree's libraries, and not the
/// machine's: the tree carries Debian's C library, and the machine's
/// loader with that library under it does not get as far as `main`.
fn pinned(tool: &Path, suffix: &str, args: &[&str]) -> Result<String, String> {
    let tree = tool
        .to_str()
        .and_then(|p| p.strip_suffix(suffix))
        .ok_or_else(|| {
            format!("{}: not a pinned tree's tool", tool.display())
        })?;
    let out = Command::new(format!("{}/usr/lib64/ld-linux-x86-64.so.2", tree))
        .arg("--library-path")
        .arg(format!("{}/usr/lib/x86_64-linux-gnu", tree))
        .arg(tool)
        .args(args)
        .output()
        .map_err(|e| format!("running {}: {}", tool.display(), e))?;
    if !out.status.success() {
        return Err(format!(
            "{} failed: {}",
            tool.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Runs `pdftotext -bbox` from the pinned tree on one document.
fn bbox(pdftotext: &Path, pdf: &Path) -> Result<String, String> {
    let pdf_s = pdf.to_string_lossy();
    pinned(pdftotext, "/usr/bin/pdftotext", &["-bbox", &pdf_s, "-"])
        .map_err(|e| format!("{}: {}", pdf.display(), e))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut pdftotext = None;
    let mut pdftocairo = None;
    let mut check_frames = false;
    let mut slack = 0.0;
    let mut pdfs = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--pdftotext" => pdftotext = args.next(),
            "--pdftocairo" => pdftocairo = args.next(),
            "--frames" => check_frames = true,
            "--slack" => {
                slack = args.next().and_then(|s| s.parse().ok()).unwrap_or_else(
                    || {
                        eprintln!("pdfedge: --slack wants a number of points");
                        std::process::exit(2)
                    },
                )
            }
            _ => pdfs.push(a),
        }
    }
    let pdftotext = match pdftotext {
        Some(p) => p,
        None => {
            eprintln!(
                "usage: pdfedge --pdftotext PATH [--frames --pdftocairo PATH] \
                 [--slack PT] DOCUMENT.pdf..."
            );
            std::process::exit(2)
        }
    };
    if check_frames {
        let Some(cairo) = pdftocairo else {
            eprintln!("pdfedge: --frames wants --pdftocairo PATH");
            std::process::exit(2)
        };
        std::process::exit(check_listings(&pdftotext, &cairo, &pdfs, slack));
    }
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
    println!(
        "pdfedge: {} document(s), every word on its page",
        pdfs.len()
    );
}

/// The frame check over every document: 0 when every listing's words
/// end inside its frame, 1 when one does not, 2 when a tool fails.
fn check_listings(
    pdftotext: &str,
    pdftocairo: &str,
    pdfs: &[String],
    slack: f64,
) -> i32 {
    let mut bad = 0;
    for pdf in pdfs {
        let pages = match bbox(Path::new(pdftotext), Path::new(pdf)) {
            Ok(t) => words(&t),
            Err(e) => {
                eprintln!("pdfedge: {}", e);
                return 2;
            }
        };
        for (k, on_page) in pages.iter().enumerate() {
            let n = (k + 1).to_string();
            let args = ["-svg", "-f", &n, "-l", &n, pdf, "-"];
            let svg = match pinned(
                Path::new(pdftocairo),
                "/usr/bin/pdftocairo",
                &args,
            ) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("pdfedge: {}: {}", pdf, e);
                    return 2;
                }
            };
            for o in overflows(k + 1, on_page, &frames(&svg), slack) {
                bad += 1;
                eprintln!(
                    "{}: page {}: '{}' ends at {:.1} pt, past its listing's frame at {:.1}",
                    pdf, o.page, o.word, o.x_max, o.edge
                );
            }
        }
    }
    if bad > 0 {
        eprintln!(
            "pdfedge: {} word(s) past a listing's frame: a source line wider \
             than the listing's column. Wrap the line, or set the document's \
             \\listingsize smaller (issue 1061).",
            bad
        );
        return 1;
    }
    println!(
        "pdfedge: {} document(s), every listing inside its frame",
        pdfs.len()
    );
    0
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

    /// One listing line as `pdftocairo -svg` draws it: the margin, the
    /// text and the margin, then the frame's rule, which is a stroke and
    /// not a fill, and a black fill, which is not the shade.
    const PAGE: &str = r#"<svg>
<path fill-rule="nonzero" fill="rgb(96.998596%, 96.998596%, 96.998596%)" fill-opacity="1" d="M 48.96 65.86 L 51.95 65.86 L 51.95 58.89 L 48.96 58.89 Z M 48.96 65.86 "/>
<path fill="none" stroke-width="0.398" stroke="rgb(96.998596%, 96.998596%, 96.998596%)" d="M 0 0 L 0 6.97 " transform="matrix(1, 0, 0, -1, 48.765, 65.865)"/>
<path fill-rule="nonzero" fill="rgb(96.998596%, 96.998596%, 96.998596%)" fill-opacity="1" d="M 51.95 65.86 L 297.04 65.86 L 297.04 58.89 L 51.95 58.89 Z M 51.95 65.86 "/>
<path fill-rule="nonzero" fill="rgb(96.998596%, 96.998596%, 96.998596%)" fill-opacity="1" d="M 297.04 65.86 L 300.02 65.86 L 300.02 58.89 L 297.04 58.89 Z M 297.04 65.86 "/>
<path fill-rule="nonzero" fill="rgb(0%, 0%, 0%)" fill-opacity="1" d="M 10 10 L 20 10 L 20 20 L 10 20 Z M 10 10 "/>
</svg>"#;

    #[test]
    fn a_listing_line_is_its_widest_shaded_piece() {
        let f = frames(PAGE);
        assert_eq!(f.len(), 1, "one line: {f:?}");
        let (x0, y0, x1, y1) = f[0];
        assert!((x0 - 51.95).abs() < 1e-9 && (x1 - 297.04).abs() < 1e-9);
        assert!((y0 - 58.89).abs() < 1e-9 && (y1 - 65.86).abs() < 1e-9);
    }

    #[test]
    fn a_word_that_starts_in_a_frame_ends_in_it() {
        let f = frames(PAGE);
        let w =
            |x0: f64, x1: f64, t: &str| ((x0, 59.5, x1, 65.0), t.to_string());
        let on_page = vec![
            w(52.0, 120.0, "fits"),
            w(280.0, 296.9, "edge"),
            w(290.0, 302.7, "past"),
            // Text beside the listing, in the other column, is not its.
            w(320.0, 400.0, "beside"),
        ];
        let found = overflows(3, &on_page, &f, 0.0);
        let named: Vec<(usize, &str)> =
            found.iter().map(|o| (o.page, o.word.as_str())).collect();
        assert_eq!(named, vec![(3, "past")]);
        assert!((found[0].edge - 297.04).abs() < 1e-9);
        // Below the listing nothing is checked.
        let below = vec![((52.0, 70.0, 310.0, 76.0), "prose".to_string())];
        assert!(overflows(3, &below, &f, 0.0).is_empty());
    }

    #[test]
    fn words_are_read_page_by_page() {
        let pages = words(SAMPLE);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1][0].1, "left");
    }

    #[test]
    fn a_tag_without_the_attribute_is_skipped() {
        assert_eq!(attr("<word yMin=\"1\">", "xMax"), None);
        assert_eq!(attr("<page width=\"612.5\">", "width"), Some(612.5));
    }
}
