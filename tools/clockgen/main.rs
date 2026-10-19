// SPDX-License-Identifier: Apache-2.0
//! The clock generator's command line (issue 304): plan a 7-series clock
//! manager for the clocks asked for, and write the module that makes
//! them and a report of the plan, or say what cannot be made.
//!
//!   clockgen --name clk_eth --primitive mmcm --input-hz 200000000 \
//!       --out 125000000:0 --out 25175000:1000 \
//!       --verilog clk_eth.v --report clk_eth.txt
//!
//! Each `--out` is a frequency in hertz and its tolerance in parts per
//! million, nought for exactly.

use clockgen::{
    fixed, plan, testbench, verilog, Limits, Primitive, Refusal, Want,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut name, mut prim, mut input) = (None, Primitive::Mmcm, None);
    let (mut wants, mut vpath, mut rpath) = (Vec::new(), None, None);
    let mut settings: Option<Vec<u32>> = None;
    let mut tbpath: Option<String> = None;
    let mut whole = false;
    while let Some(a) = args.next() {
        let mut val =
            || args.next().unwrap_or_else(|| panic!("{a} wants a value"));
        match a.as_str() {
            "--name" => name = Some(val()),
            "--primitive" => {
                prim = match val().as_str() {
                    "mmcm" => Primitive::Mmcm,
                    "pll" => Primitive::Pll,
                    p => panic!("a primitive is mmcm or pll, not {p}"),
                }
            }
            "--input-hz" => input = Some(val().parse::<f64>().expect("hertz")),
            "--out" => {
                let v = val();
                let (hz, ppm) = v.split_once(':').unwrap_or((&v, "0"));
                wants.push(Want {
                    hz: hz.parse().expect("hertz"),
                    ppm: ppm.parse().expect("parts per million"),
                });
            }
            "--verilog" => vpath = Some(val()),
            "--report" => rpath = Some(val()),
            "--testbench" => tbpath = Some(val()),
            // Whole numbers only, an MMCM as a PLL divides (#1830).
            "--whole" => whole = true,
            // Given settings, `D,M8,O8[,O8...]`, written as they are with
            // nothing checked: a limit's test breaks it on purpose.
            "--settings" => {
                settings = Some(
                    val()
                        .split(',')
                        .map(|s| s.parse().expect("a whole number"))
                        .collect(),
                )
            }
            _ => panic!("unknown argument {a}"),
        }
    }
    let name = name.expect("--name is required");
    let input = input.expect("--input-hz is required");
    if let Some(s) = settings {
        if s.len() < 3 {
            eprintln!("{name}: --settings is D,M8,O8[,O8...], three or more");
            std::process::exit(2);
        }
        let p = fixed(prim, input, s[0], s[1], &s[2..]);
        let wants: Vec<Want> =
            p.outs.iter().map(|o| Want { hz: o.hz, ppm: 0.0 }).collect();
        let v = verilog(&name, &p, &wants);
        if let Some(path) = vpath {
            std::fs::write(&path, &v).unwrap_or_else(|e| panic!("{path}: {e}"));
        }
        return;
    }
    let mut limits = Limits::artix7_2(prim);
    limits.fractional &= !whole;
    match plan(prim, &limits, input, &wants) {
        Ok(p) => {
            let v = verilog(&name, &p, &wants);
            let report: String = v
                .lines()
                .take_while(|l| l.starts_with("//"))
                .fold(String::new(), |mut s, l| {
                    s.push_str(l.trim_start_matches("// "));
                    s.push('\n');
                    s
                });
            print!("{report}");
            if let Some(path) = vpath {
                std::fs::write(&path, &v)
                    .unwrap_or_else(|e| panic!("{path}: {e}"));
            }
            if let Some(path) = rpath {
                std::fs::write(&path, &report)
                    .unwrap_or_else(|e| panic!("{path}: {e}"));
            }
            if let Some(path) = tbpath {
                std::fs::write(&path, testbench(&name, &p))
                    .unwrap_or_else(|e| panic!("{path}: {e}"));
            }
        }
        Err(r) => {
            match r {
                Refusal::Input(hz) => eprintln!(
                    "{name}: the input, {hz} Hz, is outside the part's range"
                ),
                Refusal::TooMany { asked: 0, .. } => {
                    eprintln!("{name}: no outputs asked")
                }
                Refusal::TooMany { asked, has } => {
                    eprintln!("{name}: {asked} outputs asked, the primitive has {has}")
                }
                Refusal::Want(k) => eprintln!(
                    "{name}: clk_{k} wants a frequency above nought and a \
                     tolerance not below it"
                ),
                Refusal::Unmet(near) if near.is_empty() => eprintln!(
                    "{name}: no setting puts every output inside the \
                     part's output range"
                ),
                Refusal::Unmet(near) => {
                    eprintln!(
                        "{name}: no setting meets every output; the nearest:"
                    );
                    for (k, (got, allowed)) in near.iter().enumerate() {
                        eprintln!(
                            "  clk_{k}: {got:.1} ppm off, {allowed} allowed"
                        );
                    }
                }
            }
            std::process::exit(1);
        }
    }
}
