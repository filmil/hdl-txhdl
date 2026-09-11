// SPDX-License-Identifier: Apache-2.0
//! Print the public interface of a Rust file: the doc comments and
//! signatures of `pub` items, of the items of public traits, and of the
//! public items of `impl` blocks, with every body dropped and every
//! private field hidden. Line based, which is enough for a tree that
//! rustfmt has formatted: an item starts at its block's indent, its
//! signature ends at the first line ending in `{` or `;`, and a body
//! ends at the line holding only `}` at the same indent.
//!
//! Usage: api FILE > OUT
use std::io::Write;

fn main() {
    let path = std::env::args().nth(1).expect("usage: api FILE");
    let src = std::fs::read_to_string(&path).expect("read");
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::new();
    // Module docs.
    for l in &lines {
        if l.starts_with("//!") {
            out.push_str(l);
            out.push('\n')
        } else if !l.is_empty() {
            break;
        }
    }
    let mut i = 0;
    items(&lines, &mut i, 0, Kind::Module, &mut out);
    std::io::stdout().write_all(out.as_bytes()).unwrap();
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Module,
    Trait,
    Impl,
    TraitImpl,
    Struct,
    Enum,
}

/// Emit the items of one block at `indent`, from line `*i` to the line
/// holding only `}` at that indent (or the end of the file).
fn items(
    lines: &[&str],
    i: &mut usize,
    indent: usize,
    kind: Kind,
    out: &mut String,
) {
    let pad = " ".repeat(indent);
    let mut docs = String::new();
    let mut attrs = String::new();
    while *i < lines.len() {
        let line = lines[*i];
        if indent > 0 && line == format!("{}}}", " ".repeat(indent - 4)) {
            return;
        }
        let t = line.trim_start();
        if t.is_empty()
            || !line.starts_with(&pad)
            || line.len() - t.len() != indent
        {
            *i += 1;
            continue;
        }
        if t.starts_with("///") {
            docs.push_str(line);
            docs.push('\n');
            *i += 1;
            continue;
        }
        if t.starts_with("#[") {
            attrs.push_str(line);
            attrs.push('\n');
            *i += 1;
            continue;
        }
        if t.starts_with("//") {
            *i += 1;
            continue;
        }
        // The signature: this line and following ones until `{` or `;`.
        let start = *i;
        let mut sig = String::new();
        loop {
            let l = lines[*i];
            sig.push_str(l);
            *i += 1;
            let e = l.trim_end();
            if e.ends_with('{')
                || e.ends_with(';')
                || e.ends_with(',')
                || e.ends_with("{}")
                || (e.contains('{') && e.ends_with('}'))
                || *i >= lines.len()
            {
                break;
            }
            sig.push('\n');
        }
        let e = sig.trim_end();
        let one_line_body = e.contains('{') && e.ends_with('}');
        let has_block = e.ends_with('{');
        // Visibility: public items, trait items, macro_rules with export.
        let public = t.starts_with("pub ")
            || t.starts_with("impl")
            || kind == Kind::Trait
            || kind == Kind::TraitImpl
            || kind == Kind::Enum
            || (t.starts_with("macro_rules!")
                && attrs.contains("macro_export"));
        let child = if t.starts_with("pub trait") || t.starts_with("trait ") {
            Some(Kind::Trait)
        } else if t.starts_with("impl") && t.contains(" for ") {
            Some(Kind::TraitImpl)
        } else if t.starts_with("impl") {
            Some(Kind::Impl)
        } else if t.starts_with("pub struct") || t.starts_with("struct ") {
            Some(Kind::Struct)
        } else if t.starts_with("pub enum") || t.starts_with("enum ") {
            Some(Kind::Enum)
        } else if t.starts_with("pub mod") || t.starts_with("mod ") {
            Some(Kind::Module)
        } else {
            None
        };
        if !public {
            docs.clear();
            attrs.clear();
            if has_block {
                skip_body(lines, i, indent)
            }
            continue;
        }
        let head = signature(e, kind);
        match (child, has_block) {
            (Some(k), true) => {
                let mut inner = String::new();
                items(lines, i, indent + 4, k, &mut inner);
                *i += 1; // the closing brace
                let inherent = k == Kind::Impl;
                if inherent && inner.trim().is_empty() {
                    // An inherent impl with nothing public is not interface.
                } else if inner.trim().is_empty() {
                    let body = if k == Kind::Struct { "{ .. }" } else { "{}" };
                    out.push_str(&format!(
                        "{docs}{attrs}{} {body}\n\n",
                        head.trim_end_matches('{').trim_end()
                    ));
                } else {
                    out.push_str(&format!(
                        "{docs}{attrs}{head}\n{inner}{pad}}}\n\n"
                    ));
                }
            }
            (Some(_), false) => {
                // `pub struct X;`, `pub struct X(..);`, or a one-line `{}`.
                out.push_str(&format!("{docs}{attrs}{head}\n"));
                if kind == Kind::Module {
                    out.push('\n')
                }
            }
            (None, _) => {
                let _ = start;
                out.push_str(&format!("{docs}{attrs}{head}\n"));
                if has_block && !one_line_body {
                    skip_body(lines, i, indent)
                }
                if t.starts_with("macro_rules!") {
                    out.push('\n')
                }
            }
        }
        docs.clear();
        attrs.clear();
    }
}

/// Skip to just past the line holding only `}` at `indent`.
fn skip_body(lines: &[&str], i: &mut usize, indent: usize) {
    let close = format!("{}}}", " ".repeat(indent));
    while *i < lines.len() {
        let l = lines[*i];
        *i += 1;
        if l == close {
            return;
        }
    }
}

/// The printed form of a signature: bodies dropped, private tuple fields
/// hidden, one-line bodies reduced to `;`.
fn signature(e: &str, kind: Kind) -> String {
    let t = e.trim_start();
    if t.starts_with("macro_rules!") {
        return format!("{} {{ .. }}", e.split('{').next().unwrap().trim_end());
    }
    // A tuple struct with private fields shows `(..)`.
    if (t.starts_with("pub struct") || t.starts_with("struct"))
        && !e.contains(" {")
    {
        if let Some(p) = paren_at_depth0(e) {
            if !e[p..].contains("pub ") {
                return format!("{}(..);", &e[..p]);
            }
        }
    }
    if e.ends_with('{') {
        let s = e.trim_end_matches('{').trim_end();
        return if matches!(
            kind,
            Kind::Trait | Kind::Impl | Kind::TraitImpl | Kind::Module
        ) && (t.starts_with("pub fn")
            || t.starts_with("fn")
            || t.starts_with("pub async fn")
            || t.starts_with("async fn"))
            || (kind == Kind::Module
                && !(t.starts_with("pub trait")
                    || t.starts_with("pub struct")
                    || t.starts_with("pub enum")
                    || t.starts_with("pub mod")
                    || t.starts_with("impl")))
        {
            format!("{s};")
        } else {
            format!("{s} {{")
        };
    }
    if e.contains('{') && e.ends_with('}') {
        // `fn f() { .. }` on one line, or `impl X for Y {}`.
        let s = e.split('{').next().unwrap().trim_end();
        let block = t.starts_with("impl")
            || t.starts_with("pub trait")
            || t.starts_with("pub enum");
        return if block {
            format!("{s} {{}}")
        } else if t.starts_with("pub struct") {
            format!("{s} {{ .. }}")
        } else {
            format!("{s};")
        };
    }
    e.to_string()
}

/// The `(` that opens a tuple struct's fields: the first at angle depth 0.
fn paren_at_depth0(s: &str) -> Option<usize> {
    let mut d = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '<' => d += 1,
            '>' => d -= 1,
            '(' if d == 0 => return Some(i),
            _ => {}
        }
    }
    None
}
