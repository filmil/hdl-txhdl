// SPDX-License-Identifier: Apache-2.0
//! A VHDL testbench from a trace. The Rust simulation wrote an FST; the
//! lowering wrote the entity and a ports file. This replays the trace's
//! inputs into the entity and asserts, at every tick, that the entity's
//! registers and outputs are what the trace holds.
//!
//! Time: one tick is 1 ns and the clock rises at even ticks. An input
//! the trace holds at tick 2k was set before that edge, so it is applied
//! at 2k-1. A register the trace holds at tick 2k took its value at that
//! edge, so it is checked at 2k+1. A wire the Rust process drove at
//! tick 2k was computed from the registers as they stood before that
//! edge, so it is checked at 2k-1, where the entity's continuous
//! assignment shows the same value; and from the inputs of that edge,
//! so the inputs are applied a tenth of a tick before 2k-1, every wire
//! settles however deep, and the checks come just before 2k-1, ahead
//! of the falling edge there, which is where the Verilog testbench
//! checks too; a falling-edge register is therefore checked against
//! the trace at 2k-1, the edge before. A channel's inputs, the head
//! of the buffer on the receiving side and the room on the sending
//! side, are registered state of the channel: the trace at 2k holds
//! them as that edge left them, which is what the entity sees during
//! the cycle after it, so
//! they are applied at 2k+1. A sender's data is checked only at ticks
//! where the trace has its valid high, since under a low valid the
//! trace holds the last offer and the entity computes the wire anyway.
//!
//! Held and pulsed are the two kinds of signal here, and they are
//! checked differently. A register and a wire are held: each keeps its
//! value until the next edge, so reading one late gives the same value
//! later and the only question is which edge to count from, which is
//! what the per-port period above settles. A channel's `ready` and
//! `valid` are not held. The runtime asserts one while the `recv` or
//! `send` runs, so the trace has it high at that port's own clock edge
//! and low at every tick between, while the entity computes the same
//! wire combinationally and may legitimately show something else in
//! between. There is no offset that turns a pulse into a held signal,
//! so a handshake is compared only at a tick which is an edge of its
//! own clock, and the frame says nothing about it elsewhere.
//!
//! That is the trace having no opinion between edges rather than the
//! check being dropped where it was inconvenient, and the difference
//! is worth stating because they look the same from outside. Only the
//! value at an edge is read by anything synchronous: the handshake
//! decides whether a transfer happened at that edge, and no register
//! on either side samples it in between. So the entity's wire being
//! high at a tick where the trace holds low is not two implementations
//! disagreeing, it is a tick at which nothing asked. The boundary that
//! follows is real and worth knowing: this testbench cannot catch an
//! entity whose handshake is wrong strictly between its own edges.
//! Nothing reads it there, and nothing here looks. A
//! sender's data follows its valid, for the same reason and at the
//! same ticks. Under the default clock every tick the testbench visits
//! is an edge, which is why this was invisible until a unit had a
//! channel on a slower clock (issue 384).
//!
//! Form: in VHDL, one procedure holds a tick; a cycle's inputs and
//! expected values are one string of bits, a frame, kept as hex, and
//! the replay is a loop over constant arrays of frames, each array a
//! package of its own of a few hundred frames. nvc's heap for one
//! design unit is finite, and a statement per check, or a call per
//! cycle with an argument per signal, is more than it holds for a run
//! of a few hundred cycles; a literal is data, but a unit of half a
//! megabyte of them is over the limit too, so they are spread.
//!
//! Usage: fst2tb FILE.fst FILE.vhd.ports ENTITY UNIT
//!          [--verilog VECTORS PATH] > tb
//! where UNIT is the name the Rust testbench gave the unit in the trace;
//! `--verilog` writes the same testbench in Verilog, for Verilator, with
//! its frames in the file VECTORS, which the testbench opens as PATH.
use std::collections::BTreeMap;

// The netlist's reserved words, from the file the runtime and the
// macros read, so that the three cannot disagree (issue 497).
#[path = "../../lib/src/reserved.rs"]
mod reserved;

/// Whether a port starts high: the `ready` a unit reads from a
/// channel it sends on, which is high at power-on because an empty
/// elastic buffer has room. It matters for one cycle and one only.
/// The testbench's first edge is the run's first cycle, and no trace
/// sample stands for the inputs of that cycle: the sample at tick
/// zero already holds the state it left and the inputs of the cycle
/// after it. Leaving every input at zero therefore says the run
/// began with every channel full, which is the one thing a fresh
/// channel is not, and a unit that acts in its first cycle, as the
/// rasteriser does when it goes looking for its display list, is
/// then a cycle out for the whole run.
/// The names a trace carries more than once, each with the widths it
/// was carried at, in the order the trace names them.
///
/// The width is in the message rather than in the decision: a name
/// carried twice is a fault whether or not the widths agree, and
/// agreeing widths are the worse case because nothing downstream
/// notices. Reporting both lets a reader see which unit's signal the
/// testbench had been reading.
fn collisions(read: &[(String, Vec<String>)]) -> Vec<(String, Vec<usize>)> {
    let mut seen: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut order: Vec<&str> = Vec::new();
    for (name, per_tick) in read {
        let width = per_tick
            .iter()
            .find(|v| !v.is_empty())
            .map_or(0, |v| v.len());
        let e = seen.entry(name.as_str()).or_default();
        if e.is_empty() {
            order.push(name.as_str());
        }
        e.push(width);
    }
    order
        .into_iter()
        .filter(|n| seen[n].len() > 1)
        .map(|n| (n.to_string(), seen[n].clone()))
        .collect()
}

fn empty_ready(name: &str, dir: &str) -> bool {
    dir == "txin" && name.ends_with("_ready")
}

/// The name each register takes in the testbench, where it is read
/// through an alias of the signal inside the entity. The name is
/// `reg_<register>`, lengthened by another `reg_` while it is a name
/// the testbench already declares.
///
/// The prefix used to be `r_`, which a channel port called `r` also
/// writes: a unit with a register called `data` and such a port
/// declared `r_data` twice, and every check of the port read the
/// register instead, so nvc reported the port as differing for the
/// whole run. That is issue 211.
fn aliases(
    regs: &[String],
    declared: &[String],
) -> std::collections::HashMap<String, String> {
    let mut used: std::collections::HashSet<String> =
        declared.iter().cloned().collect();
    // The testbench's own signal, declared beside the ports.
    used.insert("errors".to_string());
    let mut out = std::collections::HashMap::new();
    for n in regs {
        let mut a = format!("reg_{n}");
        while used.contains(&a) {
            a = format!("reg_{a}");
        }
        used.insert(a.clone());
        out.insert(n.clone(), a);
    }
    out
}

/// A string of bits as `$readmemh` reads a vector: hex digits, the
/// first bit the highest, padded with zeros on the left. The VHDL
/// frames pad on the right instead, since there a frame is a string
/// read from its first character.
fn vhex(bits: &str) -> String {
    let pad = (4 - bits.len() % 4) % 4;
    let bits = format!("{}{bits}", "0".repeat(pad));
    bits.as_bytes()
        .chunks(4)
        .map(|c| {
            let v = c.iter().fold(0u32, |v, &b| v << 1 | (b == b'1') as u32);
            char::from_digit(v, 16).unwrap()
        })
        .collect()
}

/// Names for the testbench's own signals, each `base` lengthened by
/// `tb_` in front while it is a name in `taken` or one already given.
fn fresh<const N: usize>(bases: &[&str; N], taken: &[String]) -> [String; N] {
    let mut used: std::collections::HashSet<String> =
        taken.iter().cloned().collect();
    used.insert("errors".to_string());
    bases.map(|b| {
        let mut n = b.to_string();
        while used.contains(&n) {
            n = format!("tb_{n}");
        }
        used.insert(n.clone());
        n
    })
}

/// An expected value into a frame, followed by whether to check it: a
/// value the trace holds, or zeros and no check where it holds none.
fn push_expected(f: &mut String, w: usize, v: Option<String>) {
    match v {
        Some(v) => {
            f.push_str(&v);
            f.push('1');
        }
        None => {
            f.push_str(&"0".repeat(w));
            f.push('0');
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (fst, ports, entity, unit) = (&a[1], &a[2], &a[3], &a[4]);
    // The entity as the netlist names it: escaped where VHDL or
    // SystemVerilog reserves its name, as the netlist writer escapes
    // it (issue 497). The testbench keeps the name it was given, with
    // `_tb` after it, since that is what the build asks to run.
    let module = reserved::escaped(entity);
    let verilog = a.get(5).map(|s| s == "--verilog").unwrap_or(false);
    // Where the Verilog testbench's vectors are written, and the path
    // the testbench opens them by when it runs.
    let (vectors_out, vectors_path) = if verilog {
        (
            a.get(6).expect("--verilog VECTORS PATH").clone(),
            a.get(7).expect("--verilog VECTORS PATH").clone(),
        )
    } else {
        (String::new(), String::new())
    };
    // The ports file may hold several entities, each section opened by
    // `entity NAME`; the one asked for is taken, or everything when the
    // file has no sections.
    let mut section: Option<String> = None;
    // A fourth field names the trace scope a port is found under, when
    // it is not the port's own name: a channel two units share under
    // one name in the run, with a port name of their own on each side.
    let mut scopes: Vec<(String, String)> = Vec::new();
    // Which clock each port is on, when it is not the default one.
    let mut port_clocks: Vec<(String, String)> = Vec::new();
    let ports: Vec<(String, String, usize)> = std::fs::read_to_string(ports)
        .expect("ports")
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() == 2 && f[0] == "entity" {
                section = Some(f[1].to_string());
                return None;
            }
            if section.as_deref().is_some_and(|s| s != module) {
                return None;
            }
            // A memory's line has a depth as its fourth field and is not
            // a port.
            if f.len() >= 4 && f[1] == "mem" {
                return None;
            }
            // After the width a port's line carries two optional
            // fields: its trace scope, and the clock it is on, which
            // is marked with `@` so the two are told apart rather
            // than counted (issue 131). A port on the default clock
            // says nothing, which is what every unit of one clock
            // does.
            for x in f.iter().skip(3) {
                match x.strip_prefix('@') {
                    Some(c) => {
                        port_clocks.push((f[0].to_string(), c.to_string()))
                    }
                    None => scopes.push((f[0].to_string(), x.to_string())),
                }
            }
            (f.len() >= 3).then(|| {
                (f[0].to_string(), f[1].to_string(), f[2].parse().unwrap())
            })
        })
        .collect();
    let opts = wellen::LoadOptions::default();
    let mut w =
        wellen::simple::read_with_options(fst, &opts).expect("read fst");
    let times: Vec<u64> = w.time_table().to_vec();
    // Every variable's value per time index, by its full name.
    let h = w.hierarchy();
    let vars: Vec<(String, wellen::SignalRef)> = h
        .all_vars()
        .map(|r| {
            let v = &h[r];
            (v.full_name(h), v.signal_ref())
        })
        .collect();
    let ids: Vec<wellen::SignalRef> = vars.iter().map(|(_, s)| *s).collect();
    w.load_signals(&ids);
    // Read every signal first and look for a name carried twice
    // before any of it goes into a map keyed by name, since the map
    // is where the second one would replace the first.
    let mut read: Vec<(String, Vec<String>)> = Vec::new();
    for (name, s) in &vars {
        let sig = w.get_signal(*s).unwrap();
        let mut per_tick = vec![String::new(); times.len()];
        let mut last = String::new();
        let mut changes = sig.iter_changes().peekable();
        for (i, _) in times.iter().enumerate() {
            while let Some((ti, val)) = changes.peek() {
                if (*ti as usize) <= i {
                    last = val.to_bit_string().unwrap_or_default();
                    changes.next();
                } else {
                    break;
                }
            }
            per_tick[i] = last.clone();
        }
        read.push((name.clone(), per_tick));
    }

    // A wave records a signal under its port's name, so two lowered
    // units in one run that both have a port called `out` record two
    // signals called `out`, and whichever is read second would win.
    // The testbench then compares one unit's port against the other
    // unit's trace.
    //
    // That is only loud when the widths differ, and then it is loud
    // in the wrong place: nvc says "expected 41 elements in string
    // literal but have 35" about the generated file, which says
    // nothing about a collision. When the widths agree it is silent,
    // and a testbench that compares the wrong signal can pass. Issue
    // 462 had three collisions in one run, `out`, `bytes` and `go`,
    // and only `out` was reported.
    let clash = collisions(&read);
    if !clash.is_empty() {
        let each: Vec<String> = clash
            .iter()
            .map(|(n, w)| {
                let w: Vec<String> = w.iter().map(|b| b.to_string()).collect();
                format!("`{n}` at {} bits", w.join(" and "))
            })
            .collect();
        eprintln!(
            "fst2tb: the trace of {entity} carries {} name{} more than \
             once: {}.\nTwo lowered units in one run share a port \
             name, so the wave holds a signal of that name for each \
             and a testbench cannot say which it meant.\nGive the \
             channels distinct names where they are added to the \
             wave, as `Wave::add(\"frame_out\", ..)` rather than \
             `Wave::add(\"out\", ..)` for each.",
            clash.len(),
            if clash.len() == 1 { "" } else { "s" },
            each.join(", ")
        );
        std::process::exit(1);
    }

    let values: BTreeMap<String, Vec<String>> = read.into_iter().collect();
    // Every clock the netlist names, in the order it names them. The
    // first is the one the testbench counts ticks by; the rest are
    // driven beside it, each at its own period and phase, which is
    // what lets a unit of several clocks be checked at all (issue
    // 131). A ports file that marks none, which is one written before
    // this, leaves `clk`.
    let clocks: Vec<String> = ports
        .iter()
        .filter(|p| p.1 == "clock")
        .map(|p| p.0.clone())
        .collect();
    let clocks = if clocks.is_empty() {
        vec!["clk".to_string()]
    } else {
        clocks
    };

    // Directions: in, out, reg, and a channel's rxin, rxout, txin,
    // txout. A channel's wire is traced under the channel by its side.
    // A clock is driven by the testbench as an input is, so it
    // declares like one; `is_in` is what says "the testbench drives
    // this", and the places that read a value from the trace ask for
    // the direction by name instead.
    let is_in =
        |d: &str| d == "in" || d == "rxin" || d == "txin" || d == "clock";
    // A wire kept as a field is checked as an output is, but reached
    // inside the entity, as a register is.
    let is_out =
        |d: &str| d == "out" || d == "rxout" || d == "txout" || d == "wire";
    let registered = |d: &str| d == "rxin" || d == "txin";
    let is_reg = |d: &str| d == "reg" || d == "regf";
    let inside = |d: &str| d == "reg" || d == "regf" || d == "wire";
    let scope_of = |port: &str| -> Option<String> {
        scopes
            .iter()
            .find(|(p, _)| p == port)
            .map(|(_, s)| s.clone())
    };
    let trace_name = |port: &str, dir: &str| -> String {
        if dir == "clock" {
            // A wave records its clocks together, under `clocks`, so a
            // clock is looked up there and not by its bare name.
            format!("clocks.{port}")
        } else if inside(dir) {
            format!("{unit}.{port}")
        } else if dir.starts_with("rx") || dir.starts_with("tx") {
            let (ch, part) = port.rsplit_once('_').unwrap_or((port, ""));
            let ch = scope_of(port).unwrap_or_else(|| ch.to_string());
            format!("{ch}.{}_{part}", &dir[..2])
        } else {
            scope_of(port).unwrap_or_else(|| port.to_string())
        }
    };
    let lit = |w: usize, bits: &str| {
        if w == 1 {
            format!("'{bits}'")
        } else {
            format!("\"{bits}\"")
        }
    };
    let ty = |w: usize| {
        if w == 1 {
            "std_logic".to_string()
        } else {
            format!("unsigned({} downto 0)", w - 1)
        }
    };
    let at = |i: usize| -> Option<String> {
        None.or(Some(String::new())).filter(|_| i < times.len())
    };
    let _ = at;
    // A clock's first rising tick and the ticks between its rising
    // edges, read from the trace rather than assumed, so that a clock
    // of any rate is replayed at the rate it ran (issue 131). A clock
    // the trace says nothing about keeps the default clock's two ticks
    // a cycle, rising at zero.
    //
    // A shape is a constant of the trace, and it is asked for per port
    // per cycle, so it is found once per clock and kept; only the first
    // two rising edges are ever used, so the search stops there. Found
    // afresh each time, it made a run's testbench cost the square of
    // the run's length: over an hour for the SD host's (issue 629).
    let shapes: std::cell::RefCell<
        std::collections::HashMap<String, (usize, usize)>,
    > = Default::default();
    let clock_shape = |name: &str| -> (usize, usize) {
        if let Some(s) = shapes.borrow().get(name) {
            return *s;
        }
        let s = find_clock_shape(values.get(name));
        shapes.borrow_mut().insert(name.to_string(), s);
        s
    };
    // How many ticks a port's own clock takes for a cycle. A check
    // one cycle after a drive means one cycle of the port's clock,
    // not of the testbench's first one: before issue 131 every port
    // was read two ticks on, so a port on a slower clock was compared
    // against a value its own edge had not produced yet, and the two
    // disagreed for part of every period.
    let port_period = |port: &str| -> usize {
        let named = port_clocks.iter().find(|(p, _)| p == port);
        let c = match named {
            Some((_, c)) => c.clone(),
            None => clocks[0].clone(),
        };
        clock_shape(&trace_name(&c, "clock")).1
    };
    // Whether a tick is a rising edge of a port's own clock.
    //
    // A channel's `ready` and `valid` are not held between edges. The
    // runtime asserts one while the `recv` or `send` runs, and the
    // trace therefore has it high at that clock's edge tick and low at
    // every tick between. The entity computes the same wire
    // combinationally, so between those edges it may legitimately show
    // something else, and the two are not comparable there: there is
    // no value the trace can offer for a tick at which it holds
    // nothing. A held signal, a wire or a register, is the other case
    // and is checked at every tick as before.
    //
    // For the default clock every tick the testbench visits is an edge
    // and this is always true, which is why the fault was invisible
    // until a unit had a port on a slower clock (issue 384).
    let port_edge = |port: &str, tick: usize| -> bool {
        let named = port_clocks.iter().find(|(p, _)| p == port);
        let c = match named {
            Some((_, c)) => c.clone(),
            None => clocks[0].clone(),
        };
        let (first, period) = clock_shape(&trace_name(&c, "clock"));
        tick >= first && (tick - first).is_multiple_of(period)
    };
    // The first rising edge of a port's own clock at or after a tick:
    // the edge a plain input applied before it is sampled at.
    //
    // An input is applied before every tick of the testbench's clock,
    // and a port on a slower clock samples only the last application
    // before its own edge; so what is applied at 2k+1 must be the
    // value the trace holds at that next edge, not at 2k plus the
    // period, which for a port six ticks a cycle is a value from up to
    // four ticks after the edge: an input the run changed between two
    // of the port's edges was then seen one edge early (issue 405).
    // For the default clock the next edge is 2k+2, as before.
    let port_next_edge = |port: &str, tick: usize| -> usize {
        let named = port_clocks.iter().find(|(p, _)| p == port);
        let c = match named {
            Some((_, c)) => c.clone(),
            None => clocks[0].clone(),
        };
        let (first, period) = clock_shape(&trace_name(&c, "clock"));
        if tick <= first {
            first
        } else {
            first + (tick - first).div_ceil(period) * period
        }
    };
    // A channel's handshake, which pulses, as against its data and the
    // plain wires, which are held.
    let pulsed = |port: &str, dir: &str| -> bool {
        (dir == "rxout" || dir == "txout") && !port.ends_with("_data")
    };
    let val = |name: &str, i: usize| -> Option<String> {
        values
            .get(name)
            .and_then(|v| v.get(i))
            .filter(|s| !s.is_empty())
            .cloned()
    };
    // A port the trace knows nothing about is a testbench that drives
    // zeros into the entity and compares it against zeros, which fails
    // from the first cycle the unit does anything and says nothing
    // about why. It happens when a run gives a channel a name of its
    // own: the trace is read by the port's name, so
    // `Wave::add("inp", ..)` is what a port called `inp` wants, and a
    // channel added as something else is not found. Say so here.
    // A pad is the exception: the trace carries nothing on one, and
    // the testbench neither drives nor checks it.
    let missing: Vec<String> = ports
        .iter()
        .filter(|(_, d, _)| !inside(d) && d != "inout")
        .map(|(n, d, _)| trace_name(n, d))
        .filter(|t| !values.contains_key(t))
        .collect();
    if !missing.is_empty() {
        let mut have: Vec<&String> = values.keys().collect();
        have.sort();
        let have: Vec<String> =
            have.iter().take(40).map(|s| (*s).clone()).collect();
        eprintln!(
            "fst2tb: the trace holds nothing for {} of {entity}'s ports: \
             {}\nThe trace is read by the port's name, so a channel \
             must be added to the wave under the name the port has.\n\
             What the trace does hold: {}",
            missing.len(),
            missing.join(", "),
            have.join(", ")
        );
        std::process::exit(1);
    }
    // The frames: one per cycle, the inputs for its edge and the
    // values expected after it, as a string of bits. Each expected
    // value is followed by whether to check it, since the trace may
    // not hold one, and a sender's data is checked only under its
    // valid. An input the trace holds nothing for keeps the value it
    // had, starting from zero. The Verilog testbench starts an empty
    // channel's `ready` high, as its signal starts, which is
    // `ready_high`; the VHDL one's frames start it at zero, as they
    // always have. Both testbenches replay these, the VHDL from
    // packages of literals and the Verilog from a file of vectors.
    let last = times.last().copied().unwrap_or(0) as usize;
    let build_frames = |ready_high: bool| -> Vec<String> {
        let mut frames: Vec<String> = Vec::new();
        let mut last_in: Vec<String> = ports
            .iter()
            .filter(|(n, d, _)| is_in(d) && !clocks.contains(n))
            .map(|(n, d, w)| {
                if ready_high && empty_ready(n, d) {
                    format!("{}1", "0".repeat(*w - 1))
                } else {
                    "0".repeat(*w)
                }
            })
            .collect();
        let mut t = 0usize;
        while t + 2 <= last {
            // At tick 2k+1: the inputs for the next edge, from the trace at
            // 2k+2, or at 2k for a registered input; the registers against
            // the trace at 2k, or 2k+1 for a falling-edge one, which takes
            // its value at that instant; the outputs against 2k+2.
            let mut f = String::new();
            let mut i = 0;
            for (n, d, _) in &ports {
                if is_in(d) && !clocks.contains(n) {
                    let at = if registered(d) {
                        t
                    } else {
                        port_next_edge(n, t + 2)
                    };
                    if let Some(v) = val(&trace_name(n, d), at) {
                        last_in[i] = v;
                    }
                    f.push_str(&last_in[i]);
                    i += 1;
                }
            }
            for (n, d, w) in &ports {
                let at = match d.as_str() {
                    "reg" => Some(t),
                    "regf" => t.checked_sub(1),
                    _ => continue,
                };
                let v = at.and_then(|at| val(&trace_name(n, d), at));
                push_expected(&mut f, *w, v);
            }
            for (n, d, w) in &ports {
                if is_out(d) {
                    let mut v = val(&trace_name(n, d), t + port_period(n));
                    // A handshake between its own clock's edges: the
                    // trace holds nothing for it, so the frame says not
                    // to check rather than checking against a value the
                    // run never had (issue 384).
                    if pulsed(n, d) && !port_edge(n, t + port_period(n)) {
                        v = None;
                    }
                    if d == "txout" && n.ends_with("_data") {
                        let vn = n.replace("_data", "_valid");
                        if !port_edge(&vn, t + port_period(n)) {
                            v = None;
                        } else {
                            let valid = trace_name(&vn, d);
                            if val(&valid, t + port_period(n)).as_deref()
                                != Some("1")
                            {
                                v = None;
                            }
                        }
                    }
                    push_expected(&mut f, *w, v);
                }
            }
            frames.push(f);
            t += 2;
        }
        frames
    };
    // The same testbench in Verilog, for Verilator. The clock's edges
    // sit at 2k + 0.5, so the inputs applied at time zero and at 2k+1
    // come before the edge that reads them, and a tenth of a tick
    // separates applying the inputs from checking the wires.
    if verilog {
        let vlit = |w: usize, bits: &str| format!("{w}'b{bits}");
        let vty = |w: usize| {
            if w == 1 {
                String::new()
            } else {
                format!("[{}:0] ", w - 1)
            }
        };
        let mut o = String::new();
        o.push_str(&format!("`timescale 1ns/1ps\nmodule {entity}_tb;\n"));
        for (n, d, w) in &ports {
            if inside(d) {
                continue;
            }
            if is_in(d) {
                o.push_str(&format!(
                    "  reg {}{n} = {};\n",
                    vty(*w),
                    if empty_ready(n, d) { 1 } else { 0 }
                ));
            } else {
                o.push_str(&format!("  wire {}{n};\n", vty(*w)));
            }
        }
        o.push_str("  integer errors = 0;\n");
        let maps: Vec<String> = ports
            .iter()
            .filter(|p| !inside(&p.1))
            .map(|p| format!(".{0}({0})", p.0))
            .collect();
        o.push_str(&format!("  {module} uut ({});\n", maps.join(", ")));
        // One driver per clock, each at the period and phase its own
        // trace shows. The half tick is the same offset the single
        // clock always had: the edge falls between the tick the inputs
        // are applied at and the tick they are checked at.
        for c in &clocks {
            let (phase, period) = clock_shape(&trace_name(c, "clock"));
            let half = period as f64 / 2.0;
            o.push_str(&format!(
                "  initial begin\n    {c} = 0;\n    #{};\n    \
                 repeat ({}) begin\n      {c} = 1; #{half};\n      \
                 {c} = 0; #{half};\n    end\n  end\n",
                phase as f64 + 0.5,
                last / period + 2
            ));
        }
        // The replay is data, not code: one vector per cycle in a file
        // beside the testbench, read with `$readmemh`, and one loop
        // that applies and checks a vector, a statement per port and
        // not per port per cycle. Written out per cycle, Verilator took
        // 11 GiB and twelve minutes over the SD host's run of two
        // million lines, and the small CI runner stopped it (issue
        // 601); as data, the testbench is the same size whatever the
        // run's length. A vector is a frame with a marker bit in front,
        // so a vector the file does not hold is caught and not replayed
        // as zeros that check nothing.
        let frames = build_frames(true);
        let fw = frames.first().map_or(0, |f| f.len()) + 1;
        let nv = frames.len();
        let mut file = String::new();
        for f in &frames {
            file.push_str(&vhex(&format!("1{f}")));
            file.push('\n');
        }
        std::fs::write(vectors_out, file).expect("write the vectors");
        // The testbench's own names, none of them a port's: a unit
        // with a port called `i` is in the tree.
        let taken: Vec<String> = ports.iter().map(|p| p.0.clone()).collect();
        let [vs, f, i, fd, path] =
            fresh(&["vectors", "f", "i", "fd", "path"], &taken);
        o.push_str(&format!(
            "  reg [{}:0] {vs} [0:{}];\n  reg [{}:0] {f};\n  \
             integer {i};\n  integer {fd};\n  reg [8*512-1:0] {path};\n",
            fw - 1,
            nv.max(1) - 1,
            fw - 1
        ));
        o.push_str("  initial begin\n");
        for (n, d, w) in &ports {
            if d == "in" && !clocks.contains(n) {
                if let Some(v) = val(&trace_name(n, d), 0) {
                    o.push_str(&format!("    {n} = {};\n", vlit(*w, &v)));
                }
            }
        }
        // The file's path from the runfiles, where a test runs, or from
        // `+vectors=` for a run from anywhere else. A file that is not
        // there stops the run, since `$readmemh` only warns.
        o.push_str(&format!(
            "    if (!$value$plusargs(\"vectors=%s\", {path}))\n      \
             {path} = \"{vectors_path}\";\n    \
             {fd} = $fopen({path}, \"r\");\n    \
             if ({fd} == 0) $fatal(1, \"no vectors at %0s\", {path});\n    \
             $fclose({fd});\n    $readmemh({path}, {vs});\n    \
             for ({i} = 0; {i} < {nv}; {i} = {i} + 1) begin\n      #1;\n      \
             {f} = {vs}[{i}];\n      if ({f}[{}] !== 1'b1)\n        \
             $fatal(1, \"vector %0d is missing from %0s\", {i}, {path});\n",
            fw - 1
        ));
        // A field's bits in the vector, from its offset in the frame,
        // the frame's first bit being the vector's highest but one.
        let mut off = 0usize;
        let mut field = |w: usize| -> String {
            let hi = fw - 2 - off;
            off += w;
            if w == 1 {
                format!("{f}[{hi}]")
            } else {
                format!("{f}[{hi}:{}]", hi + 1 - w)
            }
        };
        for (n, d, w) in &ports {
            if is_in(d) && !clocks.contains(n) {
                o.push_str(&format!("      {n} = {};\n", field(*w)));
            }
        }
        o.push_str("      #0.1;\n");
        // A rising-edge register took its value at 2k; a falling-edge
        // one at the falling edge before, 2k-1 in the trace, since here
        // the clock falls at 2k+1.5. Which, and whether at all, is the
        // frame's flag; the tick, the signal and both values are the
        // message.
        let check =
            |o: &mut String, what: &str, sig: &str, e: &str, c: &str| {
                o.push_str(&format!(
                    "      if ({c} && {sig} !== {e}) begin\n        \
                 $display(\"{what} differs at tick %0d: expected %0h, \
                 got %0h\", \
                 $time, {e}, {sig});\n        errors = errors + 1;\n      \
                 end\n"
                ));
            };
        for (n, d, w) in &ports {
            if is_reg(d) {
                let e = field(*w);
                let c = field(1);
                check(&mut o, n, &format!("uut.{n}"), &e, &c);
            }
        }
        for (n, d, w) in &ports {
            if is_out(d) {
                let e = field(*w);
                let c = field(1);
                let sig = if d == "wire" {
                    format!("uut.{n}")
                } else {
                    n.clone()
                };
                check(&mut o, n, &sig, &e, &c);
            }
        }
        debug_assert_eq!(off + 1, fw);
        o.push_str(
            "      #0.9;\n    end\n    #1;\n    if (errors == 0) \
             $display(\"the lowering agrees with the trace\");\n    \
             else $fatal(1, \"the lowering differs from the trace\");\n    \
             $finish;\n  end\nendmodule\n",
        );
        print!("{o}");
        return;
    }
    let names: Vec<String> = ports
        .iter()
        .filter(|(_, d, _)| inside(d))
        .map(|(n, _, _)| n.clone())
        .collect();
    let declared: Vec<String> = ports
        .iter()
        .filter(|(_, d, _)| !inside(d))
        .map(|(n, _, _)| n.clone())
        .collect();
    let alias = aliases(&names, &declared);
    let mut o = String::new();
    o.push_str(
        "library ieee;\nuse ieee.std_logic_1164.all;\n\
         use ieee.numeric_std.all;\n\n",
    );
    o.push_str(&format!(
        "entity {entity}_tb is\nend entity;\n\n\
         architecture sim of {entity}_tb is\n"
    ));
    for (n, d, w) in &ports {
        // A pad is left undriven here and never checked: the trace
        // carries nothing on it, and the module inside drives it.
        if d == "inout" {
            let t = if *w == 1 {
                "std_logic".to_string()
            } else {
                format!("std_logic_vector({} downto 0)", w - 1)
            };
            o.push_str(&format!("  signal {n} : {t};\n"));
        } else if !inside(d) {
            let init = if empty_ready(n, d) {
                "'1'"
            } else if *w == 1 {
                "'0'"
            } else {
                "(others => '0')"
            };
            o.push_str(&format!("  signal {n} : {} := {init};\n", ty(*w)));
        }
    }
    let _ = is_out;
    o.push_str(&format!(
        "  signal errors : natural := 0;\nbegin\n\
           uut : entity work.{module} port map ("
    ));
    let maps: Vec<String> = ports
        .iter()
        .filter(|p| !inside(&p.1))
        .map(|p| format!("{0} => {0}", p.0))
        .collect();
    o.push_str(&format!("{});\n\n", maps.join(", ")));
    // One process per clock, each at the period and phase its own
    // trace shows, one tick per nanosecond. The first edge is at the
    // clock's phase, where the inputs for it are applied too: a few
    // deltas first, so the wires that follow those inputs have settled
    // when the edge comes.
    for (k, c) in clocks.iter().enumerate() {
        let (phase, period) = clock_shape(&trace_name(c, "clock"));
        let half = period as f64 / 2.0;
        o.push_str(&format!(
            "  -- {c}: rising every {period} ticks from tick {phase}.\n\
             clock{k} : process\n  begin\n"
        ));
        let wait = if phase == 0 {
            String::new()
        } else {
            format!("    wait for {phase} ns;\n")
        };
        o.push_str(&format!(
            "    for i in 1 to 8 loop wait for 0 ns; end loop;\n{wait}\
             for i in 0 to {} loop\n      {c} <= '1'; wait for {half} ns;\n\
             {c} <= '0'; wait for {half} ns;\n    end loop;\n    wait;\n\
             end process;\n\n",
            last / period + 1
        ));
    }
    o.push_str("  -- The trace, replayed and checked.\n  check : process\n");
    o.push_str(
        "    procedure expect(what : string; ok : boolean; at : time) is\n\
         begin\n      if not ok then\n\
         report what & \" differs at \" & time'image(at) severity error;\n\
         errors <= errors + 1;\n      end if;\n    end procedure;\n",
    );
    // A register is reached by its external name, declared once as an
    // alias: a name per check would be most of the unit, and nvc's
    // heap for one unit is finite.
    for (n, d, w) in &ports {
        if inside(d) {
            o.push_str(&format!(
                "    alias {} is << signal .{entity}_tb.uut.{n} : {} >>;\n",
                alias[n],
                ty(*w)
            ));
        }
    }
    // Inputs for the edge at tick 0 are applied before any wait; a
    // registered input starts as the channel does, empty.
    let mut first = String::new();
    for (n, d, w) in &ports {
        if d == "in" && !clocks.contains(n) {
            if let Some(v) = val(&trace_name(n, d), 0) {
                first.push_str(&format!("    {n} <= {};\n", lit(*w, &v)));
            }
        }
    }
    // One cycle is one frame: a string of the inputs and the expected
    // values as bits, each expected value followed by whether to check
    // it, since the trace may not hold one, and a sender's data is
    // checked only under its valid. The frames are one constant array
    // of string literals, and one procedure replays a frame, reading
    // its fields at fixed offsets. A statement per check, or a call
    // with eighty arguments per cycle, grew past nvc's heap for one
    // unit once a run passed a few hundred cycles; an array of
    // literals is data, and the unit stays small whatever the run's
    // length.
    let mut width = 0usize;
    let mut take = |w: usize| -> (usize, usize) {
        let r = (width + 1, width + w);
        width += w;
        r
    };
    let conv = |w: usize, (a, b): (usize, usize)| {
        if w == 1 {
            format!("bit1(f({a}))")
        } else {
            format!("bits(f({a} to {b}))")
        }
    };
    // The tick sits just before the odd tick: the inputs for the next
    // rising edge are applied, every wire settles however deep, and
    // the checks come before the falling edge at the odd tick itself,
    // as the Verilog testbench's do.
    let mut tick = String::from("      wait for 900 ps;\n");
    for (n, d, w) in &ports {
        if is_in(d) && !clocks.contains(n) {
            let v = take(*w);
            tick.push_str(&format!("      {n} <= {};\n", conv(*w, v)));
        }
    }
    tick.push_str("      wait for 50 ps;\n");
    for (n, d, w) in &ports {
        if is_reg(d) {
            let v = take(*w);
            let (c, _) = take(1);
            tick.push_str(&format!(
                "      if f({c}) = '1' then expect(\"{n}\", {} = {}, \
                 now); end if;\n",
                alias[n],
                conv(*w, v)
            ));
        }
    }
    for (n, d, w) in &ports {
        if is_out(d) {
            let v = take(*w);
            let (c, _) = take(1);
            let sig = if d == "wire" {
                alias[n].clone()
            } else {
                n.clone()
            };
            tick.push_str(&format!(
                "      if f({c}) = '1' then expect(\"{n}\", {sig} = {}, \
                 now); end if;\n",
                conv(*w, v)
            ));
        }
    }
    tick.push_str("      wait for 1050 ps;\n");
    // The closure borrows `width`, and this is where the borrow ends;
    // the closure itself has nothing to drop.
    #[allow(clippy::drop_non_drop)]
    drop(take);
    o.push_str(
        "    function bits(s : string) return unsigned is\n\
         variable v : unsigned(s'length - 1 downto 0);\n    begin\n\
         for i in 0 to s'length - 1 loop\n\
         if s(s'left + i) = '1' then v(v'left - i) := '1';\n\
         else v(v'left - i) := '0'; end if;\n\
         end loop;\n      return v;\n    end function;\n\
         function bit1(c : character) return std_logic is\n    begin\n\
         if c = '1' then return '1'; else return '0'; end if;\n\
         end function;\n\
         function unhex(h : string) return string is\n\
         variable s : string(1 to 4 * h'length);\n\
         variable v : integer;\n    begin\n\
         for i in 0 to h'length - 1 loop\n\
         case h(h'left + i) is\n\
         when '0' => v := 0; when '1' => v := 1; when '2' => v := 2;\n\
         when '3' => v := 3; when '4' => v := 4; when '5' => v := 5;\n\
         when '6' => v := 6; when '7' => v := 7; when '8' => v := 8;\n\
         when '9' => v := 9; when 'a' => v := 10; when 'b' => v := 11;\n\
         when 'c' => v := 12; when 'd' => v := 13; when 'e' => v := 14;\n\
         when others => v := 15;\n        end case;\n\
         for k in 0 to 3 loop\n\
         if (v / 2 ** (3 - k)) mod 2 = 1 then s(4 * i + k + 1) := '1';\n\
         else s(4 * i + k + 1) := '0'; end if;\n\
         end loop;\n      end loop;\n      return s;\n    end function;\n",
    );
    o.push_str(&format!(
        "    procedure tb_tick(f : string) is\n    begin\n{tick}    \
         end procedure;\n"
    ));
    let frames = build_frames(false);
    debug_assert!(frames.iter().all(|f| f.len() == width));
    // The frames, as hex, in packages of a few hundred before the
    // entity: one package is one design unit under nvc's heap limit.
    let hex = |f: &str| -> String {
        let mut f = f.to_string();
        while !f.len().is_multiple_of(4) {
            f.push('0');
        }
        f.as_bytes()
            .chunks(4)
            .map(|c| {
                let v = c.iter().fold(0u8, |v, &b| v << 1 | (b == b'1') as u8);
                char::from_digit(v as u32, 16).unwrap()
            })
            .collect()
    };
    let hwidth = width.div_ceil(4);
    let mut packages = String::new();
    let chunks: Vec<&[String]> = frames.chunks(256).collect();
    for (c, chunk) in chunks.iter().enumerate() {
        packages.push_str(&format!(
            "package {entity}_tb_frames{c} is\n  type frame_array is array \
             (natural range <>) of string(1 to {hwidth});\n  constant \
             frames : frame_array := (\n"
        ));
        for (i, f) in chunk.iter().enumerate() {
            let sep = if i + 1 < chunk.len() { "," } else { "" };
            packages.push_str(&format!("    {i} => \"{}\"{sep}\n", hex(f)));
        }
        packages.push_str("  );\nend package;\n\n");
    }
    o.push_str("  begin\n");
    o.push_str(&first);
    for c in 0..chunks.len() {
        o.push_str(&format!(
            "    for i in work.{entity}_tb_frames{c}.frames'range loop\n      \
             tb_tick(unhex(work.{entity}_tb_frames{c}.frames(i)));\n    \
             end loop;\n"
        ));
    }
    o.push_str(
        "    wait for 1 ns;\n    if errors = 0 then\n\
         report \"the lowering agrees with the trace\";\n    else\n\
         report \"the lowering differs from the trace\" severity failure;\n\
         end if;\n    std.env.finish;\n  end process;\nend architecture;\n",
    );
    print!("{packages}{o}");
}

/// A clock's first rising tick and the ticks between its first two
/// rising edges, from its value at every tick; the default clock's
/// shape, rising at zero every two ticks, for a clock the trace does
/// not hold. Only the first two rising edges after tick 0 are looked
/// for, so the search ends as soon as it has them (issue 629).
fn find_clock_shape(v: Option<&Vec<String>>) -> (usize, usize) {
    let Some(v) = v else {
        return (0, 2);
    };
    let mut high: Vec<usize> = Vec::with_capacity(2);
    for i in 1..v.len() {
        if v[i].ends_with('1') && !v[i - 1].ends_with('1') {
            high.push(i);
            if high.len() == 2 {
                break;
            }
        }
    }
    let first = if v.first().is_some_and(|b| b.ends_with('1')) {
        0
    } else {
        *high.first().unwrap_or(&0)
    };
    let period = match (high.first(), high.get(1)) {
        (Some(a), Some(b)) => b - a,
        _ => 2,
    };
    (first, period.max(1))
}

#[cfg(test)]
mod tests {
    use super::aliases;

    use super::vhex;

    use super::fresh;

    /// The case of `ex_valif`, whose unit has a port called `i`: the
    /// loop's counter takes another name, and so does any name a port
    /// already has.
    #[test]
    fn a_testbench_name_is_never_a_port() {
        let [f, i, fd] = fresh(&["f", "i", "fd"], &names(&["i", "tb_i"]));
        assert_eq!(f, "f");
        assert_eq!(i, "tb_tb_i");
        assert_eq!(fd, "fd");
    }

    /// A vector is padded on the left, so that its first bit is the
    /// highest of the word `$readmemh` reads, and a field's place in
    /// it counts down from there.
    #[test]
    fn a_vector_is_padded_on_the_left() {
        assert_eq!(vhex("1"), "1");
        assert_eq!(vhex("10000"), "10");
        assert_eq!(vhex("110100101"), "1a5");
        assert_eq!(vhex("11111111"), "ff");
    }

    use super::collisions;

    fn read(v: &[(&str, usize)]) -> Vec<(String, Vec<String>)> {
        v.iter()
            .map(|(n, w)| (n.to_string(), vec!["0".repeat(*w)]))
            .collect()
    }

    /// The case of issue 462, as `ex_ethdma` met it: `FrameOut` has
    /// `out: Tx<EthByte>` at 9 bits and `FrameIn` has `out: Tx<U<32>>`
    /// at 32, both recorded, and the second testbench took the first
    /// signal.
    #[test]
    fn a_name_carried_twice_is_reported_with_both_widths() {
        let r = read(&[("clk", 1), ("out", 9), ("out", 32)]);
        let c = collisions(&r);
        assert_eq!(c.len(), 1, "one name is carried twice");
        assert_eq!(c[0].0, "out");
        assert_eq!(c[0].1, vec![9, 32], "both widths, in trace order");
    }

    /// The silent half, and the reason this is a collision check and
    /// not a width check. That run had three collisions and nvc
    /// reported one, because only `out` had widths that differed;
    /// `bytes` and `go` matched, so the testbench compared the wrong
    /// unit's signal and said nothing.
    #[test]
    fn a_collision_is_reported_even_when_the_widths_agree() {
        let r = read(&[
            ("clk", 1),
            ("out", 9),
            ("bytes", 16),
            ("go", 1),
            ("out", 32),
            ("bytes", 16),
            ("go", 1),
        ]);
        let c = collisions(&r);
        let names: Vec<&str> = c.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["out", "bytes", "go"],
            "all three, and not only the one whose widths differ"
        );
        assert_eq!(c[1].1, vec![16, 16], "`bytes` agreed and still counts");
        assert_eq!(c[2].1, vec![1, 1], "so did `go`");
    }

    /// A trace with no repeated name is not a fault, and the common
    /// case must not be made to look like one.
    #[test]
    fn distinct_names_are_not_a_collision() {
        let r = read(&[("clk", 1), ("inp", 8), ("out", 8), ("go", 1)]);
        assert!(collisions(&r).is_empty());
    }

    /// A signal that never changed has no recorded value to measure,
    /// and it is still a collision. Reporting its width as zero is
    /// better than dropping it: the name is the fault.
    #[test]
    fn a_signal_that_never_changed_still_collides() {
        let r = vec![
            ("go".to_string(), vec![String::new(), String::new()]),
            ("go".to_string(), vec![String::new(), "1".to_string()]),
        ];
        let c = collisions(&r);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].1, vec![0, 1]);
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// The case of issue 211: a register called `data` and an
    /// AXI-Lite channel port called `r`, whose data wire the netlist
    /// calls `r_data`.
    #[test]
    fn an_alias_is_never_a_declared_signal() {
        let regs = names(&["data", "state", "errors"]);
        let ports = names(&["clk", "r_data", "r_valid", "r_ready"]);
        let a = aliases(&regs, &ports);
        for r in &regs {
            assert!(!ports.contains(&a[r]), "{} took a port's name", a[r]);
            assert_ne!(a[r], "errors");
        }
        assert_eq!(a["data"], "reg_data");
        assert_eq!(a["state"], "reg_state");
    }

    /// A port that takes the prefixed name too, which nothing in the
    /// tree has, and the alias lengthens rather than colliding.
    #[test]
    fn an_alias_lengthens_past_a_port() {
        let a = aliases(&names(&["data"]), &names(&["reg_data"]));
        assert_eq!(a["data"], "reg_reg_data");
    }

    /// Two registers never share an alias, however they are named.
    #[test]
    fn aliases_are_distinct() {
        let regs = names(&["data", "reg_data", "reg_reg_data"]);
        let a = aliases(&regs, &names(&["r_data"]));
        let mut all: Vec<&String> = regs.iter().map(|r| &a[r]).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), regs.len());
    }
}
