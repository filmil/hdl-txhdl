// SPDX-License-Identifier: Apache-2.0
//! A structural netlist, from the same walk the waveform uses. Every
//! unit is a module, every register a `reg`, and every wire or channel
//! whose ends are fields of different units is a net between them,
//! with a port on each and a wire in their common parent. The walk
//! sees fields, so a unit whose ports are fields yields ports, and a
//! unit that takes its ports as parameters of `run` yields only its
//! registers: that is the gap between a structural netlist and the
//! lowering, which the proc-macro route closes by reading the
//! `Unit` impl. Behaviour is not here at all; the bodies are empty.
use crate::comp::trace::{collect, Kind, Probe, Traceable};
use crate::comp::{Clock, In, Mem, Out, Reg, Rx, Tx};
use crate::types::Value;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::marker::PhantomData;

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
        modules
            .entry(scope_of(&p.path).to_string())
            .or_default()
            .push(p);
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
            let spans_children =
                e.iter().any(|q| child_of(path, scope_of(&q.path)) != first);
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

// ---------------------------------------------------------------------
// Lowering a unit: what `#[lower]` needs at run time

/// What a field of a unit is to a netlist: state or an end of a wire,
/// with a width, or nothing. Every type a unit may hold implements it,
/// and `#[derive(Trace)]` gives a unit its `Fields` from them.
pub trait Port {
    const KIND: Option<Kind> = None;
    const WIDTH: usize = 0;
}
impl<T: Value + Copy + 'static, C: Clock> Port for Reg<T, C> {
    const KIND: Option<Kind> = Some(Kind::Reg);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Value + Copy + 'static, C: Clock> Port for Out<T, C> {
    const KIND: Option<Kind> = Some(Kind::Out);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Value + Copy + 'static, C: Clock> Port for In<T, C> {
    const KIND: Option<Kind> = Some(Kind::In);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Value + crate::types::Transaction + 'static, C: Clock> Port
    for Tx<T, C>
{
    const KIND: Option<Kind> = Some(Kind::Tx);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Value + crate::types::Transaction + 'static, C: Clock> Port
    for Rx<T, C>
{
    const KIND: Option<Kind> = Some(Kind::Rx);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Copy, const N: usize, C: Clock> Port for Mem<T, N, C> {}
impl<T> Port for PhantomData<T> {}
macro_rules! plain {
    ($($t:ty),*) => { $( impl Port for $t {} )* };
}
plain!(u8, u16, u32, u64, u128, usize, bool, &'static str);
plain!(crate::types::Bit, crate::types::Logic);
impl<const N: usize> Port for crate::types::U<N> {}

/// A unit's fields by name, kind and width. Derived with `Trace`.
pub trait Fields {
    fn fields() -> Vec<(&'static str, Option<Kind>, usize)>;
}

/// A value as a sized Verilog literal, `W'b...`.
pub fn literal<V: Value>(v: V) -> String {
    format!("{}'b{}", V::WIDTH, v.vcd())
}

/// One statement of a lowered `run` body, as `#[lower]` emits it.
pub enum Stmt {
    /// `target.set(expr)` or `target <= expr`: a register takes it at
    /// the edge, a wire is assigned it.
    Drive(String, String),
    /// `when!(cond => { drives } else { drives })`.
    When(String, Vec<(String, String)>, Vec<(String, String)>),
    /// `case!(value => { pattern => { drives }, .. })`: arms in order,
    /// each a condition on the value, the first that holds wins.
    Case(Vec<(String, Vec<(String, String)>)>),
    /// The wait the loop makes: every register drive after it happens
    /// only at an edge at which the condition holds.
    Guard(String),
}

/// The Verilog of one unit: its ports from `run`'s signature, its
/// registers from its fields, and its body from the statements
/// `#[lower]` read out of `run`.
pub fn unit_verilog(
    name: &str,
    clock: &str,
    fields: &[(&'static str, Option<Kind>, usize)],
    ports: &[(&str, Kind, usize)],
    body: &[Stmt],
) -> String {
    let is_reg = |t: &str| {
        fields
            .iter()
            .any(|(n, k, _)| *n == t && *k == Some(Kind::Reg))
    };
    let mut out = String::new();
    let mut plist = vec![format!("input {clock}")];
    for (n, k, w) in ports {
        match k {
            Kind::Out => plist.push(format!("output {}{n}", range(*w))),
            Kind::In => plist.push(format!("input {}{n}", range(*w))),
            // A channel is a valid/ready handshake around its data.
            Kind::Tx => plist.push(format!(
                "output {}{n}_data, output {n}_valid, input {n}_ready",
                range(*w)
            )),
            Kind::Rx => plist.push(format!(
                "input {}{n}_data, input {n}_valid, output {n}_ready",
                range(*w)
            )),
            Kind::Reg => {}
        }
    }
    // One port per line, so a long list stays readable.
    writeln!(out, "module {name}(\n  {}\n);", plist.join(",\n  ")).unwrap();
    for (n, k, w) in fields {
        if *k == Some(Kind::Reg) {
            writeln!(out, "  reg {}{n};", range(*w)).unwrap();
        }
    }
    let mut seq: Vec<String> = Vec::new();
    let mut comb: Vec<String> = Vec::new();
    let mut guard: Option<String> = None;
    let drive = |seq: &mut Vec<String>,
                 comb: &mut Vec<String>,
                 indent: &str,
                 t: &str,
                 e: &str| {
        if is_reg(t) {
            seq.push(format!("{indent}{t} <= {e};"));
        } else {
            comb.push(format!("  assign {t} = {e};"));
        }
    };
    for st in body {
        match st {
            Stmt::Guard(c) => {
                guard = Some(c.clone());
                seq.push(format!("    if ({c}) begin"));
            }
            Stmt::Drive(t, e) => {
                let ind = if guard.is_some() { "      " } else { "    " };
                drive(&mut seq, &mut comb, ind, t, e)
            }
            Stmt::When(c, then, otherwise) => {
                seq.push(format!("    if ({c}) begin"));
                for (t, e) in then {
                    drive(&mut seq, &mut comb, "      ", t, e);
                }
                if !otherwise.is_empty() {
                    seq.push("    end else begin".into());
                    for (t, e) in otherwise {
                        drive(&mut seq, &mut comb, "      ", t, e);
                    }
                }
                seq.push("    end".into());
            }
            Stmt::Case(arms) => {
                for (i, (c, drives)) in arms.iter().enumerate() {
                    let kw = if i == 0 { "if" } else { "end else if" };
                    seq.push(format!("    {kw} ({c}) begin"));
                    for (t, e) in drives {
                        drive(&mut seq, &mut comb, "      ", t, e);
                    }
                }
                if !arms.is_empty() {
                    seq.push("    end".into());
                }
            }
        }
    }
    if guard.is_some() {
        seq.push("    end".into());
    }
    let has_drives = seq.iter().any(|l| l.contains("<="));
    if has_drives {
        writeln!(out, "  always @(posedge {clock}) begin").unwrap();
        for l in &seq {
            writeln!(out, "{l}").unwrap();
        }
        writeln!(out, "  end").unwrap();
    }
    for l in &comb {
        writeln!(out, "{l}").unwrap();
    }
    writeln!(out, "endmodule").unwrap();
    out
}
