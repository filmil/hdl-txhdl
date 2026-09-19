// SPDX-License-Identifier: Apache-2.0
//! Check FST files: read one with wellen, the reader Surfer uses, and
//! print its time table and every variable's changes; or write small
//! ones with fst-writer, to see which usage the pair agrees on.
//!
//! Usage: fstcheck read FILE.fst [HOW MANY TO PRINT]
//!        fstcheck write FILE.fst [dup]
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("read") => read(&args[2]),
        Some("write") => write(&args[2], args.get(3).is_some()),
        Some("test") => write_test(&args[2]),
        _ => eprintln!("usage: fstcheck read FILE [N] | write FILE [dup]"),
    }
}

fn read(path: &str) {
    let opts = wellen::LoadOptions::default();
    let mut w = wellen::simple::read_with_options(path, &opts).expect("read");
    println!("time table: {:?}", w.time_table());
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
    // How many there are, before any of them: a listing that stopped
    // at the first few once read as a trace that held only those, and
    // the hunt went looking for the writer rather than for the reader.
    println!("variables: {}", vars.len());
    // All of them unless a count is given, since the question is
    // usually whether a signal is there at all.
    let show: usize = std::env::args()
        .nth(3)
        .and_then(|v| v.parse().ok())
        .unwrap_or(vars.len());
    for (name, s) in vars.iter().take(show) {
        let sig = w.get_signal(*s).unwrap();
        let changes: Vec<String> = sig
            .iter_changes()
            .map(|(t, val)| {
                format!("{t}:{}", val.to_bit_string().unwrap_or_default())
            })
            .collect();
        println!("{name}: {}", changes.join(" "));
    }
    let _ = std::io::stdout().flush();
}

/// The crate's own round-trip test, verbatim.
fn write_test(path: &str) {
    use fst_writer::*;
    let ts: i8 = std::env::var("FST_TS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let info = FstInfo {
        start_time: 0,
        timescale_exponent: ts,
        version: "test 0.2.3".to_string(),
        date: "2034-10-10".to_string(),
        file_type: FstFileType::Verilog,
    };
    let mut writer = open_fst(path, &info).unwrap();
    writer
        .scope("simple", "Simple", FstScopeType::Module)
        .unwrap();
    let a = writer
        .var(
            "a",
            FstSignalType::bit_vec(1),
            FstVarType::Logic,
            FstVarDirection::Implicit,
            None,
        )
        .unwrap();
    let wide = std::env::var("FST_WIDE").is_ok();
    let w4 = if wide {
        Some(
            writer
                .var(
                    "w",
                    FstSignalType::bit_vec(4),
                    FstVarType::Logic,
                    FstVarDirection::Implicit,
                    None,
                )
                .unwrap(),
        )
    } else {
        None
    };
    writer.up_scope().unwrap();
    let mut writer = writer.finish().unwrap();
    writer.signal_change(a, b"0").unwrap();
    if let Some(w) = w4 {
        writer.signal_change(w, b"0000").unwrap();
    }
    let many: u64 = std::env::var("FST_MANY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    for t in 1..=many {
        writer.time_change(t).unwrap();
        writer
            .signal_change(a, if t % 2 == 1 { b"1" } else { b"0" })
            .unwrap();
        if let Some(w) = w4 {
            writer
                .signal_change(w, format!("{:04b}", t % 16).as_bytes())
                .unwrap();
        }
    }
    writer.finish().unwrap();
}

fn write(path: &str, dup: bool) {
    use fst_writer::*;
    let info = FstInfo {
        start_time: 0,
        timescale_exponent: -9,
        version: "fstcheck 0.1".to_string(),
        date: std::env::var("FST_DATE").unwrap_or_default(),
        file_type: FstFileType::Verilog,
    };
    let mut h = open_fst(path, &info).expect("open");
    h.scope("top", "", FstScopeType::Module).expect("scope");
    let a = h
        .var(
            "a",
            FstSignalType::bit_vec(4),
            FstVarType::Reg,
            FstVarDirection::Implicit,
            None,
        )
        .expect("var");
    let vt = if std::env::var("FST_LOGIC").is_ok() {
        FstVarType::Logic
    } else {
        FstVarType::Wire
    };
    let c = h
        .var(
            "clk",
            FstSignalType::bit_vec(1),
            vt,
            FstVarDirection::Implicit,
            None,
        )
        .expect("var");
    h.up_scope().expect("up");
    let mut b = h.finish().expect("finish header");
    // Initial values before any time change, as the crate's test does;
    // `zero` in the mode name adds an explicit time_change(0) first.
    if std::env::var("FST_ZERO").is_ok() {
        b.time_change(0).expect("t");
    }
    b.signal_change(a, b"0000").expect("c");
    b.signal_change(c, b"0").expect("c");
    for t in 0..12u64 {
        if t == 0 && !dup {
            continue;
        }
        b.time_change(t).expect("t");
        b.signal_change(c, if t % 2 == 0 { b"1" } else { b"0" })
            .expect("c");
        if t % 2 == 0 {
            b.signal_change(a, format!("{:04b}", t / 2 + 1).as_bytes())
                .expect("c");
        }
    }
    b.finish().expect("finish");
}
