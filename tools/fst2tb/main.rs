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
//! assignment shows the same value.
//!
//! Usage: fst2tb FILE.fst FILE.vhd.ports ENTITY UNIT > tb.vhd
//! where UNIT is the name the Rust testbench gave the unit in the trace.
use std::collections::BTreeMap;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (fst, ports, entity, unit) = (&a[1], &a[2], &a[3], &a[4]);
    let ports: Vec<(String, String, usize)> = std::fs::read_to_string(ports)
        .expect("ports")
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() == 3).then(|| {
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
    let trace_name = |port: &str, dir: &str| -> String {
        if dir == "reg" {
            format!("{unit}.{port}")
        } else {
            port.to_string()
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
        if d != "reg" {
            let init = if *w == 1 { "'0'" } else { "(others => '0')" };
            o.push_str(&format!("  signal {n} : {} := {init};\n", ty(*w)));
        }
    }
    o.push_str(&format!(
        "  signal errors : natural := 0;\nbegin\n\
           uut : entity work.{entity} port map ("
    ));
    let maps: Vec<String> = ports
        .iter()
        .filter(|p| p.1 != "reg")
        .map(|p| format!("{0} => {0}", p.0))
        .collect();
    o.push_str(&format!("{});\n\n", maps.join(", ")));
    let last = times.last().copied().unwrap_or(0) as usize;
    o.push_str(
        "  -- The clock: rising at even ticks, one tick per nanosecond.\n\
           clock : process\n  begin\n",
    );
    o.push_str(&format!(
        "    for i in 0 to {} loop\n      {clock} <= '1'; wait for 1 ns;\n\
         {clock} <= '0'; wait for 1 ns;\n    end loop;\n    wait;\n\
         end process;\n\n",
        last / 2 + 1
    ));
    o.push_str("  -- The trace, replayed and checked.\n  check : process\n");
    o.push_str(
        "    procedure expect(what : string; ok : boolean; at : time) is\n\
         begin\n      if not ok then\n\
         report what & \" differs at \" & time'image(at) severity error;\n\
         errors <= errors + 1;\n      end if;\n    end procedure;\n  begin\n",
    );
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
    // Inputs for the edge at tick 0 are applied before any wait.
    for (n, d, w) in &ports {
        if d == "in" && *n != clock {
            if let Some(v) = val(&trace_name(n, d), 0) {
                o.push_str(&format!("    {n} <= {};\n", lit(*w, &v)));
            }
        }
    }
    let mut t = 0usize;
    while t + 2 <= last {
        // At tick 2k+1: check registers against the trace at 2k, and
        // outputs and inputs for the next edge against the trace at 2k+2.
        o.push_str("    wait for 1 ns;\n");
        let odd = t + 1;
        for (n, d, w) in &ports {
            if d == "reg" {
                if let Some(v) = val(&trace_name(n, d), t) {
                    o.push_str(&format!(
                        "    expect(\"{n}\", << signal .{entity}_tb.uut.{n} : {} >> \
                         = {}, now);\n",
                        ty(*w),
                        lit(*w, &v)
                    ));
                }
            }
        }
        for (n, d, w) in &ports {
            if d == "out" {
                if let Some(v) = val(&trace_name(n, d), t + 2) {
                    o.push_str(&format!(
                        "    expect(\"{n}\", {n} = {}, now);\n",
                        lit(*w, &v)
                    ));
                }
            }
        }
        for (n, d, w) in &ports {
            if d == "in" && *n != clock {
                if let Some(v) = val(&trace_name(n, d), t + 2) {
                    o.push_str(&format!("    {n} <= {};\n", lit(*w, &v)));
                }
            }
        }
        o.push_str("    wait for 1 ns;\n");
        let _ = odd;
        t += 2;
    }
    o.push_str(
        "    wait for 1 ns;\n    if errors = 0 then\n\
         report \"the lowering agrees with the trace\";\n    else\n\
         report \"the lowering differs from the trace\" severity failure;\n\
         end if;\n    std.env.finish;\n  end process;\nend architecture;\n",
    );
    print!("{o}");
}
