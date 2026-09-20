// SPDX-License-Identifier: Apache-2.0
//! Does a waveform fit the column it is put in?
//!
//! Nothing else catches it. LaTeX reports no overfull box for a TikZ
//! picture that draws past its bounding box, so a diagram wider than
//! its column lands on the other column, or in the margin, and the
//! build is green. Three had, when issue 324 was filed, and the only
//! evidence was the rendered page.
//!
//! The generated diagrams carry their own width: the coordinates in
//! `NAME_timing.tex` are centimetres, so the drawing is as wide as its
//! rightmost coordinate less its leftmost. The signal names sit left of
//! zero as nodes anchored east, outside the coordinates, so a fixed
//! allowance is added for them. A column in these documents is about
//! 8.9 cm; the page-wide diagrams measure 16 cm and the column ones 7
//! to 8, so the threshold is not delicate.
//!
//! Usage:
//!
//! ```text
//! figwidth [--column CM] [--names CM] --timing NAME_timing.tex SOURCE.tex...
//! ```
//!
//! For every `\input{NAME_timing.tex}` in the sources, the tool finds
//! the environment it sits in and the document it belongs to. A
//! `figure*`, or a document set `onecolumn`, has the whole page and
//! passes. A plain `figure` in a two-column document has the column,
//! and fails if the drawing plus the names allowance is wider than it,
//! saying the file, the line, the diagram and the two numbers, and that
//! the fix is `figure*`, which is what the documents already use for a
//! page-wide diagram. A diagram no source places is not an error: the
//! check is about placements.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The column, and what the names take of it, in centimetres.
const COLUMN: f64 = 8.9;
const NAMES: f64 = 1.3;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut column = COLUMN;
    let mut names = NAMES;
    let mut timing: Option<PathBuf> = None;
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--column" => {
                column =
                    args[i + 1].parse().expect("--column takes centimetres");
                i += 2;
            }
            "--names" => {
                names = args[i + 1].parse().expect("--names takes centimetres");
                i += 2;
            }
            "--timing" => {
                timing = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            a => {
                sources.push(PathBuf::from(a));
                i += 1;
            }
        }
    }
    let Some(timing) = timing else {
        eprintln!("usage: figwidth [--column CM] [--names CM] --timing NAME_timing.tex SOURCE.tex...");
        std::process::exit(2);
    };
    let file = timing
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = std::fs::read_to_string(&timing).unwrap_or_else(|e| {
        eprintln!("{}: {e}", timing.display());
        std::process::exit(2);
    });
    let width = drawing_width(&text);

    let docs = Documents::read(&sources);
    let mut failed = false;
    for placement in docs.placements(&file) {
        let need = width + names;
        if placement.whole_page {
            continue;
        }
        if need > column {
            failed = true;
            eprintln!(
                "{}:{}: {file} is {width:.2} cm wide, {need:.2} cm with its \
                 names, and the column of {} is {column:.1} cm. Put it in \
                 figure* rather than figure, as the page-wide diagrams are.",
                placement.file.display(),
                placement.line,
                placement.document.display()
            );
        }
    }
    if failed {
        std::process::exit(1);
    }
    println!(
        "{file}: {width:.2} cm, fits every column it is put in ({} placements)",
        docs.placements(&file).len()
    );
}

/// The drawing's width: its rightmost x coordinate less its leftmost,
/// over every `(x, y)` in the file.
fn drawing_width(text: &str) -> f64 {
    let mut least = f64::INFINITY;
    let mut most = f64::NEG_INFINITY;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let start = j;
            while j < bytes.len()
                && (bytes[j].is_ascii_digit()
                    || bytes[j] == b'.'
                    || bytes[j] == b'-')
            {
                j += 1;
            }
            let mut k = j;
            while k < bytes.len() && bytes[k] == b' ' {
                k += 1;
            }
            if k < bytes.len() && bytes[k] == b',' && j > start {
                if let Ok(x) = text[start..j].parse::<f64>() {
                    least = least.min(x);
                    most = most.max(x);
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    if least.is_finite() && most.is_finite() {
        most - least
    } else {
        0.0
    }
}

/// Where a diagram is put: the file and line of the `\input`, the
/// document that file is part of, and whether it has the whole page.
struct Placement {
    file: PathBuf,
    line: usize,
    document: PathBuf,
    whole_page: bool,
}

/// The sources, read once: each file's text, which files are documents
/// and whether each is set `onecolumn`, and which document each file is
/// input by.
struct Documents {
    text: HashMap<PathBuf, String>,
    onecolumn: HashMap<PathBuf, bool>,
    owner: HashMap<PathBuf, PathBuf>,
}

impl Documents {
    fn read(sources: &[PathBuf]) -> Self {
        let mut text = HashMap::new();
        for p in sources {
            if let Ok(t) = std::fs::read_to_string(p) {
                text.insert(p.clone(), t);
            }
        }
        // A document is a file with a `\documentclass`. It is one
        // column if that line says so, or if the class is `article`,
        // which is one column unless told otherwise.
        let mut onecolumn = HashMap::new();
        for (p, t) in &text {
            if let Some(line) =
                t.lines().find(|l| l.contains("\\documentclass"))
            {
                let one = line.contains("onecolumn")
                    || (line.contains("{article}")
                        && !line.contains("twocolumn"));
                onecolumn.insert(p.clone(), one);
            }
        }
        // A file input by a document belongs to it, and so does a file
        // input by that file, and so on. The two-column documents claim
        // first, so a section two documents share is judged by the
        // narrower column, and the answer does not depend on the order
        // a map happens to yield its keys in.
        let mut docs: Vec<&PathBuf> = onecolumn.keys().collect();
        docs.sort_by_key(|d| (onecolumn[*d], (*d).clone()));
        let mut owner: HashMap<PathBuf, PathBuf> = HashMap::new();
        for doc in docs {
            let mut stack = vec![doc.clone()];
            while let Some(f) = stack.pop() {
                if owner.contains_key(&f) {
                    continue;
                }
                owner.insert(f.clone(), doc.clone());
                let Some(t) = text.get(&f) else { continue };
                for name in inputs(t) {
                    let dir = doc.parent().unwrap_or(Path::new(""));
                    let mut q = dir.join(&name);
                    if q.extension().is_none() {
                        q.set_extension("tex");
                    }
                    if text.contains_key(&q) {
                        stack.push(q);
                    }
                }
            }
        }
        Documents {
            text,
            onecolumn,
            owner,
        }
    }

    /// Every place `file` is input, with its environment and document.
    fn placements(&self, file: &str) -> Vec<Placement> {
        let needle = format!("\\input{{{file}}}");
        let mut out = Vec::new();
        let mut files: Vec<&PathBuf> = self.text.keys().collect();
        files.sort();
        for p in files {
            let t = &self.text[p];
            let mut env = String::new();
            for (n, line) in t.lines().enumerate() {
                if let Some(i) = line.find("\\begin{") {
                    let rest = &line[i + 7..];
                    if let Some(j) = rest.find('}') {
                        let name = &rest[..j];
                        if name.starts_with("figure")
                            || name.starts_with("table")
                        {
                            env = name.to_string();
                        }
                    }
                }
                if line.contains("\\end{figure") || line.contains("\\end{table")
                {
                    env.clear();
                }
                if line.contains(&needle) {
                    let document =
                        self.owner.get(p).cloned().unwrap_or(p.clone());
                    let one =
                        self.onecolumn.get(&document).copied().unwrap_or(false);
                    let starred = env.ends_with('*');
                    out.push(Placement {
                        file: p.clone(),
                        line: n + 1,
                        document,
                        whole_page: one || starred,
                    });
                }
            }
        }
        out
    }
}

/// The names in a file's `\input{...}` commands.
fn inputs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("\\input{") {
        let after = &rest[i + 7..];
        if let Some(j) = after.find('}') {
            out.push(after[..j].to_string());
            rest = &after[j..];
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_width_is_the_span_of_the_x_coordinates() {
        let text = "\\draw (0.00,-0.00) -- (0.20,0.55);\n\\node at (-0.15, 0.28) {clk};\n\\draw (7.85,0.10) -- ( 7.85 , 0.55);";
        // The node at -0.15 is a coordinate too, and counts; the
        // rightmost is 7.85.
        assert!((drawing_width(text) - 8.0).abs() < 1e-9);
        assert_eq!(drawing_width("nothing here"), 0.0);
    }

    /// Sources on disk, as the test needs them: a two-column document
    /// and a one-column one, each inputting a section that places the
    /// diagram, one in `figure` and one in `figure*`.
    fn sources(dir: &Path) -> Vec<PathBuf> {
        let write = |name: &str, text: &str| -> PathBuf {
            let p = dir.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
            p
        };
        vec![
            write(
                "two.tex",
                "\\documentclass[journal,9pt]{IEEEtran}\n\\input{parts/narrow}\n\\input{parts/wide}\n",
            ),
            write(
                "one.tex",
                "\\documentclass[journal,onecolumn,9pt]{IEEEtran}\n\\input{parts/narrow}\n",
            ),
            write(
                "parts/narrow.tex",
                "text\n\\begin{figure}[t]\n\\input{x_timing.tex}\n\\end{figure}\n",
            ),
            write(
                "parts/wide.tex",
                "\\begin{figure*}[t]\n\\input{x_timing.tex}\n\\end{figure*}\n",
            ),
        ]
    }

    #[test]
    fn a_placement_knows_its_document_and_its_environment() {
        let dir = std::env::temp_dir()
            .join(format!("figwidth-{}", std::process::id()));
        let docs = Documents::read(&sources(&dir));
        let places = docs.placements("x_timing.tex");
        // The narrow section is input by both documents and belongs
        // to the two-column one, which claims first; the wide one is
        // only in the two-column document.
        assert_eq!(places.len(), 2);
        let wide = places
            .iter()
            .find(|p| p.file.ends_with("wide.tex"))
            .unwrap();
        assert!(wide.whole_page, "figure* has the page");
        assert!(wide.document.ends_with("two.tex"));
        let narrow = places
            .iter()
            .find(|p| p.file.ends_with("narrow.tex"))
            .unwrap();
        assert_eq!(narrow.line, 3);
        assert!(narrow.document.ends_with("two.tex"));
        assert!(!narrow.whole_page, "a plain figure in two columns");
        std::fs::remove_dir_all(&dir).ok();
    }
}
