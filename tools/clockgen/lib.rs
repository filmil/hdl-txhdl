// SPDX-License-Identifier: Apache-2.0
//! Clocks from a clock (issue 304): given the input clock's frequency and
//! the frequencies wanted, the settings of a 7-series clock manager that
//! make them, and the Verilog that instantiates it.
//!
//! [`plan`] searches every legal setting of the primitive, the input
//! divider `D`, the feedback multiplier `M` and an output divider `O`
//! for each output, keeping the phase detector, `input / D`, and the
//! oscillator, `input * M / D`, inside the part's ranges, which
//! [`Limits`] holds. Of the settings that meet every output within its
//! tolerance, it takes the one whose worst output is nearest, then the
//! highest oscillator, which UG472 gives as the least jitter, then the
//! smallest `D`. [`verilog`] writes the module.
//!
//! The MMCM's feedback multiplier and its first output's divider take
//! eighths (`CLKFBOUT_MULT_F`, `CLKOUT0_DIVIDE_F`); every other setting
//! is a whole number. Every setting is held here in eighths, so the two
//! kinds are searched alike.

use std::fmt::Write as _;

/// Which clock manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    /// `MMCME2_BASE`: seven outputs, fractional feedback and first output.
    Mmcm,
    /// `PLLE2_BASE`: six outputs, whole numbers only.
    Pll,
}

/// A range of frequencies, in hertz, both ends allowed.
#[derive(Clone, Copy, Debug)]
pub struct Range {
    pub min: f64,
    pub max: f64,
}

impl Range {
    fn holds(&self, f: f64) -> bool {
        f >= self.min && f <= self.max
    }
}

/// What a primitive allows on one part: the input, the phase detector,
/// the oscillator and an output, and its dividers' ranges.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub input: Range,
    pub pfd: Range,
    pub vco: Range,
    pub output: Range,
    /// The input divider `D`'s largest.
    pub d_max: u32,
    /// How many outputs.
    pub outputs: usize,
}

const MHZ: f64 = 1e6;

impl Limits {
    /// The limits on the flagship's part, xc7a200tfbg484-2: Artix-7, speed
    /// grade -2, from DS181's MMCM and PLL switching characteristics, and
    /// each checked against Vivado 2025.2 where Vivado says it:
    ///
    /// * the oscillator's range by the bitstream's DRC, PDRC-34 for the
    ///   MMCM and PDRC-43 for the PLL, which state 600 to 1440 MHz and 800
    ///   to 1866 MHz and refuse a setting just outside them
    ///   (`//tools/clockgen:drc_test`);
    /// * the input's and an output's ranges by the Clocking Wizard, which
    ///   refuses an input just outside 10 to 800 MHz (MMCM) or 19 to 800
    ///   MHz (PLL), and states an output's as 4.687 to 800 MHz (MMCM) and
    ///   6.250 to 800 MHz (PLL);
    /// * the phase detector's range by neither: no DRC and no wizard check
    ///   refuses one outside it, so it rests on DS181 alone. With the
    ///   board's 200 MHz input it cannot bind, since `input / D` is never
    ///   above 200 MHz.
    pub fn artix7_2(p: Primitive) -> Limits {
        match p {
            Primitive::Mmcm => Limits {
                input: Range {
                    min: 10.0 * MHZ,
                    max: 800.0 * MHZ,
                },
                pfd: Range {
                    min: 10.0 * MHZ,
                    max: 500.0 * MHZ,
                },
                vco: Range {
                    min: 600.0 * MHZ,
                    max: 1440.0 * MHZ,
                },
                output: Range {
                    min: 4.687 * MHZ,
                    max: 800.0 * MHZ,
                },
                d_max: 106,
                outputs: 7,
            },
            Primitive::Pll => Limits {
                input: Range {
                    min: 19.0 * MHZ,
                    max: 800.0 * MHZ,
                },
                pfd: Range {
                    min: 19.0 * MHZ,
                    max: 500.0 * MHZ,
                },
                vco: Range {
                    min: 800.0 * MHZ,
                    max: 1866.0 * MHZ,
                },
                output: Range {
                    min: 6.25 * MHZ,
                    max: 800.0 * MHZ,
                },
                d_max: 56,
                outputs: 6,
            },
        }
    }
}

/// An output wanted: its frequency in hertz, and how far from it is
/// still right, in parts per million; nought asks for it exactly.
#[derive(Clone, Copy, Debug)]
pub struct Want {
    pub hz: f64,
    pub ppm: f64,
}

/// An output as planned: its divider in eighths, and the frequency it
/// gets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Out {
    pub o8: u32,
    pub hz: f64,
}

impl Out {
    /// How far from `want` this is, in parts per million.
    pub fn ppm(&self, want: &Want) -> f64 {
        (self.hz - want.hz).abs() / want.hz * 1e6
    }
}

/// A clock manager's settings that make the outputs wanted.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub primitive: Primitive,
    pub input: f64,
    /// The input divider, `DIVCLK_DIVIDE`.
    pub d: u32,
    /// The feedback multiplier in eighths: `CLKFBOUT_MULT_F` times 8.
    pub m8: u32,
    pub pfd: f64,
    pub vco: f64,
    pub outs: Vec<Out>,
}

/// Why no plan: the input is outside the part's range, more outputs are
/// asked for than the primitive has, or no setting meets every output.
#[derive(Debug, PartialEq)]
pub enum Refusal {
    Input(f64),
    TooMany {
        asked: usize,
        has: usize,
    },
    /// The nearest setting found, and for each output how far it was,
    /// in parts per million, against what it allowed.
    Unmet(Vec<(f64, f64)>),
}

/// The divider steps an output allows, in eighths: the MMCM's first
/// output from 2.000 to 128.000 in eighths, every other from 1 to 128.
fn steps(p: Primitive, k: usize) -> (u32, u32, u32) {
    if p == Primitive::Mmcm && k == 0 {
        (16, 1024, 1)
    } else {
        (8, 1024, 8)
    }
}

/// The best divider for an output of the oscillator `input * m8 / 8 / d`:
/// the steps either side of the ideal, of those whose frequency the part
/// allows. Each frequency is worked out in one division,
/// `input * m8 / (d * o8)`, so a whole one comes out whole.
fn best_out(
    l: &Limits,
    p: Primitive,
    k: usize,
    (input, d, m8): (f64, u32, u32),
    w: &Want,
) -> Option<Out> {
    let (lo, hi, step) = steps(p, k);
    let ideal = input * m8 as f64 / (d as f64 * w.hz);
    let near = ((ideal / step as f64).floor() as i64) * step as i64;
    let mut best: Option<Out> = None;
    for c in [
        near - step as i64,
        near,
        near + step as i64,
        near + 2 * step as i64,
    ] {
        if c < lo as i64 || c > hi as i64 {
            continue;
        }
        let o8 = c as u32;
        let hz = input * m8 as f64 / (d as f64 * o8 as f64);
        if !l.output.holds(hz) {
            continue;
        }
        let out = Out { o8, hz };
        if best.is_none_or(|b| out.ppm(w) < b.ppm(w)) {
            best = Some(out);
        }
    }
    best
}

/// How badly a setting misses: its worst output's error over that
/// output's tolerance, so that one or less meets every output. An exact
/// request is met only exactly.
fn score(outs: &[Out], wants: &[Want]) -> f64 {
    outs.iter()
        .zip(wants)
        .map(|(o, w)| {
            let e = o.ppm(w);
            if w.ppm > 0.0 {
                e / w.ppm
            } else if e < 1e-6 {
                0.0
            } else {
                f64::INFINITY
            }
        })
        .fold(0.0, f64::max)
}

/// The settings of `primitive` that make `wants` from `input` hertz, on a
/// part with `limits`.
pub fn plan(
    primitive: Primitive,
    limits: &Limits,
    input: f64,
    wants: &[Want],
) -> Result<Plan, Refusal> {
    if !limits.input.holds(input) {
        return Err(Refusal::Input(input));
    }
    if wants.len() > limits.outputs || wants.is_empty() {
        return Err(Refusal::TooMany {
            asked: wants.len(),
            has: limits.outputs,
        });
    }
    let m_step = if primitive == Primitive::Mmcm { 1 } else { 8 };
    let mut best: Option<(f64, Plan)> = None;
    // The nearest any setting came, for the refusal.
    let mut nearest: Option<(f64, Vec<Out>)> = None;
    for d in 1..=limits.d_max {
        let pfd = input / d as f64;
        if !limits.pfd.holds(pfd) {
            continue;
        }
        for m8 in (16..=512).step_by(m_step) {
            let vco = input * m8 as f64 / (8.0 * d as f64);
            if !limits.vco.holds(vco) {
                continue;
            }
            let outs: Option<Vec<Out>> = wants
                .iter()
                .enumerate()
                .map(|(k, w)| best_out(limits, primitive, k, (input, d, m8), w))
                .collect();
            let Some(outs) = outs else { continue };
            let s = score(&outs, wants);
            if nearest.as_ref().is_none_or(|(n, _)| s < *n) {
                nearest = Some((s, outs.clone()));
            }
            let p = Plan {
                primitive,
                input,
                d,
                m8,
                pfd,
                vco,
                outs,
            };
            let better = match &best {
                None => true,
                Some((bs, bp)) => {
                    s < *bs
                        || (s == *bs && p.vco > bp.vco)
                        || (s == *bs && p.vco == bp.vco && p.d < bp.d)
                }
            };
            if better {
                best = Some((s, p));
            }
        }
    }
    match best {
        Some((s, p)) if s <= 1.0 => Ok(p),
        _ => Err(Refusal::Unmet(
            nearest
                .map(|(_, outs)| {
                    outs.iter()
                        .zip(wants)
                        .map(|(o, w)| (o.ppm(w), w.ppm))
                        .collect()
                })
                .unwrap_or_default(),
        )),
    }
}

/// The plan of given settings, the input divider `d`, the feedback
/// multiplier `m8` and each output's divider in eighths, with its
/// frequencies worked out and nothing checked: for the module that tests
/// a limit by breaking it (`//tools/clockgen:drc_test`).
pub fn fixed(
    primitive: Primitive,
    input: f64,
    d: u32,
    m8: u32,
    o8s: &[u32],
) -> Plan {
    Plan {
        primitive,
        input,
        d,
        m8,
        pfd: input / d as f64,
        vco: input * m8 as f64 / (8.0 * d as f64),
        outs: o8s
            .iter()
            .map(|&o8| Out {
                o8,
                hz: input * m8 as f64 / (d as f64 * o8 as f64),
            })
            .collect(),
    }
}

/// An eighths count as Verilog writes a real parameter: `63.125`.
fn eighths(v: u32) -> String {
    format!("{}.{:03}", v / 8, (v % 8) * 125)
}

/// The Verilog module `name` that makes `plan`'s outputs from `clk_in`:
/// the primitive, its feedback and every output through a `BUFG`, and
/// `locked`. Its comment says what was asked and what each output gets.
pub fn verilog(name: &str, plan: &Plan, wants: &[Want]) -> String {
    let mut v = String::new();
    let p = plan;
    let (prim, mult) = match p.primitive {
        Primitive::Mmcm => ("MMCME2_BASE", "CLKFBOUT_MULT_F"),
        Primitive::Pll => ("PLLE2_BASE", "CLKFBOUT_MULT"),
    };
    let _ = writeln!(
        v,
        "// Generated by //tools/clockgen (issue 304); do not edit.\n\
         // {prim} from {:.6} MHz: D = {}, M = {}, the phase detector at \
         {:.6} MHz and the oscillator at {:.6} MHz.",
        p.input / MHZ,
        p.d,
        eighths(p.m8),
        p.pfd / MHZ,
        p.vco / MHZ
    );
    for (k, (o, w)) in p.outs.iter().zip(wants).enumerate() {
        let _ = writeln!(
            v,
            "// clk_{k}: asked {:.6} MHz within {} ppm, gets {:.6} MHz \
             (O = {}), {:.1} ppm off.",
            w.hz / MHZ,
            w.ppm,
            o.hz / MHZ,
            eighths(o.o8),
            o.ppm(w)
        );
    }
    let ports: Vec<String> = (0..p.outs.len())
        .map(|k| format!("output clk_{k}"))
        .collect();
    let _ = writeln!(
        v,
        "`timescale 1ps / 1ps\nmodule {name} (\n  input clk_in,\n  input rst,\n  {},\n  output locked\n);",
        ports.join(",\n  ")
    );
    let _ = writeln!(v, "  wire fb, fb_buf;");
    for k in 0..p.outs.len() {
        let _ = writeln!(v, "  wire raw_{k};");
    }
    let mut params = vec![
        format!(".CLKIN1_PERIOD({:.3})", 1e9 / p.input),
        format!(".DIVCLK_DIVIDE({})", p.d),
        match p.primitive {
            Primitive::Mmcm => format!(".{mult}({})", eighths(p.m8)),
            Primitive::Pll => format!(".{mult}({})", p.m8 / 8),
        },
    ];
    for (k, o) in p.outs.iter().enumerate() {
        if p.primitive == Primitive::Mmcm && k == 0 {
            params.push(format!(".CLKOUT0_DIVIDE_F({})", eighths(o.o8)));
        } else {
            params.push(format!(".CLKOUT{k}_DIVIDE({})", o.o8 / 8));
        }
    }
    let _ =
        writeln!(v, "  {prim} #(\n    {}\n  ) mgr (", params.join(",\n    "));
    let mut pins = vec![
        ".CLKIN1(clk_in)".to_string(),
        ".RST(rst)".to_string(),
        ".PWRDWN(1'b0)".to_string(),
        ".CLKFBIN(fb_buf)".to_string(),
        ".CLKFBOUT(fb)".to_string(),
        ".LOCKED(locked)".to_string(),
    ];
    for k in 0..p.outs.len() {
        pins.push(format!(".CLKOUT{k}(raw_{k})"));
    }
    let _ = writeln!(v, "    {}\n  );", pins.join(",\n    "));
    let _ = writeln!(v, "  BUFG fb_bufg (.I(fb), .O(fb_buf));");
    for k in 0..p.outs.len() {
        let _ = writeln!(v, "  BUFG out_{k} (.I(raw_{k}), .O(clk_{k}));");
    }
    let _ = writeln!(v, "endmodule");
    v
}

/// A testbench for the module `name` that [`verilog`] wrote for `plan`:
/// the input clock at its frequency, the reset let go, and once the
/// manager says `locked`, each output's period measured over 64 of its
/// cycles and held to the plan's within 200 parts per million and two
/// picoseconds. `ok` says whether every output passed, which a verdict
/// script reads (`//tools/clockgen:sim_test`).
pub fn testbench(name: &str, plan: &Plan) -> String {
    let mut v = String::new();
    let n = plan.outs.len();
    let half = 1e12 / plan.input / 2.0;
    let slowest = plan.outs.iter().map(|o| 1e12 / o.hz).fold(0.0, f64::max);
    let _ = writeln!(
        v,
        "// Generated by //tools/clockgen (issue 304); do not edit.\n\
         `timescale 1ps / 1ps\nmodule {name}_tb;\n  reg clk = 0;\n  \
         always #({half:.3}) clk = ~clk;\n  reg rst = 1;\n  \
         initial #100000 rst = 0;\n  wire locked;"
    );
    for k in 0..n {
        let _ = writeln!(
            v,
            "  wire c{k};\n  realtime t0_{k}, t1_{k};\n  integer n{k} = 0;"
        );
    }
    let pins: Vec<String> = (0..n).map(|k| format!(".clk_{k}(c{k})")).collect();
    let _ = writeln!(
        v,
        "  {name} dut (.clk_in(clk), .rst(rst), {}, .locked(locked));\n  \
         reg ok = 1;\n  real p;",
        pins.join(", ")
    );
    for k in 0..n {
        let _ = writeln!(
            v,
            "  always @(posedge c{k}) if (locked) begin\n    \
             if (n{k} == 16) t0_{k} = $realtime;\n    \
             if (n{k} == 80) t1_{k} = $realtime;\n    \
             n{k} = n{k} + 1;\n  end"
        );
    }
    let _ = writeln!(
        v,
        "  initial begin\n    wait (locked);\n    #({:.0});",
        90.0 * slowest
    );
    for (k, o) in plan.outs.iter().enumerate() {
        let want = 1e12 / o.hz;
        let tol = want * 200e-6 + 2.0;
        let _ = writeln!(
            v,
            "    if (n{k} < 81) begin\n      ok = 0;\n      \
             $display(\"clk_{k}: %0d edges after locked\", n{k});\n    \
             end else begin\n      p = (t1_{k} - t0_{k}) / 64.0;\n      \
             if (p < {:.3} || p > {:.3}) ok = 0;\n      \
             $display(\"clk_{k}: period %f ps, planned {want:.3} ps\", p);\n    end",
            want - tol,
            want + tol
        );
    }
    let _ = writeln!(
        v,
        "    $display(\"verdict %0d\", ok);\n    $finish;\n  end\n  \
         initial begin\n    #2000000000;\n    ok = 0;\n    \
         $display(\"timed out, locked %0d\", locked);\n    $finish;\n  end\nendmodule"
    );
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want(mhz: f64, ppm: f64) -> Want {
        Want { hz: mhz * MHZ, ppm }
    }

    fn mmcm(input_mhz: f64, wants: &[Want]) -> Result<Plan, Refusal> {
        let p = Primitive::Mmcm;
        plan(p, &Limits::artix7_2(p), input_mhz * MHZ, wants)
    }

    /// Every plan keeps the phase detector, the oscillator and each
    /// output inside the part's ranges.
    fn inside(p: &Plan) {
        let l = Limits::artix7_2(p.primitive);
        assert!(l.pfd.holds(p.pfd), "{p:?}");
        assert!(l.vco.holds(p.vco), "{p:?}");
        assert!(p.outs.iter().all(|o| l.output.holds(o.hz)), "{p:?}");
    }

    /// The flagship's Ethernet clock: 125 MHz from 200 MHz, exactly.
    #[test]
    fn the_flagships_125_mhz_is_made_exactly() {
        let w = [want(125.0, 0.0)];
        let p = mmcm(200.0, &w).expect("a plan");
        inside(&p);
        assert_eq!(p.outs[0].hz, 125.0 * MHZ, "{p:?}");
    }

    /// The flagship's pixel clock: VGA's 25.175 MHz from 200 MHz, no
    /// further off than the hand-picked D = 10, M = 63, O = 50, which makes
    /// 25.2 MHz, about 993 ppm off.
    #[test]
    fn the_pixel_clock_is_as_near_as_the_hand_picked_one() {
        let w = [want(25.175, 1000.0)];
        let p = mmcm(200.0, &w).expect("a plan");
        inside(&p);
        let hand = Out {
            o8: 50 * 8,
            hz: 200.0 * MHZ / 10.0 * 63.0 / 50.0,
        };
        assert!(p.outs[0].ppm(&w[0]) <= hand.ppm(&w[0]), "{p:?}");
    }

    /// Fractional settings only where the MMCM allows them: its feedback
    /// and its first output. A PLL's are all whole.
    #[test]
    fn eighths_only_where_the_primitive_takes_them() {
        let w = [want(25.175, 1000.0), want(33.3, 1000.0)];
        let p = mmcm(200.0, &w).expect("a plan");
        assert_eq!(p.outs[1].o8 % 8, 0, "{p:?}");
        let pl = Primitive::Pll;
        let w = [want(25.0, 0.0), want(33.333, 1000.0)];
        let q = plan(pl, &Limits::artix7_2(pl), 200.0 * MHZ, &w)
            .expect("a PLL plan");
        inside(&q);
        assert_eq!(q.m8 % 8, 0, "{q:?}");
        assert!(q.outs.iter().all(|o| o.o8 % 8 == 0), "{q:?}");
    }

    /// Seven outputs from one MMCM, each a whole fraction of a common
    /// oscillator, all exact.
    #[test]
    fn seven_outputs_from_one_mmcm() {
        let w: Vec<Want> = [200.0, 100.0, 50.0, 25.0, 40.0, 125.0, 250.0]
            .iter()
            .map(|&m| want(m, 0.0))
            .collect();
        let p = mmcm(200.0, &w).expect("a plan");
        inside(&p);
        for (o, w) in p.outs.iter().zip(&w) {
            assert_eq!(o.hz, w.hz, "{p:?}");
        }
    }

    /// What nothing reaches is refused: eight outputs from an MMCM, an
    /// input below the range, an output above it, and an exact frequency
    /// no setting divides to.
    #[test]
    fn what_cannot_be_made_is_refused() {
        let eight = vec![want(100.0, 0.0); 8];
        assert_eq!(
            mmcm(200.0, &eight),
            Err(Refusal::TooMany { asked: 8, has: 7 })
        );
        assert_eq!(
            mmcm(5.0, &[want(100.0, 0.0)]),
            Err(Refusal::Input(5.0 * MHZ))
        );
        assert!(matches!(
            mmcm(200.0, &[want(900.0, 0.0)]),
            Err(Refusal::Unmet(_))
        ));
        assert!(matches!(
            mmcm(200.0, &[want(123.456789, 0.0)]),
            Err(Refusal::Unmet(_))
        ));
    }

    /// The module instantiates the primitive with the plan's parameters,
    /// fractional where it may be, and a buffer on every output.
    #[test]
    fn the_module_says_the_plan() {
        let w = [want(125.0, 0.0), want(50.0, 0.0)];
        let p = mmcm(200.0, &w).expect("a plan");
        let v = verilog("clk_eth", &p, &w);
        assert!(v.contains("module clk_eth ("), "{v}");
        assert!(v.contains("MMCME2_BASE #("), "{v}");
        assert!(v.contains(".CLKIN1_PERIOD(5.000)"), "{v}");
        assert!(
            v.contains(&format!(".CLKFBOUT_MULT_F({})", eighths(p.m8))),
            "{v}"
        );
        assert!(v.contains(".CLKOUT0_DIVIDE_F("), "{v}");
        assert!(v.contains(".CLKOUT1_DIVIDE("), "{v}");
        assert!(v.contains("BUFG out_1 (.I(raw_1), .O(clk_1));"), "{v}");
        assert!(v.contains("output locked"), "{v}");
    }
}
