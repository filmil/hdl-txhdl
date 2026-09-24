// SPDX-License-Identifier: Apache-2.0
//! A scanout path, end to end and across two clocks.
//!
//! Memory to pixels, with nothing written by hand in between:
//!
//! ```text
//!   Rom -- AxiPer -- AxiHost -- LineFetch ]--- ChanCdc ---[ LineBuf
//!   <------------- bus clock ------------->     ><      <- pixel clock ->
//! ```
//!
//! `LineFetch` reads a line from memory in bursts on the bus clock,
//! `ChanCdc` carries the words to the pixel clock, and `LineBuf`
//! holds them for the raster to read a column at a time. All three
//! are parts in `//lib/parts` and all three are lowered; the wiring
//! here is the only thing that is not hardware.
//!
//! The two clocks are deliberately unrelated: the bus at period four
//! and the pixel clock at period six with a phase of two, which is
//! roughly the ratio a real design has and, more to the point, is not
//! a ratio that hides a crossing fault by keeping the edges in step.
//!
//! What this asserts is that the pixels come out of the far end in
//! order and with the values that were in memory. A word that crossed
//! badly, a burst that landed at the wrong address, or a line buffer
//! that filled out of step would all show up as the wrong number at
//! the wrong column, which is what the check at the end reads.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, signal, Clock, DefaultClock, Mem, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{
    axi_units, Answer, AxiHost, AxiPer, PerPort, Resp, R,
};
use txhdl_parts::cdc::ChanCdc;
use txhdl_parts::dma::{LineBuf, LineFetch};

const ADDR: usize = 32;
const IDB: usize = 2;
const IDS: usize = 4;
/// A line, and the memory behind it.
const LINE: usize = 32;
const LINE_AW: usize = 5;
const BEATS: usize = 16;
const N: usize = 256;

/// The bus clock.
pub struct ClkBus;
impl Clock for ClkBus {
    const NAME: &'static str = "clk_bus";
    const PERIOD: u64 = 4;
}

/// The pixel clock, unrelated to the bus clock on purpose.
pub struct ClkPix;
impl Clock for ClkPix {
    const NAME: &'static str = "clk_pix";
    const PERIOD: u64 = 6;
    const PHASE: u64 = 2;
}

/// The memory the line is read from. A burst arrives as one request
/// carrying `len` and is owed `len + 1` beats, the last marked.
#[derive(Trace, Default)]
pub struct Rom<const A: usize, const I: usize, const M: usize> {
    pub px: Mem<U<32>, M>,
    pub busy: Reg<Bit>,
    pub at: Reg<U<16>>,
    pub left: Reg<U<9>>,
    pub rid: Reg<U<I>>,
}

#[lower]
impl<const A: usize, const I: usize, const M: usize> Unit for Rom<A, I, M> {
    async fn run(&mut self, bus: PerPort<A, 32, 4, I>, _out: ()) {
        loop {
            DefaultClock::rising().await;
            let q = bus.req.head();
            let offered = Bit::from(bus.req.peek().is_some());
            let busy = self.busy.get();
            let mask = U::<16>::from((M - 1) as u32);
            let qat = (q.addr >> 2u32).resize::<16>() & mask;
            let take = offered & q.read & !busy;
            let write = offered & !q.read & bus.ans.ready();
            let _ = bus.req.recv_if(take | write);
            let _ = bus.w.recv_if(write);
            let beat = busy & bus.r.ready();
            let last = beat & (self.left.get() == 1);
            with!(self <= {
                take ? {
                    busy: Bit::One,
                    at: qat,
                    left: (q.len.resize::<9>() + 1),
                    rid: q.id,
                },
                beat ? {
                    at: (self.at.get() + 1) & mask,
                    left: self.left.get() - 1,
                },
                last ? busy: Bit::Zero,
            });
            if beat.to_bool() {
                bus.r.send(R {
                    id: self.rid.get(),
                    data: self.px.read(self.at.get().slice::<0, 8>()),
                    resp: Resp::Okay,
                    last,
                });
            }
            if write.to_bool() {
                bus.ans.send(Answer {
                    id: q.id,
                    resp: Resp::Okay,
                });
            }
        }
    }
}

fn main() {
    let link = axi_units::<ADDR, 32, 4, IDB>();
    let (issue, wbeat, release, grant, done, rdata) = link.host_client;
    let bus: PerPort<ADDR, 32, 4, IDB> = link.per_client.into();
    let (base_o, base) = signal::<U<ADDR>, DefaultClock>();
    let (words_o, words) = signal::<U<16>, DefaultClock>();
    let (go_o, go) = signal::<Bit, DefaultClock>();
    let (run_o, running) = signal::<Bit, DefaultClock>();
    let (col_o, col) = signal::<U<LINE_AW>, ClkPix>();
    let (sol_o, sol) = signal::<Bit, ClkPix>();
    let (pix_o, pix) = signal::<U<32>, ClkPix>();
    // The bus side of the crossing, and the pixel side.
    let (fetched_tx, fetched_rx) = chan::<U<32>, DefaultClock>();
    let (crossed_tx, crossed_rx) = chan::<U<32>, ClkPix>();

    // Each word is its own address, so a pixel that came from the
    // wrong place says which place.
    let image: Vec<U<32>> = (0..N).map(|i| U::<32>::from(i as u32)).collect();
    let mut rom = Rom::<ADDR, IDB, N> {
        px: Mem::with(&image),
        ..Default::default()
    };
    let mut host = AxiHost::<ADDR, 32, 4, IDB, IDS>::default();
    let mut per = AxiPer::<ADDR, 32, 4, IDB>::default();
    let mut fetch = LineFetch::<ADDR, IDB, BEATS, 16>::default();
    let mut cross = ChanCdc::<U<32>, 4, 16, 5, DefaultClock, ClkPix>::default();
    let mut line = LineBuf::<LINE, LINE_AW, ClkPix>::default();

    // The far end is the unit under check here: its ports under
    // their own names, and the unit under the entity's name, which is
    // what the testbench generator reads the trace by. `LineFetch`
    // has its own co-simulation in `ex_dma`.
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.clock::<ClkPix>();
        w.add("inp", &crossed_rx);
        w.add("col", &col);
        w.add("sol", &sol);
        w.add("pix", &pix);
        w.add("linebuf", &line);
        w.start();
    }

    base_o.set(U::<ADDR>::from(0u32));
    words_o.set(U::<16>::from(LINE as u32));
    go_o.set(Bit::One);
    sol_o.set(Bit::Zero);
    col_o.set(U::<LINE_AW>::from(0u8));

    let mut sim = Running::new(join2(
        join2(
            join2(
                host.run(link.host_in, link.host_out),
                per.run(link.per_in, link.per_out),
            ),
            rom.run(bus, ()),
        ),
        join2(
            fetch.run(
                (grant, done, rdata, base, words, go),
                (issue, release, fetched_tx, run_o),
            ),
            join2(
                cross.run(fetched_rx, crossed_tx),
                line.run((crossed_rx, col, sol), pix_o),
            ),
        ),
    ));
    let _ = wbeat;
    let _ = running;

    // Let the line fill. The fetch is told to stop asking once the
    // line is in flight, so the run settles rather than looping.
    let start = now();
    while now() - start < 1200 {
        sim.cycle();
    }
    go_o.set(Bit::Zero);
    while now() - start < 1600 {
        sim.cycle();
    }

    // Read the line back a column at a time, on the pixel clock, and
    // check each against the memory it came from.
    let mut got: Vec<u128> = Vec::new();
    for c in 0..LINE {
        col_o.set(U::<LINE_AW>::from(c as u32));
        // Two pixel-clock edges: one for the column to be applied,
        // one for the pixel it names to come out.
        for _ in 0..12 {
            sim.cycle();
        }
        got.push(pix.get().raw());
    }
    for (c, v) in got.iter().enumerate() {
        println!("column {c:2} is {v}");
    }

    assert_eq!(got.len(), LINE, "a pixel per column");
    for (c, v) in got.iter().enumerate() {
        assert_eq!(
            *v, c as u128,
            "column {c} holds the word at address {c}, not {v}"
        );
    }
    println!("the line crossed intact: {LINE} pixels");

    print!("\n{}", LineBuf::<LINE, LINE_AW, ClkPix>::verilog("linebuf"));
    stop();
    txhdl::netlist::write_vhdl_from_env(
        &LineBuf::<LINE, LINE_AW, ClkPix>::lowered("linebuf"),
    );
}
