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
//! Form: in VHDL, one procedure holds a tick; a cycle's inputs and
//! expected values are one string of bits, a frame, kept as hex, and
//! the replay is a loop over constant arrays of frames, each array a
//! package of its own of a few hundred frames. nvc's heap for one
//! design unit is finite, and a statement per check, or a call per
//! cycle with an argument per signal, is more than it holds for a run
//! of a few hundred cycles; a literal is data, but a unit of half a
//! megabyte of them is over the limit too, so they are spread.
//!
//! Usage: fst2tb FILE.fst FILE.vhd.ports ENTITY UNIT [--verilog] > tb
//! where UNIT is the name the Rust testbench gave the unit in the trace;
//! `--verilog` writes the same testbench in Verilog, for Verilator.
use std::collections::BTreeMap;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (fst, ports, entity, unit) = (&a[1], &a[2], &a[3], &a[4]);
    let verilog = a.get(5).map(|s| s == "--verilog").unwrap_or(false);
    // The ports file may hold several entities, each section opened by
    // `entity NAME`; the one asked for is taken, or everything when the
    // file has no sections.
    let mut section: Option<String> = None;
    // A fourth field names the trace scope a port is found under, when
    // it is not the port's own name: a channel two units share under
    // one name in the run, with a port name of their own on each side.
    let mut scopes: Vec<(String, String)> = Vec::new();
    let ports: Vec<(String, String, usize)> = std::fs::read_to_string(ports)
        .expect("ports")
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() == 2 && f[0] == "entity" {
                section = Some(f[1].to_string());
                return None;
            }
            if section.as_deref().is_some_and(|s| s != entity) {
                return None;
            }
            // A memory's line has a depth as its fourth field and is not
            // a port; a port's fourth field is its scope.
            if f.len() == 4 && f[1] == "mem" {
                return None;
            }
            if f.len() == 4 {
                scopes.push((f[0].to_string(), f[3].to_string()));
            }
            (f.len() == 3 || f.len() == 4).then(|| {
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
    let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
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
        values.insert(name.clone(), per_tick);
    }
    let clock = ports
        .iter()
        .find(|p| p.1 == "in" && p.0 == "clk")
        .map(|p| p.0.clone())
        .unwrap_or("clk".into());
    // Directions: in, out, reg, and a channel's rxin, rxout, txin,
    // txout. A channel's wire is traced under the channel by its side.
    let is_in = |d: &str| d == "in" || d == "rxin" || d == "txin";
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
        if inside(dir) {
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
    let val = |name: &str, i: usize| -> Option<String> {
        values
            .get(name)
            .and_then(|v| v.get(i))
            .filter(|s| !s.is_empty())
            .cloned()
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
                o.push_str(&format!("  reg {}{n} = 0;\n", vty(*w)));
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
        o.push_str(&format!("  {entity} uut ({});\n", maps.join(", ")));
        let last = times.last().copied().unwrap_or(0) as usize;
        o.push_str(&format!(
            "  initial begin\n    #0.5;\n    repeat ({}) begin\n      \
             {clock} = 1; #1;\n      {clock} = 0; #1;\n    end\n  end\n",
            last / 2 + 2
        ));
        o.push_str("  initial begin\n");
        for (n, d, w) in &ports {
            if d == "in" && *n != clock {
                if let Some(v) = val(&trace_name(n, d), 0) {
                    o.push_str(&format!("    {n} = {};\n", vlit(*w, &v)));
                }
            }
        }
        let check =
            |o: &mut String, what: &str, sig: &str, w: usize, v: &str| {
                o.push_str(&format!(
                    "    if ({sig} !== {}) begin $display(\"{what} differs \
                 at %0t\", $time); errors = errors + 1; end\n",
                    vlit(w, v)
                ));
            };
        let mut t = 0usize;
        while t + 2 <= last {
            o.push_str("    #1;\n");
            for (n, d, w) in &ports {
                if is_in(d) && *n != clock {
                    let at = if registered(d) { t } else { t + 2 };
                    if let Some(v) = val(&trace_name(n, d), at) {
                        o.push_str(&format!("    {n} = {};\n", vlit(*w, &v)));
                    }
                }
            }
            o.push_str("    #0.1;\n");
            // A rising-edge register took its value at 2k; a
            // falling-edge one at the falling edge before, 2k-1 in the
            // trace, since here the clock falls at 2k+1.5.
            for (n, d, w) in &ports {
                let at = match d.as_str() {
                    "reg" => t,
                    "regf" if t >= 1 => t - 1,
                    _ => continue,
                };
                if let Some(v) = val(&trace_name(n, d), at) {
                    check(&mut o, n, &format!("uut.{n}"), *w, &v);
                }
            }
            for (n, d, w) in &ports {
                if is_out(d) {
                    if d == "txout" && n.ends_with("_data") {
                        let valid =
                            trace_name(&n.replace("_data", "_valid"), d);
                        if val(&valid, t + 2).as_deref() != Some("1") {
                            continue;
                        }
                    }
                    if let Some(v) = val(&trace_name(n, d), t + 2) {
                        let sig = if d == "wire" {
                            format!("uut.{n}")
                        } else {
                            n.clone()
                        };
                        check(&mut o, n, &sig, *w, &v);
                    }
                }
            }
            o.push_str("    #0.9;\n");
            t += 2;
        }
        o.push_str(
            "    #1;\n    if (errors == 0) \
             $display(\"the lowering agrees with the trace\");\n    \
             else $fatal(1, \"the lowering differs from the trace\");\n    \
             $finish;\n  end\nendmodule\n",
        );
        print!("{o}");
        return;
    }
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
        if !inside(d) {
            let init = if *w == 1 { "'0'" } else { "(others => '0')" };
            o.push_str(&format!("  signal {n} : {} := {init};\n", ty(*w)));
        }
    }
    let _ = is_out;
    o.push_str(&format!(
        "  signal errors : natural := 0;\nbegin\n\
           uut : entity work.{entity} port map ("
    ));
    let maps: Vec<String> = ports
        .iter()
        .filter(|p| !inside(&p.1))
        .map(|p| format!("{0} => {0}", p.0))
        .collect();
    o.push_str(&format!("{});\n\n", maps.join(", ")));
    let last = times.last().copied().unwrap_or(0) as usize;
    o.push_str(
        "  -- The clock: rising at even ticks, one tick per nanosecond.\n\
           clock : process\n  begin\n",
    );
    // The first edge is at time zero, where the inputs for it are
    // applied too: a few deltas first, so the wires that follow those
    // inputs have settled when the edge comes.
    o.push_str(&format!(
        "    for i in 1 to 8 loop wait for 0 ns; end loop;\n\
         for i in 0 to {} loop\n      {clock} <= '1'; wait for 1 ns;\n\
         {clock} <= '0'; wait for 1 ns;\n    end loop;\n    wait;\n\
         end process;\n\n",
        last / 2 + 1
    ));
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
                "    alias r_{n} is << signal .{entity}_tb.uut.{n} : {} >>;\n",
                ty(*w)
            ));
        }
    }
    // Inputs for the edge at tick 0 are applied before any wait; a
    // registered input starts as the channel does, empty.
    let mut first = String::new();
    for (n, d, w) in &ports {
        if d == "in" && *n != clock {
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
        if is_in(d) && *n != clock {
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
                "      if f({c}) = '1' then expect(\"{n}\", r_{n} = {}, \
                 now); end if;\n",
                conv(*w, v)
            ));
        }
    }
    for (n, d, w) in &ports {
        if is_out(d) {
            let v = take(*w);
            let (c, _) = take(1);
            let sig = if d == "wire" {
                format!("r_{n}")
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
    let mut frames: Vec<String> = Vec::new();
    let mut last_in: Vec<String> = ports
        .iter()
        .filter(|(n, d, _)| is_in(d) && *n != clock)
        .map(|(_, _, w)| "0".repeat(*w))
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
            if is_in(d) && *n != clock {
                let at = if registered(d) { t } else { t + 2 };
                if let Some(v) = val(&trace_name(n, d), at) {
                    last_in[i] = v;
                }
                f.push_str(&last_in[i]);
                i += 1;
            }
        }
        let expected = |f: &mut String, w: usize, v: Option<String>| match v {
            Some(v) => {
                f.push_str(&v);
                f.push('1');
            }
            None => {
                f.push_str(&"0".repeat(w));
                f.push('0');
            }
        };
        for (n, d, w) in &ports {
            let at = match d.as_str() {
                "reg" => Some(t),
                "regf" => t.checked_sub(1),
                _ => continue,
            };
            let v = at.and_then(|at| val(&trace_name(n, d), at));
            expected(&mut f, *w, v);
        }
        for (n, d, w) in &ports {
            if is_out(d) {
                let mut v = val(&trace_name(n, d), t + 2);
                if d == "txout" && n.ends_with("_data") {
                    let valid = trace_name(&n.replace("_data", "_valid"), d);
                    if val(&valid, t + 2).as_deref() != Some("1") {
                        v = None;
                    }
                }
                expected(&mut f, *w, v);
            }
        }
        debug_assert_eq!(f.len(), width);
        frames.push(f);
        t += 2;
    }
    // The frames, as hex, in packages of a few hundred before the
    // entity: one package is one design unit under nvc's heap limit.
    let hex = |f: &str| -> String {
        let mut f = f.to_string();
        while f.len() % 4 != 0 {
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
