// SPDX-License-Identifier: Apache-2.0
//! A structural netlist, from the same walk the waveform uses. Every
//! unit is a module, every register a `reg`, and every wire or channel
//! whose ends are fields of different units is a net between them,
//! with a port on each and a wire in their common parent. The walk
//! sees fields, so a unit whose ports are fields yields ports, and a
//! unit that takes its ports as parameters of `run` yields only its
//! registers: that is the gap between a structural netlist and the
//! lowering, which the proc-macro route closes by reading the
//! `Module` impl. Behaviour is not here at all; the bodies are empty.
use crate::comp::trace::{collect, Kind, Probe, Traceable};
use std::collections::BTreeMap;
use std::fmt::Write;

/// The Verilog skeleton of `top`.
pub fn verilog(name: &str, top: &impl Traceable) -> String {
    let probes = collect(name, top);
    let ends = |cell: usize| -> Vec<&Probe> {
        probes
            .iter()
            .filter(|p| p.kind != Kind::Reg && p.cell == cell)
            .collect()
    };
    // Every scope that holds something, plus every ancestor.
    let mut modules: BTreeMap<String, Vec<&Probe>> = BTreeMap::new();
    for p in &probes {
        modules.entry(scope_of(&p.path).to_string()).or_default().push(p);
        let mut s = scope_of(&p.path);
        while let Some((parent, _)) = s.rsplit_once('.') {
            modules.entry(parent.to_string()).or_default();
            s = parent;
        }
    }
    let mut out = String::new();
    for (path, items) in &modules {
        let mut ports = Vec::new();
        let mut regs = Vec::new();
        for p in items {
            if p.kind == Kind::Reg {
                let r = format!("  reg {}{};", range(p.width), leaf(&p.path));
                regs.push(r);
            } else if ends(p.cell).iter().any(|q| scope_of(&q.path) != path) {
                let dir = match p.kind {
                    Kind::Out | Kind::Tx => "output",
                    _ => "input",
                };
                let w = range(p.width);
                ports.push(format!("{dir} {w}{}", leaf(&p.path)));
            }
        }
        writeln!(out, "module {}({});", ident(path), ports.join(", ")).unwrap();
        for r in regs {
            writeln!(out, "{r}").unwrap();
        }
        // Nets whose ends lie in this module's children: declared here.
        let mut seen = Vec::new();
        for p in &probes {
            if p.kind == Kind::Reg || seen.contains(&p.cell) {
                continue;
            }
            let e = ends(p.cell);
            let below = |q: &&Probe| {
                scope_of(&q.path)
                    .strip_prefix(path.as_str())
                    .map_or(false, |r| r.starts_with('.'))
            };
            if e.len() < 2 || !e.iter().all(below) {
                continue;
            }
            let first = child_of(path, scope_of(&e[0].path));
            let spans_children = e
                .iter()
                .any(|q| child_of(path, scope_of(&q.path)) != first);
            if spans_children {
                seen.push(p.cell);
                let w = range(p.width);
                writeln!(out, "  wire {w}{};", net_name(&e)).unwrap();
            }
        }
        // Instances: the direct children, ports connected to the nets.
        for (child, citems) in &modules {
            let Some(rest) = child.strip_prefix(&format!("{path}.")) else {
                continue;
            };
            if rest.contains('.') {
                continue;
            }
            let mut conns = Vec::new();
            for p in citems {
                if p.kind == Kind::Reg {
                    continue;
                }
                let e = ends(p.cell);
                if e.iter().any(|q| scope_of(&q.path) != child) {
                    conns.push(format!(".{}({})", leaf(&p.path), net_name(&e)));
                }
            }
            let c = conns.join(", ");
            writeln!(out, "  {} {rest}({c});", ident(child)).unwrap();
        }
        writeln!(out, "endmodule\n").unwrap();
    }
    out
}

/// A net is named after its driving end, or its first end.
fn net_name(ends: &[&Probe]) -> String {
    let d = ends
        .iter()
        .find(|p| matches!(p.kind, Kind::Out | Kind::Tx))
        .unwrap_or(&ends[0]);
    leaf(&d.path).replace('.', "_")
}
/// The child of `parent` on the way to `scope`.
fn child_of<'a>(parent: &str, scope: &'a str) -> &'a str {
    let rest = &scope[parent.len() + 1..];
    rest.split('.').next().unwrap()
}
fn leaf(path: &str) -> &str {
    path.rsplit('.').next().unwrap()
}
fn scope_of(path: &str) -> &str {
    path.rsplit_once('.').map(|(a, _)| a).unwrap_or("")
}
fn ident(path: &str) -> String {
    path.replace('.', "_")
}
fn range(w: usize) -> String {
    if w > 1 {
        format!("[{}:0] ", w - 1)
    } else {
        String::new()
    }
}
