// SPDX-License-Identifier: Apache-2.0
//! Does each hand-written Verilog top still fit the lowered modules it
//! instantiates?
//!
//! A lowered unit's ports are written by the build, and a hand-written
//! top connects them by name. Nothing else checks that the two agree:
//! every top in the tree is synthesised by Vivado, and every Vivado
//! target is manual, so a lowered unit whose ports change can leave a
//! top that no longer elaborates, or that leaves an input undriven,
//! behind a green suite (issue 508). Both have happened. Converting
//! `Hdmi` to one bus port renamed fifteen of its ports, and the two tops
//! that connect it would have broken unseen; and the implicit reset of
//! #368 was never connected in any top that instantiates a lowered
//! module directly, except `board` (issue 509).
//!
//! So this reads each generated netlist's module headers, which give
//! every port its name and direction, and then every instance of those
//! modules in the hand-written tops, and it fails on two things:
//!
//! * a port the top connects that the module does not declare, which is
//!   what a rename leaves, and which would not elaborate;
//! * an input or inout the module declares that the top leaves
//!   unconnected, which elaborates and is undriven.
//!
//! An output left unconnected is allowed: a top may ignore what a
//! module tells it. Modules the generated netlists do not define, the
//! hand-written ones and the vendor primitives, are not its business.
//!
//! A gap that is known and has an issue of its own is named in the
//! known-gap file as `module.port #issue`, and is reported rather than
//! failed. A known gap that is no longer a gap FAILS, so the entry is
//! deleted by the change that closes it rather than left to excuse the
//! same fault later.
//!
//! Usage:
//!
//! ```text
//! topcheck --known FILE [--group --tops A.v ... --netlists X.v ...]...
//! ```
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::process::ExitCode;

/// Which way a port faces, as its module declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    In,
    Out,
    InOut,
}

/// A module's ports, by name, as its header declares them.
pub type Ports = BTreeMap<String, Dir>;

/// Verilog text with its comments and string literals blanked out, so
/// that nothing inside them is read as a name.
pub fn strip(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            out.push(' ');
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += 1;
            }
            i += 1;
            out.push(' ');
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// The text as names, numbers and single punctuation marks.
pub fn tokens(src: &str) -> Vec<String> {
    let s = strip(src);
    let c: Vec<char> = s.chars().collect();
    let mut t = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch.is_ascii_alphabetic() || ch == '_' {
            let st = i;
            while i < c.len()
                && (c[i].is_ascii_alphanumeric() || c[i] == '_' || c[i] == '$')
            {
                i += 1;
            }
            t.push(c[st..i].iter().collect());
        } else if ch.is_ascii_digit() {
            let st = i;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            t.push(c[st..i].iter().collect());
        } else {
            t.push(ch.to_string());
            i += 1;
        }
    }
    t
}

/// Every module a generated netlist defines, with its ports.
pub fn modules(src: &str) -> BTreeMap<String, Ports> {
    let t = tokens(src);
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < t.len() {
        if t[i] == "module" && i + 2 < t.len() && t[i + 2] == "(" {
            let name = t[i + 1].clone();
            let mut ports = Ports::new();
            let mut dir = Dir::In;
            let mut depth = 0;
            let mut j = i + 2;
            while j < t.len() {
                match t[j].as_str() {
                    "(" => depth += 1,
                    ")" => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    "[" => {
                        // A width: skip to its close.
                        while j < t.len() && t[j] != "]" {
                            j += 1;
                        }
                    }
                    "input" => dir = Dir::In,
                    "output" => dir = Dir::Out,
                    "inout" => dir = Dir::InOut,
                    "wire" | "reg" | "signed" | "logic" | "," => {}
                    w if depth == 1 && is_name(w) => {
                        ports.insert(w.to_string(), dir);
                    }
                    _ => {}
                }
                j += 1;
            }
            out.insert(name, ports);
            i = j;
        }
        i += 1;
    }
    out
}

fn is_name(w: &str) -> bool {
    w.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

/// One instance in a top: the module, its instance name, and the ports
/// it connects by name.
#[derive(Debug)]
pub struct Instance {
    pub module: String,
    pub name: String,
    pub connects: BTreeSet<String>,
}

/// Every instance, in a top, of a module in `known`.
pub fn instances(src: &str, known: &BTreeMap<String, Ports>) -> Vec<Instance> {
    let t = tokens(src);
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let is_inst =
            known.contains_key(&t[i]) && (i == 0 || t[i - 1] != "module");
        if !is_inst {
            i += 1;
            continue;
        }
        let module = t[i].clone();
        let mut j = i + 1;
        // Parameters, if any: `#( ... )`.
        if j < t.len() && t[j] == "#" {
            j += 1;
            j = skip_parens(&t, j);
        }
        if j + 1 >= t.len() || !is_name(&t[j]) || t[j + 1] != "(" {
            i += 1;
            continue;
        }
        let name = t[j].clone();
        let mut connects = BTreeSet::new();
        let mut depth = 0;
        let mut k = j + 1;
        while k < t.len() {
            match t[k].as_str() {
                "(" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                "." if depth == 1 && k + 1 < t.len() => {
                    connects.insert(t[k + 1].clone());
                }
                _ => {}
            }
            k += 1;
        }
        out.push(Instance {
            module,
            name,
            connects,
        });
        i = k;
    }
    out
}

/// The index just past a balanced `( ... )` that starts at `j`.
fn skip_parens(t: &[String], mut j: usize) -> usize {
    if j >= t.len() || t[j] != "(" {
        return j;
    }
    let mut depth = 0;
    while j < t.len() {
        if t[j] == "(" {
            depth += 1;
        } else if t[j] == ")" {
            depth -= 1;
            if depth == 0 {
                return j + 1;
            }
        }
        j += 1;
    }
    j
}

/// A gap that has an issue of its own, `module.port`, and the issue.
pub type Known = BTreeMap<(String, String), String>;

/// The known-gap file: one `module.port #issue` a line, `#` comments
/// and blank lines ignored.
pub fn known(src: &str) -> Known {
    let mut out = Known::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(mp), Some(issue)) = (parts.next(), parts.next()) else {
            continue;
        };
        if let Some((m, p)) = mp.split_once('.') {
            out.insert((m.to_string(), p.to_string()), issue.to_string());
        }
    }
    out
}

/// What checking one top found: the faults, and the known gaps it met.
#[derive(Debug, Default)]
pub struct Found {
    pub faults: Vec<String>,
    pub known_met: BTreeSet<(String, String)>,
}

/// Every instance in a top against the modules' declared ports.
pub fn check(
    top: &str,
    top_src: &str,
    mods: &BTreeMap<String, Ports>,
    gaps: &Known,
) -> Found {
    let mut f = Found::default();
    for inst in instances(top_src, mods) {
        let ports = &mods[&inst.module];
        for p in &inst.connects {
            if !ports.contains_key(p) {
                f.faults.push(format!(
                    "{top}: `{}` ({}) connects `{p}`, which `{}` does not declare",
                    inst.name, inst.module, inst.module
                ));
            }
        }
        for (p, d) in ports {
            if *d == Dir::Out || inst.connects.contains(p) {
                continue;
            }
            let key = (inst.module.clone(), p.clone());
            if gaps.contains_key(&key) {
                f.known_met.insert(key);
            } else {
                f.faults.push(format!(
                    "{top}: `{}` ({}) leaves its input `{p}` unconnected",
                    inst.name, inst.module
                ));
            }
        }
    }
    f
}

struct Group {
    tops: Vec<String>,
    netlists: Vec<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut known_file = None;
    let mut groups: Vec<Group> = Vec::new();
    let mut into: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--known" => {
                i += 1;
                known_file = args.get(i).cloned();
            }
            "--group" => groups.push(Group {
                tops: vec![],
                netlists: vec![],
            }),
            "--tops" => into = Some("tops"),
            "--netlists" => into = Some("netlists"),
            path => match (into, groups.last_mut()) {
                (Some("tops"), Some(g)) => g.tops.push(path.to_string()),
                (Some("netlists"), Some(g)) => {
                    g.netlists.push(path.to_string())
                }
                _ => {
                    eprintln!("topcheck: `{path}` outside any --group --tops/--netlists");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }
    let read = |p: &str| {
        fs::read_to_string(p).unwrap_or_else(|e| panic!("reading {p}: {e}"))
    };
    let gaps = known_file.map(|f| known(&read(&f))).unwrap_or_default();

    let mut faults = Vec::new();
    let mut met: BTreeSet<(String, String)> = BTreeSet::new();
    let mut checked = 0;
    let mut defined: BTreeSet<String> = BTreeSet::new();
    for g in &groups {
        let mut mods = BTreeMap::new();
        for n in &g.netlists {
            mods.extend(modules(&read(n)));
        }
        defined.extend(mods.keys().cloned());
        for top in &g.tops {
            let src = read(top);
            checked += instances(&src, &mods).len();
            let f = check(top, &src, &mods, &gaps);
            faults.extend(f.faults);
            met.extend(f.known_met);
        }
    }
    // A known gap that nothing met is either closed or misnamed; both
    // want the entry gone, so both fail.
    for ((m, p), issue) in &gaps {
        if defined.contains(m) && !met.contains(&(m.clone(), p.clone())) {
            faults.push(format!(
                "known gap `{m}.{p}` ({issue}) is connected everywhere now: delete its entry"
            ));
        }
    }
    for (m, p) in &met {
        println!(
            "known: `{m}.{p}` unconnected ({})",
            gaps[&(m.clone(), p.clone())]
        );
    }
    if faults.is_empty() {
        println!("topcheck: {checked} instance(s) of lowered modules, every one fits");
        ExitCode::SUCCESS
    } else {
        for f in &faults {
            println!("{f}");
        }
        println!("topcheck: {} fault(s)", faults.len());
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: &str = "
        module unit(
          input clk,
          input rst,
          input [34:0] bus_aw_data, input bus_aw_valid, output bus_aw_ready,
          output [3:0] leds,
          inout [3:0] dq
        );
        endmodule";

    fn mods() -> BTreeMap<String, Ports> {
        modules(NET)
    }

    #[test]
    fn the_header_gives_every_port_its_direction() {
        let m = mods();
        let p = &m["unit"];
        assert_eq!(p["clk"], Dir::In);
        assert_eq!(p["bus_aw_ready"], Dir::Out);
        assert_eq!(p["dq"], Dir::InOut);
        assert_eq!(p.len(), 7);
    }

    #[test]
    fn a_top_that_fits_passes() {
        let top = "module t(); unit u (.clk(c), .rst(r), .bus_aw_data(d), \
                   .bus_aw_valid(v), .bus_aw_ready(y), .leds(l), .dq(q)); endmodule";
        assert!(check("t.v", top, &mods(), &Known::new()).faults.is_empty());
    }

    #[test]
    fn a_renamed_port_is_a_fault() {
        // What converting a unit to one bus port does to a top that was
        // not updated: the old names are connected and no longer exist.
        let top =
            "unit u (.clk(c), .rst(r), .aw_data(d), .bus_aw_valid(v), .dq(q));";
        let f = check("t.v", top, &mods(), &Known::new());
        assert!(
            f.faults.iter().any(|x| x.contains("`aw_data`")),
            "{:?}",
            f.faults
        );
    }

    #[test]
    fn an_unconnected_input_is_a_fault_and_an_output_is_not() {
        let top =
            "unit u (.clk(c), .bus_aw_data(d), .bus_aw_valid(v), .dq(q));";
        let f = check("t.v", top, &mods(), &Known::new());
        assert!(
            f.faults.iter().any(|x| x.contains("input `rst`")),
            "{:?}",
            f.faults
        );
        // `bus_aw_ready` and `leds` are outputs, left dangling on purpose.
        assert!(!f
            .faults
            .iter()
            .any(|x| x.contains("leds") || x.contains("ready")));
    }

    #[test]
    fn an_unconnected_inout_is_a_fault() {
        let top =
            "unit u (.clk(c), .rst(r), .bus_aw_data(d), .bus_aw_valid(v));";
        let f = check("t.v", top, &mods(), &Known::new());
        assert!(
            f.faults.iter().any(|x| x.contains("`dq`")),
            "{:?}",
            f.faults
        );
    }

    #[test]
    fn a_known_gap_is_reported_rather_than_failed() {
        let top =
            "unit u (.clk(c), .bus_aw_data(d), .bus_aw_valid(v), .dq(q));";
        let gaps = known("unit.rst #509\n");
        let f = check("t.v", top, &mods(), &gaps);
        assert!(f.faults.is_empty(), "{:?}", f.faults);
        assert!(f.known_met.contains(&("unit".into(), "rst".into())));
    }

    #[test]
    fn comments_and_strings_are_not_read_as_ports() {
        let top = "unit u ( // .ghost(x)\n .clk(c), /* .rst(r) */ .rst(r), \
                   .bus_aw_data(d), .bus_aw_valid(v), .dq(q)); \
                   ODDR #(.X(\"unit u (.bad(b))\")) o (.Q(q));";
        let f = check("t.v", top, &mods(), &Known::new());
        assert!(f.faults.is_empty(), "{:?}", f.faults);
    }

    #[test]
    fn a_module_the_netlists_do_not_define_is_left_alone() {
        // Hand-written or vendor: parameters and all, not this check's.
        let top = "chan_cdc #(.W(9), .AW(7)) x (.wr_clk(c)); ODDR o (.Q(q));";
        let f = check("t.v", top, &mods(), &Known::new());
        assert!(f.faults.is_empty());
        assert!(instances(top, &mods()).is_empty());
    }

    #[test]
    fn the_definition_of_a_module_is_not_an_instance_of_it() {
        let src = "module unit(input clk); endmodule";
        assert!(instances(src, &mods()).is_empty());
    }
}
