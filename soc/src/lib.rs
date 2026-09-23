// SPDX-License-Identifier: Apache-2.0
//! A system on one lattice: Vreteno, Razboj the GPU, a memory and a
//! serial port, each on a corner of a two by two network.
//!
//! ```text
//!     (0,0) Vreteno ------ (1,0) Razboj
//!        |                      |
//!     (0,1) memory ------- (1,1) serial port
//! ```
//!
//! Two hosts and two peripherals. The core runs a program and says
//! something on the serial port; the rasteriser draws into the same
//! memory the core is using, at a base of its own. Neither knows the
//! other is there, and neither knows the network is there: each holds
//! the channel ends of an AXI link, and a bridge at its corner turns
//! those into packets.
//!
//! The memory is the simulation-only [`Ram`], which is what a design
//! reaches for when the point is the rest of the system, and it is
//! also what the two halves talk through. The core computes a display
//! list and writes it at [`DL_BASE`], with the count at [`DL_CTRL`]
//! last; the rasteriser has been reading that count since the first
//! cycle, sees it turn non-zero, fetches the list and fills every
//! triangle in it into the framebuffer at [`FB_BASE`]. Neither waits
//! on the other through anything but the memory they share.
use razboj::op::Insn;
use razboj::raster::Raster;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, join_all, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::sim::Ram;
use txhdl_parts::bus::axi::{axi, axi_units, AxiHost, AxiPer, Link};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1};
use txhdl_parts::bus::noc::bridge::{HostBridge, PerBridge};
use txhdl_parts::bus::noc::mesh::lattice;
use txhdl_parts::bus::noc::node::Node;
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::term::Terminal;
use vreteno32::uart::Uart;

/// The link every corner speaks: thirty-two bit addresses and words,
/// four lanes, two-bit identifiers, four of them.
pub const IW: usize = 2;
/// How many identifiers a tracker and a peripheral bridge hand out.
pub const NIDS: usize = 4;
/// Coordinates are two bits, which is more than a two by two needs.
pub const XB: usize = 2;
/// The same for a row.
pub const YB: usize = 2;
/// The screen is `1 << LOGW` by [`H`] pixels.
pub const LOGW: usize = 7;
/// Rows of the screen.
pub const H: usize = 96;
/// Where the program's constants go, and where its stack ends: the
/// four kilobytes the linker script gives the data memory.
pub const DATA_BASE: usize = 0x1000;
/// Where the display list sits, above the program's data. There is
/// room for the hundred and twenty-eight instructions that reach as
/// far as the serial port, which is the next thing in the map.
pub const DL_BASE: usize = 0x2000;
/// Where its count sits, past the serial port's four words at
/// `0x3000`. The program writes it last, and that is what tells the
/// rasteriser the list is ready.
pub const DL_CTRL: usize = 0x4000;
/// Where the framebuffer's first word sits, clear of all of it.
pub const FB_BASE: usize = 0x8000;
/// Words of memory: as far as the last word of the framebuffer.
pub const RAM_WORDS: usize = FB_BASE / 4 + (1 << LOGW) * H;

// begin{map}
/// A host bridge at `X`, `Y` with the map every host here uses: the
/// serial port at 1,1, the program's data at 0,1, and everything
/// else, the framebuffer included, at 0,1 as well.
///
/// The masks reach the whole of the address and not only the digit
/// the base is written in. A mask of `0xf000` would send the
/// framebuffer's own `0x13000` to the serial port, which is a way of
/// writing a picture into a terminal and nothing else.
pub type Bridge<const X: usize, const Y: usize> = HostBridge<
    X,
    Y,
    XB,
    YB,
    32,
    32,
    4,
    IW,
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
>;

// end{map}

/// What a run came to.
pub struct Ran {
    /// What a terminal on the serial line heard.
    pub said: String,
    /// The cycle the core halted itself on, if it did.
    pub halted_at: Option<u64>,
    /// The cycle the count of the display list was first not zero in
    /// the memory, which is the cycle the core finished writing it.
    pub listed_at: Option<u64>,
    /// The cycle the rasteriser had nothing left to draw and nothing
    /// left in flight, if it reached it.
    pub drawn_at: Option<u64>,
    /// The framebuffer, read out of the memory afterwards.
    pub fb: Vec<u32>,
    /// The display list the program wrote, read back out of the
    /// memory and decoded. It is what the core asked for, so a model
    /// rendered from it says what the framebuffer should hold.
    pub list: Vec<Insn>,
    /// Cycles the run took.
    pub cycles: u64,
}

/// Run `text` on the core with `data` in the memory, for at most
/// `limit` cycles. The scene is the program's: nothing is put in the
/// memory here but the constants the program was linked with.
#[allow(clippy::too_many_lines)]
pub fn run(text: &[u32], data: &[u8], limit: u64) -> Ran {
    let width = 1usize << LOGW;

    // The lattice, and the four nodes on it, row major.
    let mut net = lattice::<XB, YB, 32, 32, 4, IW>(2, 2);
    let mut n00 = Node::<0, 0, XB, YB, 32, 32, 4, IW>::default();
    let mut n10 = Node::<1, 0, XB, YB, 32, 32, 4, IW>::default();
    let mut n01 = Node::<0, 1, XB, YB, 32, 32, 4, IW>::default();
    let mut n11 = Node::<1, 1, XB, YB, 32, 32, 4, IW>::default();
    let e00 = net.exits.remove(0);
    let e10 = net.exits.remove(0);
    let e01 = net.exits.remove(0);
    let e11 = net.exits.remove(0);
    let nodes = join_all(vec![
        Box::pin(n00.run(net.ins.remove(0), net.outs.remove(0)))
            as std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>,
        Box::pin(n10.run(net.ins.remove(0), net.outs.remove(0))),
        Box::pin(n01.run(net.ins.remove(0), net.outs.remove(0))),
        Box::pin(n11.run(net.ins.remove(0), net.outs.remove(0))),
    ]);

    // 0,0: the core, its tracker and its bridge.
    let cl = axi_units::<32, 32, 4, IW>();
    let (issue, wbeat, release, grant, cdone, crdata) = cl.host_client;
    let mut ctrk = AxiHost::<32, 32, 4, IW, NIDS>::default();
    let mut cbr = Bridge::<0, 0>::default();
    let mut cpu = Vreteno::<IW>::with(text);
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();
    let (tirq_out, tirq) = signal::<Bit, DefaultClock>();
    // The system has no interrupt controller, so nothing raises a
    // software interrupt in it.
    let (_sirq_o, sirq) = signal::<Bit, DefaultClock>();
    let (halt_out, halt) = signal::<Bit, DefaultClock>();
    // No debugger here: its request lines stay low and what the
    // core says about debug mode is not read (issue 154).
    let (_haltreq_o, haltreq) = signal::<Bit, DefaultClock>();
    let (_resumereq_o, resumereq) = signal::<Bit, DefaultClock>();
    let (debug_o, _debug) = signal::<Bit, DefaultClock>();
    // The debug module's register access, absent here: the number and
    // the word stay zero, the write never comes, the answer is unread.
    let (_dbg_regno_o, dbg_regno) = signal::<U<16>, DefaultClock>();
    let (_dbg_wdata_o, dbg_wdata) = signal::<U<32>, DefaultClock>();
    let (_dbg_we_o, dbg_we) = signal::<Bit, DefaultClock>();
    let (dbg_rdata_o, _dbg_rdata) = signal::<U<32>, DefaultClock>();
    let (instr_out, _instr) = signal::<U<32>, DefaultClock>();
    let (wb_out, _wb) = signal::<Writeback, DefaultClock>();

    // 1,0: the rasteriser, its tracker and its bridge.
    let gl = axi_units::<32, 32, 4, IW>();
    let (gissue, gwbeat, grelease, ggrant, gdone, grdata) = gl.host_client;
    let mut gtrk = AxiHost::<32, 32, 4, IW, NIDS>::default();
    let mut gbr = Bridge::<1, 0>::default();
    let mut raster =
        Raster::<32, IW, LOGW, H, FB_BASE, DL_BASE, DL_CTRL>::default();
    let (idle_out, idle) = signal::<Bit, DefaultClock>();

    // 0,1: the memory, behind a tracker and a bridge. It takes the
    // link that carries a client end, since the memory is a program
    // and not a unit.
    let Link {
        per: ram_end,
        per_in: rp_in,
        per_out: rp_out,
        host_in: rh_in,
        host_out: rh_out,
        ..
    } = axi::<32, 32, 4, IW, NIDS>();
    let mut rtrk = AxiPer::<32, 32, 4, IW>::default();
    let mut rbr = PerBridge::<0, 1, XB, YB, 32, 32, 4, IW, NIDS>::default();
    let ram = Ram::<32, 32, 4, IW>::new(RAM_WORDS);
    // The constants the program reads, in the memory before the first
    // cycle, at the base the core's decode sends a load to.
    let words: Vec<U<32>> = data
        .chunks(4)
        .map(|c| {
            let mut w = 0u32;
            for (i, b) in c.iter().enumerate() {
                w |= (*b as u32) << (8 * i);
            }
            U::from(w)
        })
        .collect();
    ram.load(DATA_BASE / 4, &words);
    // The program's own words again, from address zero: the boot
    // memory is on the bus read-only on the board (issue 268), so a
    // program's constants sit beside its code, and a load of them
    // reaches this memory here, which the bridge's default route sends
    // every unmapped address to. Writable here, where nothing writes it.
    let boot: Vec<U<32>> = text.iter().map(|&w| U::from(w)).collect();
    ram.load(0, &boot);

    // 1,1: the serial port, behind a network bridge and an AXI-Lite
    // bridge.
    let ul = axi_units::<32, 32, 4, IW>();
    // The serial port is an AXI-Lite peripheral, behind a bridge
    // from its corner's AXI4 link.
    let sl = axi_lite::<32, 32, 4>();
    let (uaw, uar, uw, ub, ur) = sl.per;
    let (baw, bar, bw, bb, br) = sl.host;
    let mut ubridge = LiteBridge1::<32, 32, 4, IW, 0x3000, 0xf000>::default();
    let mut ubr = PerBridge::<1, 1, XB, YB, 32, 32, 4, IW, NIDS>::default();
    let mut uart = Uart::<4>::default();
    let (tx_out, tx) = signal::<Bit, DefaultClock>();
    let (rx_out, rx) = signal::<Bit, DefaultClock>();
    let (uirq_out, _uirq) = signal::<Bit, DefaultClock>();

    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("rst", &rst);
        w.add("tx", &tx);
        w.add("halt", &halt);
        w.add("idle", &idle);
        w.add("cpu", &cpu);
        w.add("raster", &raster);
        w.add("uart", &uart);
        w.start();
    }

    let (rst_u, rst_c) = (rst.clone(), rst.clone());
    let corners = join2(
        join2(
            join2(
                ctrk.run(cl.host_in, cl.host_out),
                cbr.run(
                    (cl.per_in.0, cl.per_in.1, cl.per_in.2, e00.p_out),
                    (e00.q_in, cl.per_out.2, cl.per_out.3),
                ),
            ),
            join2(
                gtrk.run(gl.host_in, gl.host_out),
                gbr.run(
                    (gl.per_in.0, gl.per_in.1, gl.per_in.2, e10.p_out),
                    (e10.q_in, gl.per_out.2, gl.per_out.3),
                ),
            ),
        ),
        join2(
            join2(
                rtrk.run(rp_in, rp_out),
                rbr.run(
                    (e01.q_out, rh_in.2, rh_in.3),
                    (rh_out.0, rh_out.1, rh_out.2, e01.p_in),
                ),
            ),
            join2(
                ubridge.run(
                    (ul.per_in.0, ul.per_in.1, ul.per_in.2, bb, br),
                    (baw, bar, bw, ul.per_out.2, ul.per_out.3),
                ),
                ubr.run(
                    (e11.q_out, ul.host_in.2, ul.host_in.3),
                    (ul.host_out.0, ul.host_out.1, ul.host_out.2, e11.p_in),
                ),
            ),
        ),
    );
    let ends = join2(
        join2(
            cpu.run(
                (
                    rst_c, irq, tirq, sirq, crdata, cdone, grant, haltreq,
                    resumereq, dbg_regno, dbg_wdata, dbg_we,
                ),
                (
                    halt_out,
                    instr_out,
                    wb_out,
                    issue,
                    wbeat,
                    release,
                    debug_o,
                    dbg_rdata_o,
                ),
            ),
            raster.run(
                (ggrant, gdone, grdata),
                (gissue, gwbeat, grelease, idle_out),
            ),
        ),
        join2(
            uart.run((rst_u, rx, uaw, uar, uw), (ub, ur, tx_out, uirq_out)),
            ram.clone().serve(ram_end, 4),
        ),
    );
    let mut sim = Running::new(join2(join2(nodes, corners), ends));

    rst_out.set(Bit::One);
    sim.cycle();
    rst_out.set(Bit::Zero);
    irq_out.set(Bit::Zero);
    tirq_out.set(Bit::Zero);
    rx_out.set(Bit::One);
    let mut term = Terminal::new(b"");
    let mut halted_at = None;
    let mut listed_at = None;
    let mut drawn_at = None;
    let mut cycles = 0u64;
    for c in 0..limit {
        sim.cycle();
        cycles = c + 1;
        term.see(tx.get().to_bool());
        rx_out.set(Bit::from_bool(term.level()));
        if halted_at.is_none() && halt.get().to_bool() {
            halted_at = Some(c);
        }
        if listed_at.is_none() && ram.word(DL_CTRL / 4).raw() != 0 {
            listed_at = Some(c);
        }
        if drawn_at.is_none() && idle.get().to_bool() {
            drawn_at = Some(c);
        }
        if halted_at.is_some() && drawn_at.is_some() {
            break;
        }
    }
    // The core stops the moment it writes `mhalt` and the last byte
    // is still going out a bit at a time, so the line is read to the
    // end of the frame before the run is judged.
    for _ in 0..64 {
        sim.cycle();
        term.see(tx.get().to_bool());
    }
    stop();
    // What the program asked for, read back out of the memory the way
    // the rasteriser read it.
    let count = ram.word(DL_CTRL / 4).raw() as usize;
    let list = (0..count)
        .map(|i| {
            let at = DL_BASE / 4 + i * razboj::dl::WORDS;
            let words: Vec<u32> = (0..razboj::dl::USED)
                .map(|k| ram.word(at + k).raw() as u32)
                .collect();
            razboj::dl::decode(&words)
        })
        .collect();
    Ran {
        said: term.said.clone(),
        halted_at,
        listed_at,
        drawn_at,
        fb: (0..width * H)
            .map(|i| ram.word(FB_BASE / 4 + i).raw() as u32)
            .collect(),
        list,
        cycles,
    }
}
