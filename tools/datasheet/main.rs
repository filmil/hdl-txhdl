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
use txhdl::comp::{Clock, DefaultClock};
use txhdl::map::AddrMap;
use txhdl::netlist::Lowered;

use ddr3::Ddr3Per;
use pcie::bar::{BarRegs, PcieBar};
use razboj::fb::Fb;
use razboj::raster::Raster;
use txhdl::regmap::RegMap;
use txhdl::types::U;
use txhdl_parts::buffer::Buffer;
use txhdl_parts::bus::arbiter::Arbiter2;
use txhdl_parts::bus::axi::{AxiHost, AxiPer};
use txhdl_parts::bus::axi::{Issue, R};
use txhdl_parts::bus::axi_lite::LiteBridge;
use txhdl_parts::bus::axi_pins::AxiPins;
use txhdl_parts::bus::noc::bridge::{HostBridge, PerBridge};
use txhdl_parts::bus::noc::mesh::Mesh;
use txhdl_parts::bus::noc::node::Node;
use txhdl_parts::bus::noc::switch::Switch;
use txhdl_parts::bus::router::Router;
use txhdl_parts::bus::wb::AxiWb;
use txhdl_parts::cdc::ChanCdc;
use txhdl_parts::dma::{LineBuf, LineFetch, LineStore, NoBeats, NoReads};
use txhdl_parts::eth::{EthLite, EthRx, EthTx};
use txhdl_parts::ethdma::{FrameIn, FrameLen, FrameOut};
use txhdl_parts::ethshare::EthShare;
use txhdl_parts::ethslots::EthSlots;
use txhdl_parts::fifo::Fifo;
use txhdl_parts::flashwin::FlashWin;
use txhdl_parts::gpio::Gpio;
use txhdl_parts::hdmi::{vga, Hdmi, I2cInit};
use txhdl_parts::i2c::I2c;
use txhdl_parts::plic::Plic3;
use txhdl_parts::pwm::Pwm;
use txhdl_parts::redundant::{Check, Tee};
use txhdl_parts::remote::eth::RemoteLink;
use txhdl_parts::remote::Remote;
use txhdl_parts::sd::Sd;
use txhdl_parts::spi::Spi;
use txhdl_parts::station::Station3;
use txhdl_parts::syscon::Syscon;
use txhdl_parts::tracer::Tracer;
use txhdl_parts::trng::{Entropy, Trng};
use txhdl_parts::wdog::Wdog;
use vreteno32::board::Board;
use vreteno32::core::Vreteno;
use vreteno32::debug::Dm;
use vreteno32::dmem::Dmem;
use vreteno32::pair::{Inject, Pair, Watch};
use vreteno32::rom::Rom;
use vreteno32::timer::Timer;
use vreteno32::uart::Uart;

/// The map of the bridge the sheet is written for: Vreteno's serial
/// port, a nibble of the address space at 0x3000.
struct SerialMap;

impl AddrMap<1> for SerialMap {
    const RANGES: [(usize, usize); 1] = [(0x3000, 0xf000)];
}

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

/// A component's register table, from the map its peripheral declares
/// (issue 499): the offset, the name, the access and the sentence.
fn regs(key: &str, map: &RegMap) {
    define(
        "regs",
        key,
        &table(
            "llp{0.16\\textwidth}p{0.42\\textwidth}",
            "Offset & Register & Access & What it is",
            &map.tex_rows(),
            "No registers.",
        ),
    );
}

/// A count and its noun, the noun plural unless the count is one.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A port's label in the diagram: its name, and its width where the
/// width says something, which a one-bit wire's does not.
fn label(n: &str, k: Kind, w: usize) -> String {
    let what = match k {
        Kind::Rx | Kind::Tx => "~(ch)",
        _ => "",
    };
    if w == 1 {
        format!("\\code{{{}}}{what}", tex(n))
    } else {
        format!("\\code{{{}}}{what}~[{w}]", tex(n))
    }
}

/// The diagram of a component's interfaces: a box with the inputs
/// down its left side and the outputs down its right, each arrow
/// labelled with the port and its width. A channel is marked, since
/// it is three wires rather than one and carries a handshake.
///
/// It is drawn from the lowering, as the tables are, so it cannot say
/// a port the netlist does not have.
fn figure(key: &str, net: &Lowered) -> String {
    let side = |want: &[Kind]| -> Vec<(String, Kind, usize)> {
        net.ports
            .iter()
            .filter(|(_, k, _, _)| want.contains(k))
            .map(|(n, k, w, _)| (n.clone(), *k, *w))
            .collect()
    };
    let ins = side(&[Kind::In, Kind::Rx]);
    let outs = side(&[Kind::Out, Kind::Tx]);
    let pads = side(&[Kind::Pad]);
    let rows = ins.len().max(outs.len()).max(1);
    // A row is 7mm, and the box is as tall as the longer side.
    let height = 7 * rows + 6;
    let mut out = String::new();
    out.push_str(
        "\\begin{center}\\footnotesize\n\\begin{tikzpicture}[\n\
         every node/.style={font=\\footnotesize},\n\
         pin/.style={-{Stealth[length=1.6mm]}, thin}]\n",
    );
    out.push_str(&format!(
        "\\node[draw, rounded corners, minimum width=34mm, \
         minimum height={height}mm, align=center] (u) \
         {{\\code{{{key}}}}};\n"
    ));
    let place = |list: &[(String, Kind, usize)], left: bool| -> String {
        let mut s = String::new();
        let n = list.len();
        for (i, (name, k, w)) in list.iter().enumerate() {
            // Down the side, spread over the box's height.
            let y = (height as f64) / 2.0 - 3.0 - (7.0 * i as f64);
            let (x0, x1, anchor) = if left {
                (-17.0 - 12.0, -17.0, "east")
            } else {
                (17.0 + 12.0, 17.0, "west")
            };
            let _ = n;
            let (from, to) = if left { (x0, x1) } else { (x1, x0) };
            s.push_str(&format!(
                "\\draw[pin] ({from}mm,{y}mm) -- ({to}mm,{y}mm);\n\
                 \\node[anchor={anchor}] at ({x0}mm,{y}mm) {{{}}};\n",
                label(name, *k, *w)
            ));
        }
        s
    };
    out.push_str(&place(&ins, true));
    out.push_str(&place(&outs, false));
    if !pads.is_empty() {
        let names: Vec<String> =
            pads.iter().map(|(n, k, w)| label(n, *k, *w)).collect();
        out.push_str(&format!(
            "\\node[anchor=north, text width=60mm, align=center] at \
             (0mm,{}mm) {{pads, both ways: {}}};\n",
            -(height as f64) / 2.0 - 1.0,
            names.join(", ")
        ));
    }
    out.push_str("\\end{tikzpicture}\n\\end{center}\n");
    out
}

/// How a component is made and joined, from its lowering: the type as
/// this document lowers it, a value of it, and the call that gives its
/// ports to the simulator.
///
/// The ports are named in the order `run` takes them. Where the
/// inputs all come before the outputs, which is this repository's
/// habit, the call is written with the two tuples the unit's `run`
/// has; where they are mixed, the ports are listed instead, since the
/// lowering keeps their order and not their grouping.
fn instance(ty: &str, net: &Lowered) -> String {
    let is_in = |k: &Kind| matches!(k, Kind::In | Kind::Rx);
    let split = net.ports.iter().position(|(_, k, _, _)| !is_in(k));
    let tidy = ty.replace('<', "::<");
    let var = {
        let head: String =
            ty.chars().take_while(|c| c.is_alphanumeric()).collect();
        head.chars()
            .enumerate()
            .flat_map(|(i, c)| {
                if c.is_uppercase() && i > 0 {
                    vec!['_', c.to_ascii_lowercase()]
                } else {
                    vec![c.to_ascii_lowercase()]
                }
            })
            .collect::<String>()
    };
    // An array of ports, `ins: [Rx<T>; N]`, is ports `ins_0` to
    // `ins_{N-1}` in the lowering, in order, and `run` takes it as one
    // array: so a run of two or more ports `base_0`, `base_1`, .. of one
    // kind is written `[base_0, base_1, ..]` (issue 612).
    let names = |r: &[(String, Kind, usize)]| -> String {
        let mut out: Vec<String> = Vec::new();
        let mut i = 0;
        while i < r.len() {
            let (n, k, _) = &r[i];
            let base = n.strip_suffix("_0");
            let mut j = i + 1;
            if let Some(base) = base {
                while j < r.len()
                    && r[j].1 == *k
                    && r[j].0 == format!("{base}_{}", j - i)
                {
                    j += 1;
                }
            }
            if base.is_some() && j - i >= 2 {
                let arr: Vec<String> =
                    r[i..j].iter().map(|(n, _, _)| n.clone()).collect();
                out.push(format!("[{}]", arr.join(", ")));
                i = j;
            } else {
                out.push(n.clone());
                i += 1;
            }
        }
        out.join(", ")
    };
    let all: Vec<(String, Kind, usize)> = net
        .ports
        .iter()
        .map(|(n, k, w, _)| (n.clone(), *k, *w))
        .collect();
    let call = match split {
        Some(i) if all[i..].iter().all(|(_, k, _)| !is_in(k)) => {
            format!("{var}.run(({}), ({}))", names(&all[..i]), names(&all[i..]))
        }
        _ => format!("{var}.run(/* {} */)", names(&all)),
    };
    // Three lines, each a short one, since a sheet is not the place
    // for a wrapped call. `lstlisting` cannot be used here: listings
    // reads its body from the file, and this arrives through a macro.
    let lines = [
        format!("let mut {var} = {tidy}::default();"),
        format!("sim.spawn({call});"),
        format!("let net = {tidy}::lowered(\"{var}\");"),
    ];
    let rows: Vec<String> = lines
        .iter()
        .map(|l| format!("\\code{{{}}} \\\\", tex(l)))
        .collect();
    format!(
        "\\begin{{center}}\\footnotesize\n\\begin{{tabular}}{{@{{}}p{{0.92\\textwidth}}@{{}}}}\n{}\n\\end{{tabular}}\n\\end{{center}}\n",
        rows.join("\n")
    )
}

/// The tables of one component, lowered as `ty` states it.
fn sheet(key: &str, ty: &str, net: Lowered) {
    define("type", key, &format!("\\code{{{}}}", tex(ty)));
    let ports: Vec<String> = net
        .ports
        .iter()
        .map(|(n, k, w, _)| {
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
    define("figure", key, &figure(key, &net));
    define("instance", key, &instance(ty, &net));
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

/// A second clock, so that the crossing can be lowered with the two
/// domains it is for. Its rate is the pixel clock's in `ex_cdc`; the
/// sheet's tables only need the two to be different types.
pub struct ClkPix;
impl Clock for ClkPix {
    const NAME: &'static str = "clk_pix";
    const PERIOD: u64 = 6;
    const PHASE: u64 = 2;
}

/// The map the router's sheet is generated with: three peripherals of
/// a nibble each.
struct SheetMap;

impl AddrMap<3> for SheetMap {
    const RANGES: [(usize, usize); 3] =
        [(0x1000, 0xf000), (0x2000, 0xf000), (0x3000, 0xf000)];
}

fn main() {
    println!("% Generated by //tools/datasheet. Do not edit.");
    sheet("Buffer", "Buffer<8>", Buffer::<8>::lowered("buffer"));
    sheet(
        "ChanCdc",
        "ChanCdc<U<32>, 4, 16, 5, DefaultClock, ClkPix>",
        ChanCdc::<U<32>, 4, 16, 5, DefaultClock, ClkPix>::lowered("chan_cdc"),
    );
    sheet(
        "Fifo",
        "Fifo<U<8>, 2, 4>",
        Fifo::<U<8>, 2, 4>::lowered("fifo"),
    );
    sheet(
        "LineBuf",
        "LineBuf<32, 5, ClkPix>",
        LineBuf::<32, 5, ClkPix>::lowered("linebuf"),
    );
    sheet(
        "LineFetch",
        "LineFetch<32, 2, 16, 16>",
        LineFetch::<32, 2, 16, 16>::lowered("linefetch"),
    );
    sheet(
        "LineStore",
        "LineStore<32, 2, 16, 16>",
        LineStore::<32, 2, 16, 16>::lowered("linestore"),
    );
    sheet("NoBeats", "NoBeats", NoBeats::lowered("nobeats"));
    sheet("NoReads", "NoReads", NoReads::lowered("noreads"));
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
        "Router<3, SheetMap, 32, 32, 4, 2>",
        Router::<3, SheetMap, 32, 32, 4, 2>::lowered("router"),
    );
    sheet(
        "Arbiter",
        "Arbiter2<16, 32, 4, 2, 5, 0>",
        Arbiter2::<16, 32, 4, 2, 5, 0>::lowered("arbiter"),
    );
    sheet(
        "LiteBridge",
        "LiteBridge<1, SerialMap, 32, 32, 4, 2>",
        LiteBridge::<1, SerialMap, 32, 32, 4, 2>::lowered("lite_bridge"),
    );
    sheet(
        "AxiWb",
        "AxiWb<32, 2, 28>",
        AxiWb::<32, 2, { ddr3::AW }>::lowered("axi_wb"),
    );
    sheet(
        "AxiPins",
        "AxiPins<32, 64, 8, 4>",
        AxiPins::<32, 64, 8, 4>::lowered("axi_pins"),
    );
    sheet(
        "Switch",
        "Switch<2, 2, 32, 32, 4, 2>",
        Switch::<2, 2, 32, 32, 4, 2>::lowered("switch"),
    );
    sheet(
        "Node",
        "Node<2, 2, 32, 32, 4, 2>",
        Node::<2, 2, 32, 32, 4, 2>::lowered("node"),
    );
    sheet(
        "Mesh",
        "Mesh<3, 2, 6, 2, 1, 8, 8, 1, 2>",
        Mesh::<3, 2, 6, 2, 1, 8, 8, 1, 2>::lowered("mesh"),
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
        "EthSlots",
        "EthSlots<0x4100_0000>",
        EthSlots::<0x4100_0000>::lowered("ethslots"),
    );
    sheet("FrameOut", "FrameOut", FrameOut::lowered("frame_out"));
    sheet("FrameIn", "FrameIn", FrameIn::lowered("frame_in"));
    sheet("FrameLen", "FrameLen", FrameLen::lowered("framelen"));
    sheet(
        "EthShare",
        "EthShare<0x88b5>",
        EthShare::<0x88b5>::lowered("ethshare"),
    );
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
    sheet("Watch", "Watch", Watch::lowered("watch"));
    sheet("Inject", "Inject", Inject::lowered("inject"));
    sheet("Pair", "Pair<2>", Pair::<2>::lowered("pair"));
    sheet("Vreteno", "Vreteno<2>", Vreteno::<2>::lowered("vreteno"));
    sheet("Dmem", "Dmem<4>", Dmem::<4>::lowered("dmem"));
    sheet("Rom", "Rom<4>", Rom::<4>::lowered("rom"));
    sheet("Timer", "Timer<4>", Timer::<4>::lowered("timer"));
    sheet("Plic", "Plic3<0>", Plic3::<0>::lowered("plic"));
    sheet("Dm", "Dm", Dm::lowered("dm"));
    sheet("Gpio", "Gpio<8>", Gpio::<8>::lowered("gpio"));
    sheet("Pwm", "Pwm", Pwm::lowered("pwm"));
    sheet("Tee", "Tee<R<32, 2>>", Tee::<R<32, 2>>::lowered("tee"));
    sheet(
        "Check",
        "Check<Issue<32>>",
        Check::<Issue<32>>::lowered("check"),
    );
    sheet(
        "FlashWin",
        "FlashWin<1, 2>",
        FlashWin::<1, 2>::lowered("flashwin"),
    );
    sheet("Remote", "Remote<40>", Remote::<40>::lowered("remote"));
    sheet(
        "RemoteLink",
        "RemoteLink<3>",
        RemoteLink::<3>::lowered("remote_link"),
    );
    sheet("Spi", "Spi", Spi::lowered("spi"));
    sheet("Tracer", "Tracer<8>", Tracer::<8>::lowered("tracer"));
    sheet("I2c", "I2c", I2c::lowered("i2c"));
    sheet("Sd", "Sd", Sd::lowered("sd"));
    sheet(
        "Wdog",
        "Wdog<0x57444f47>",
        Wdog::<0x5744_4f47>::lowered("wdog"),
    );
    sheet(
        "Syscon",
        "Syscon<0x74780001, 0x10000, 0, 0, 0x52535421>",
        Syscon::<0x7478_0001, 0x1_0000, 0, 0, 0x5253_5421>::lowered("syscon"),
    );
    sheet("Uart", "Uart<868>", Uart::<868>::lowered("uart"));
    sheet("Ddr3Per", "Ddr3Per", Ddr3Per::lowered("ddr3"));
    regs("Sd", &txhdl_parts::sd::regs::MAP);
    sheet("Entropy", "Entropy", Entropy::lowered("entropy"));
    sheet("Trng", "Trng", Trng::lowered("trng"));
    sheet("Board", "Board<868>", Board::<868>::lowered("board"));
    sheet("BarRegs", "BarRegs<4>", BarRegs::<4>::lowered("bar_regs"));
    sheet("PcieBar", "PcieBar", PcieBar::lowered("pcie_bar"));
    sheet("Fb", "Fb<16, 2, 1024>", Fb::<16, 2, 1024>::lowered("fb"));
    sheet(
        "Raster",
        "Raster<16, 2, 4, 16, 0, 0x400, 0x600>",
        Raster::<16, 2, 4, 16, 0, 0x400, 0x600>::lowered("raster"),
    );
}
