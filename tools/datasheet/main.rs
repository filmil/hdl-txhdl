// SPDX-License-Identifier: Apache-2.0
//! The generated half of the datasheets: for every component, the
//! parameters it is lowered with here, and its ports, its state and its
//! children as its own lowering states them, written as LaTeX
//! definitions on standard output.
//!
//! A datasheet in `docs/datasheets/` asks for its tables by the
//! component's key, and the document refuses to build when this
//! program has not defined them. So a component's ports on its sheet
//! are always the ports its netlist has.
use txhdl::comp::trace::Kind;
use txhdl::netlist::Lowered;

use ddr3::Ddr3Per;
use gpu::fb::Fb;
use gpu::raster::Raster;
use txhdl::types::U;
use txhdl_parts::buffer::Buffer;
use txhdl_parts::bus::axi::{AxiHost, AxiPer};
use txhdl_parts::bus::axi_lite::LiteBridge1;
use txhdl_parts::bus::noc::bridge::{HostBridge, PerBridge};
use txhdl_parts::bus::noc::node::Node;
use txhdl_parts::bus::noc::switch::Switch;
use txhdl_parts::bus::router::Router3;
use txhdl_parts::bus::wb::AxiWb;
use txhdl_parts::eth::{EthLite, EthRx, EthTx};
use txhdl_parts::fifo::Fifo;
use txhdl_parts::hdmi::{vga, Hdmi, I2cInit};
use txhdl_parts::plic::Plic2;
use txhdl_parts::station::Station3;
use vreteno32::board::Board;
use vreteno32::core::Vreteno;
use vreteno32::dmem::Dmem;
use vreteno32::timer::Timer;
use vreteno32::uart::Uart;

/// Text for LaTeX: the characters a name or a type may hold that
/// LaTeX would read otherwise.
fn tex(s: &str) -> String {
    s.replace('\\', "\\textbackslash{}")
        .replace('_', "\\_")
        .replace('&', "\\&")
        .replace('#', "\\#")
        .replace('{', "\\{")
        .replace('}', "\\}")
}

/// What a port or a field is, in a datasheet's words.
fn kind(k: Option<Kind>) -> &'static str {
    match k {
        Some(Kind::In) => "in, wire",
        Some(Kind::Out) => "out, wire",
        Some(Kind::Rx) => "in, channel",
        Some(Kind::Tx) => "out, channel",
        Some(Kind::Reg) => "register",
        Some(Kind::Mem) => "memory",
        Some(Kind::Wire) => "wire kept",
        Some(Kind::Pad) => "pad, both ways",
        None => "part",
    }
}

/// One LaTeX definition, `\ds@<what>@<key>`, holding `body`.
fn define(what: &str, key: &str, body: &str) {
    println!(
        "\\expandafter\\def\\csname ds@{what}@{key}\\endcsname{{%\n{body}}}"
    );
}

/// A table, or a line saying there is nothing to put in one.
fn table(cols: &str, head: &str, rows: &[String], none: &str) -> String {
    if rows.is_empty() {
        return format!("\\noindent\\emph{{{none}}}\\par\n");
    }
    format!(
        "\\begin{{center}}\\footnotesize\n\\begin{{tabular}}{{@{{}}{cols}@{{}}}}\n\
         \\toprule\n{head} \\\\\n\\midrule\n{}\n\\bottomrule\n\
         \\end{{tabular}}\n\\end{{center}}\n",
        rows.join("\n")
    )
}

/// A count and its noun, the noun plural unless the count is one.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The tables of one component, lowered as `ty` states it.
fn sheet(key: &str, ty: &str, net: Lowered) {
    define("type", key, &format!("\\code{{{}}}", tex(ty)));
    let ports: Vec<String> = net
        .ports
        .iter()
        .map(|(n, k, w)| {
            format!("\\code{{{}}} & {} & {} \\\\", tex(n), kind(Some(*k)), w)
        })
        .collect();
    define(
        "ports",
        key,
        &table("lll", "Port & What & Width", &ports, "No ports."),
    );
    // A child is a field of its parent, and is in the children's table,
    // so the state is the fields of a kind.
    let own: Vec<_> = net.fields.iter().filter(|f| f.1.is_some()).collect();
    let state: Vec<String> = own
        .iter()
        .map(|(n, k, w, d)| {
            let depth = if *d > 0 { d.to_string() } else { String::new() };
            format!(
                "\\code{{{}}} & {} & {} & {} \\\\",
                tex(n),
                kind(*k),
                w,
                depth
            )
        })
        .collect();
    define(
        "state",
        key,
        &table(
            "llrr",
            "Field & What & Width & Words",
            &state,
            "No state of its own.",
        ),
    );
    let children: Vec<String> = net
        .instances
        .iter()
        .map(|i| {
            let what = match &i.unit.foreign {
                Some(_) => "foreign module",
                None => "lowered unit",
            };
            format!(
                "\\code{{{}}} & \\code{{{}}} & {} \\\\",
                tex(&i.name),
                tex(&i.unit.name),
                what
            )
        })
        .collect();
    define(
        "children",
        key,
        &table(
            "lll",
            "Child & Module & What",
            &children,
            "No children: the unit is a leaf.",
        ),
    );
    let bits: usize = own
        .iter()
        .map(|(_, k, w, d)| match k {
            Some(Kind::Mem) => w * d,
            _ => *w,
        })
        .sum();
    define(
        "counts",
        key,
        &format!(
            "{}, {} holding {}, {} and {}",
            count(net.ports.len(), "port", "ports"),
            count(own.len(), "field", "fields"),
            count(bits, "bit", "bits"),
            count(net.instances.len(), "child", "children"),
            count(net.procs.len(), "process", "processes"),
        ),
    );
}

fn main() {
    println!("% Generated by //tools/datasheet. Do not edit.");
    sheet("Buffer", "Buffer<8>", Buffer::<8>::lowered("buffer"));
    sheet(
        "Fifo",
        "Fifo<U<8>, 2, 4>",
        Fifo::<U<8>, 2, 4>::lowered("fifo"),
    );
    sheet(
        "Station",
        "Station3<4, 16, U<2>, U<16>, U<16>>",
        Station3::<4, 16, U<2>, U<16>, U<16>>::lowered("station"),
    );
    sheet(
        "AxiHost",
        "AxiHost<32, 32, 4, 2, 4>",
        AxiHost::<32, 32, 4, 2, 4>::lowered("axi_host"),
    );
    sheet(
        "AxiPer",
        "AxiPer<32, 32, 4, 2>",
        AxiPer::<32, 32, 4, 2>::lowered("axi_per"),
    );
    sheet(
        "Router",
        "Router3<32, 32, 4, 2, 0x1000, 0xf000, 0x2000, 0xf000, 0x3000, 0xf000>",
        Router3::<32, 32, 4, 2, 0x1000, 0xf000, 0x2000, 0xf000, 0x3000, 0xf000>::lowered("router"),
    );
    sheet(
        "LiteBridge",
        "LiteBridge1<32, 32, 4, 2, 0x3000, 0xf000>",
        LiteBridge1::<32, 32, 4, 2, 0x3000, 0xf000>::lowered("lite_bridge"),
    );
    sheet(
        "AxiWb",
        "AxiWb<32, 2, 28>",
        AxiWb::<32, 2, { ddr3::AW }>::lowered("axi_wb"),
    );
    sheet(
        "Switch",
        "Switch<0, 0, 2, 2, 32, 32, 4, 2>",
        Switch::<0, 0, 2, 2, 32, 32, 4, 2>::lowered("switch"),
    );
    sheet(
        "Node",
        "Node<0, 0, 2, 2, 32, 32, 4, 2>",
        Node::<0, 0, 2, 2, 32, 32, 4, 2>::lowered("node"),
    );
    sheet(
        "HostBridge",
        "soc::Bridge<0, 0>",
        HostBridge::<
            0,
            0,
            2,
            2,
            32,
            32,
            4,
            2,
            0x3000,
            0xffff_f000,
            1,
            1,
            0x1000,
            0xffff_f000,
            0,
            1,
            0,
            0,
            0,
            1,
        >::lowered("host_bridge"),
    );
    sheet(
        "PerBridge",
        "PerBridge<0, 1, 2, 2, 32, 32, 4, 2, 4>",
        PerBridge::<0, 1, 2, 2, 32, 32, 4, 2, 4>::lowered("per_bridge"),
    );
    sheet("EthTx", "EthTx", EthTx::lowered("eth_tx"));
    sheet("EthRx", "EthRx", EthRx::lowered("eth_rx"));
    sheet("EthLite", "EthLite", EthLite::lowered("eth_lite"));
    sheet(
        "Hdmi",
        "Hdmi<640, 16, 96, 48, 480, 10, 2, 33, 2>",
        Hdmi::<
            { vga::HV },
            { vga::HFP },
            { vga::HSW },
            { vga::HBP },
            { vga::VV },
            { vga::VFP },
            { vga::VSW },
            { vga::VBP },
            2,
        >::lowered("hdmi_video"),
    );
    sheet(
        "I2cInit",
        "I2cInit<63, 2520000>",
        I2cInit::<63, 2_520_000>::lowered("hdmi_i2c"),
    );
    sheet("Vreteno", "Vreteno<2>", Vreteno::<2>::lowered("vreteno"));
    sheet("Dmem", "Dmem<2>", Dmem::<2>::lowered("dmem"));
    sheet("Timer", "Timer<2>", Timer::<2>::lowered("timer"));
    sheet("Plic", "Plic2<0>", Plic2::<0>::lowered("plic"));
    sheet("Uart", "Uart<868>", Uart::<868>::lowered("uart"));
    sheet("Ddr3Per", "Ddr3Per<0, 0>", Ddr3Per::<0, 0>::lowered("ddr3"));
    sheet(
        "Board",
        "Board<868, 0, 0>",
        Board::<868, 0, 0>::lowered("board"),
    );
    sheet("Fb", "Fb<16, 2, 1024>", Fb::<16, 2, 1024>::lowered("fb"));
    sheet(
        "Raster",
        "Raster<16, 2, 4, 16, 0, 0x400, 0x600>",
        Raster::<16, 2, 4, 16, 0, 0x400, 0x600>::lowered("raster"),
    );
}
