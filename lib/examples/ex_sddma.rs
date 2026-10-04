// SPDX-License-Identifier: Apache-2.0
//! The SD card host moving blocks through memory, by the engines of
//! issue 151, with the card sending its blocks back to back
//! (issue 912).
//!
//! ```text
//!   client -- LiteBridge -- Sd -- card model
//!                           | dma_out      ^ dma_in
//!                       LineStore      LineFetch
//!                           |              |
//!                       AxiHost        AxiHost
//!                            \            /
//!                             Arbiter2 -- AxiPer -- Ram
//! ```
//!
//! A program writes where in memory the blocks are and how many, then
//! starts the command as it would for one block. The host runs the
//! blocks on its own: for a write it takes each block's words from the
//! fetch engine into its buffer, now a ring, and sends the block once
//! all of it is in; for a read it hands each word to the store engine
//! as the card sends it. The command is done only once the engine has
//! finished too, so a program waits as it waits for any command.
//!
//! The card here sends a read's blocks a clock apart, where a program
//! taking each word through a register could not keep up. The run
//! brings the card up on four lines, writes two blocks from memory to
//! blocks 4 and 5 with `CMD25`, reads them back into another part of
//! memory with `CMD18`, and checks the card's blocks and the memory.
//! The two engines share one way onto the bus, and never run at once,
//! which the run checks on every cycle. The host is lowered and its
//! netlist simulated against this run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, mux, now, signal, Clock, DefaultClock, Mem, Reg, Running, Unit,
};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::arbiter::Arbiter2;
use txhdl_parts::bus::axi::{
    axi, axi_units, Answer, AxiHost, AxiPer, Link, PerPort, Rd, Resp, Wr, R,
};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge, LitePort};
use txhdl_parts::dma::{LineFetch, LineStore, NoBeats, NoReads};
use txhdl_parts::sd::{
    regs, Sd, SdCard, SdLines, CMD_BUSY, CMD_CLOCKS, CMD_LONG, CMD_NOCRC,
    CMD_READ, CMD_SHORT, CMD_WRITE, CTRL_WIDE, STATUS_DCRC, STATUS_DONE,
    STATUS_DTIMEOUT, STATUS_RCRC, STATUS_RTIMEOUT, WORDS,
};

/// The client's link to the host's registers.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;
type Bridge = LiteBridge<1, SdMap, 32, 32, 4, 2>;

/// The host's registers at 0x1000.
pub struct SdMap;

impl AddrMap<1> for SdMap {
    const RANGES: [(usize, usize); 1] = [(0x1000, 0xf000)];
}

const BASE: u32 = 0x1000;
/// A half of a card clock in two cycles: 25 MHz from 100 MHz.
const DIV: u32 = 1;
/// The memory: 512 words. The blocks to write are in its upper half,
/// and the blocks read land in its lower half.
const M: usize = 512;
const FROM: u32 = 0x400;
const INTO: u32 = 0x000;
/// Blocks moved each way, and the first of them on the card.
const BLOCKS: u32 = 2;
const FIRST: u32 = 4;

/// A memory that takes writes as well as reads, as `ex_dmaw` has it,
/// with nine bits of word address.
#[derive(Trace, Default)]
pub struct Ram<const I: usize> {
    pub px: Mem<U<32>, M>,
    pub busy: Reg<Bit>,
    pub wr: Reg<Bit>,
    pub at: Reg<U<16>>,
    pub left: Reg<U<9>>,
    pub rid: Reg<U<I>>,
}

#[lower]
impl<const I: usize> Unit for Ram<I> {
    async fn run(&mut self, bus: PerPort<32, 32, 4, I>, _out: ()) {
        loop {
            DefaultClock::rising().await;
            let q = bus.req.head();
            let offered = Bit::from(bus.req.peek().is_some());
            let busy = self.busy.get();
            let wr = self.wr.get();
            let mask = U::<16>::from((M - 1) as u32);
            let qat = (q.addr >> 2u32).resize::<16>() & mask;
            let take = offered & !busy;
            let _ = bus.req.recv_if(take);
            let rbeat = busy & !wr & bus.r.ready();
            let wbeat = busy & wr & Bit::from(bus.w.peek().is_some());
            let word = bus.w.recv_if(busy & wr).unwrap_or_default();
            let slot = self.at.get().slice::<0, 9>();
            let at_last = self.left.get() == 1;
            let rlast = rbeat & at_last;
            let wlast = wbeat & at_last & bus.ans.ready();
            with!(self <= {
                take ? {
                    busy: Bit::One,
                    wr: !q.read,
                    at: qat,
                    left: (q.len.resize::<9>() + 1),
                    rid: q.id,
                },
                rbeat ? {
                    at: (self.at.get() + 1) & mask,
                    left: self.left.get() - 1,
                },
                wbeat ? {
                    px.at(slot): mux(word.strb == 0xf, word.data, self.px.read(slot)),
                    at: (self.at.get() + 1) & mask,
                    left: self.left.get() - 1,
                },
                rlast ? busy: Bit::Zero,
                wlast ? busy: Bit::Zero,
            });
            if rbeat.to_bool() {
                bus.r.send(R {
                    id: self.rid.get(),
                    data: self.px.read(slot),
                    resp: Resp::Okay,
                    last: rlast,
                });
            }
            if wlast.to_bool() {
                bus.ans.send(Answer {
                    id: self.rid.get(),
                    resp: Resp::Okay,
                });
            }
        }
    }
}

fn main() {
    // The client's way to the host's registers.
    let Link {
        host,
        host_in,
        host_out,
        per_in,
        per_out,
        ..
    } = axi::<32, 32, 4, 2, 4>();
    let (aw, ar, w, _, _) = per_in;
    let (_, _, b, r) = per_out;
    let lite = axi_lite::<32, 32, 4>();
    let (law, lar, lw, lb, lr) = lite.host;
    let bus: LitePort<32, 32, 4> = lite.per.into();
    // The card's lines.
    let (cmd_in_o, cmd_in) = signal::<Bit, DefaultClock>();
    let (dat_in_o, dat_in) = signal::<U<4>, DefaultClock>();
    let (sclk_o, sclk) = signal::<Bit, DefaultClock>();
    let (cmd_out_o, cmd_out) = signal::<Bit, DefaultClock>();
    let (cmd_oe_o, cmd_oe) = signal::<Bit, DefaultClock>();
    let (dat_out_o, dat_out) = signal::<U<4>, DefaultClock>();
    let (dat_oe_o, dat_oe) = signal::<Bit, DefaultClock>();
    let (irq_o, irq) = signal::<Bit, DefaultClock>();
    // The host's way to memory.
    let (dma_in_tx, dma_in) = chan::<U<32>, DefaultClock>();
    let (dma_out, dma_out_rx) = chan::<U<32>, DefaultClock>();
    let (dma_at_o, dma_at) = signal::<U<32>, DefaultClock>();
    let dma_at_f = dma_at.clone();
    let (dma_bytes_o, dma_bytes) = signal::<U<16>, DefaultClock>();
    let (dma_words_o, dma_words) = signal::<U<16>, DefaultClock>();
    let (store_go_o, store_go) = signal::<Bit, DefaultClock>();
    let (fetch_go_o, fetch_go) = signal::<Bit, DefaultClock>();
    let (store_busy_o, store_busy) = signal::<Bit, DefaultClock>();
    let (fetch_busy_o, fetch_busy) = signal::<Bit, DefaultClock>();
    let store_seen = store_busy.clone();
    let fetch_seen = fetch_busy.clone();
    // Each engine's host, onto the arbiter, onto the memory.
    let sl = axi_units::<32, 32, 4, 1>();
    let fl = axi_units::<32, 32, 4, 1>();
    let ml = axi_units::<32, 32, 4, 2>();
    let (s_issue, s_wbeat, s_release, s_grant, s_done, s_rdata) =
        sl.host_client;
    let (f_issue, f_wbeat, f_release, f_grant, f_done, f_rdata) =
        fl.host_client;
    // Each engine's link ends at the arbiter rather than at a
    // peripheral: the arbiter takes the addresses and words a
    // peripheral tracker would, and answers on its write responses and
    // read beats.
    let (s_aw, s_ar, s_w, _, _) = sl.per_in;
    let (_, _, s_b, s_r) = sl.per_out;
    let (f_aw, f_ar, f_w, _, _) = fl.per_in;
    let (_, _, f_b, f_r) = fl.per_out;
    // And the memory's link starts at it, in a host tracker's place.
    let (m_aw, m_ar, m_w, _, _, _) = ml.host_out;
    let (_, _, m_b, m_r, _) = ml.host_in;
    let mem_bus: PerPort<32, 32, 4, 2> = ml.per_client.into();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut sd = Sd::default();
    let mut store = LineStore::<32, 1, 16, 16>::default();
    let mut fetch = LineFetch::<32, 1, 16, 16>::default();
    let mut shost = AxiHost::<32, 32, 4, 1, 2>::default();
    let mut fhost = AxiHost::<32, 32, 4, 1, 2>::default();
    let mut noreads = NoReads::<1>::default();
    let mut nobeats = NoBeats::default();
    let mut arb = Arbiter2::<32, 32, 4, 1, 2, 0>::default();
    let mut per = AxiPer::<32, 32, 4, 2>::default();
    // The words to write, in the memory's upper half.
    let image: Vec<U<32>> = (0..M as u32)
        .map(|i| {
            if i >= FROM / 4 {
                U::<32>::from(i.wrapping_mul(0x9e37_79b9) ^ 0xa5a5_0000)
            } else {
                U::<32>::from(0u8)
            }
        })
        .collect();
    let mut ram = Ram::<2> {
        px: Mem::with(&image),
        ..Default::default()
    };
    let mem = ram.px.clone();
    let mut card = SdCard::default();
    // The card's blocks a clock apart, back to back.
    card.multi_gap = 1;

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("cmd_in", &cmd_in);
        wave.add("dat_in", &dat_in);
        wave.add("sclk", &sclk);
        wave.add("cmd_out", &cmd_out);
        wave.add("cmd_oe", &cmd_oe);
        wave.add("dat_out", &dat_out);
        wave.add("dat_oe", &dat_oe);
        wave.add("irq", &irq);
        wave.add("dma_in", &dma_in);
        wave.add("dma_out", &dma_out);
        wave.add("dma_at", &dma_at);
        wave.add("dma_bytes", &dma_bytes);
        wave.add("dma_words", &dma_words);
        wave.add("store_go", &store_go);
        wave.add("fetch_go", &fetch_go);
        wave.add("store_busy", &store_busy);
        wave.add("fetch_busy", &fetch_busy);
        wave.add("sd", &sd);
        wave.start();
    }

    let finished = std::rc::Rc::new(std::cell::Cell::new(false));
    let fin = finished.clone();
    let client = async move {
        let at = |off: u32| BASE + off;
        let get = |off: u32| {
            let h = &host;
            async move {
                let got = h.read(Rd::at(at(off), 1)).await.done().await;
                got.data[0].raw() as u32
            }
        };
        let put = |off: u32, v: u32| {
            let h = &host;
            async move {
                let v = [U::<32>::from(v)];
                let ok = h.write(Wr::at(at(off)), &v).await.done().await;
                assert_eq!(ok.resp, Resp::Okay, "the write was answered");
            }
        };
        let command = |index: u32, arg: u32, flags: u32| async move {
            put(regs::arg, arg).await;
            put(regs::cmd, index | flags).await;
            let s = loop {
                let s = get(regs::status).await;
                if s & STATUS_DONE != 0 {
                    break s;
                }
            };
            put(regs::status, STATUS_DONE).await;
            let faults =
                STATUS_RTIMEOUT | STATUS_RCRC | STATUS_DTIMEOUT | STATUS_DCRC;
            assert_eq!(s & faults, 0, "command {index}: status {s:#x}");
            s
        };
        // The card up on four lines, as `ex_sd` brings it up.
        put(regs::ctrl, DIV).await;
        command(0, 0, CMD_CLOCKS).await;
        command(0, 0, 0).await;
        command(8, 0x1aa, CMD_SHORT).await;
        loop {
            command(55, 0, CMD_SHORT).await;
            command(41, 0x4030_0000, CMD_SHORT | CMD_NOCRC).await;
            if get(regs::resp0).await & 0x8000_0000 != 0 {
                break;
            }
        }
        command(2, 0, CMD_LONG).await;
        command(3, 0, CMD_SHORT).await;
        let rca = get(regs::resp0).await >> 16;
        command(7, rca << 16, CMD_SHORT | CMD_BUSY).await;
        command(16, 512, CMD_SHORT).await;
        command(55, rca << 16, CMD_SHORT).await;
        command(6, 2, CMD_SHORT).await;
        put(regs::ctrl, DIV | CTRL_WIDE).await;
        println!("{:6}  the card is up on four lines", now());
        // Two blocks from memory to the card, then the stop.
        put(regs::dma, FROM).await;
        put(regs::blocks, BLOCKS).await;
        command(25, FIRST, CMD_SHORT | CMD_WRITE).await;
        put(regs::blocks, 0).await;
        command(12, 0, CMD_SHORT | CMD_BUSY).await;
        println!(
            "{:6}  blocks {FIRST} and {} written from memory",
            now(),
            FIRST + 1
        );
        // The same two blocks from the card back into memory.
        put(regs::dma, INTO).await;
        put(regs::blocks, BLOCKS).await;
        command(18, FIRST, CMD_SHORT | CMD_READ).await;
        let left = get(regs::blocks).await;
        put(regs::blocks, 0).await;
        command(12, 0, CMD_SHORT | CMD_BUSY).await;
        println!(
            "{:6}  blocks {FIRST} and {} read into memory, blocks {left:#x}",
            now(),
            FIRST + 1
        );
        fin.set(true);
    };

    let mut sim = Running::new(join2(
        join2(
            join2(
                host_unit.run(host_in, host_out),
                bridge.run((aw, ar, w, [lb], [lr]), ([law], [lar], [lw], b, r)),
            ),
            join2(
                sd.run(
                    bus,
                    SdLines {
                        cmd_in,
                        dat_in,
                        sclk: sclk_o,
                        cmd_out: cmd_out_o,
                        cmd_oe: cmd_oe_o,
                        dat_out: dat_out_o,
                        dat_oe: dat_oe_o,
                        irq: irq_o,
                        dma_in,
                        dma_out,
                        dma_at: dma_at_o,
                        dma_bytes: dma_bytes_o,
                        dma_words: dma_words_o,
                        store_go: store_go_o,
                        fetch_go: fetch_go_o,
                        store_busy,
                        fetch_busy,
                    },
                ),
                client,
            ),
        ),
        join2(
            join2(
                join2(
                    store.run(
                        (
                            s_grant, s_done, dma_out_rx, dma_at, dma_bytes,
                            store_go,
                        ),
                        (s_issue, s_wbeat, s_release, store_busy_o),
                    ),
                    fetch.run(
                        (
                            f_grant, f_done, f_rdata, dma_at_f, dma_words,
                            fetch_go,
                        ),
                        (f_issue, f_release, dma_in_tx, fetch_busy_o),
                    ),
                ),
                join2(
                    shost.run(sl.host_in, sl.host_out),
                    fhost.run(fl.host_in, fl.host_out),
                ),
            ),
            join2(
                join2(
                    join2(noreads.run(s_rdata, ()), nobeats.run((), f_wbeat)),
                    arb.run(
                        ([s_aw, f_aw], [s_ar, f_ar], [s_w, f_w], m_b, m_r),
                        (m_aw, m_ar, m_w, [s_b, f_b], [s_r, f_r]),
                    ),
                ),
                join2(per.run(ml.per_in, ml.per_out), ram.run(mem_bus, ())),
            ),
        ),
    ));
    println!("     t  what the program saw");
    cmd_in_o.set(Bit::One);
    dat_in_o.set(U::<4>::from(0xfu8));
    let mut both = 0u32;
    let mut cycles = 0u32;
    while !finished.get() {
        sim.cycle();
        cycles += 1;
        assert!(cycles < 400_000, "the program finished");
        if store_seen.get().to_bool() && fetch_seen.get().to_bool() {
            both += 1;
        }
        let host_cmd = cmd_oe.get().to_bool();
        let host_dat = dat_oe.get().to_bool();
        let cmd = cmd_out.get().to_bool();
        let dat = dat_out.get().raw() as u8;
        card.step(sclk.get().to_bool(), host_cmd, cmd, host_dat, dat);
        cmd_in_o.set(Bit::from_bool(if host_cmd {
            cmd
        } else {
            card.cmd_out()
        }));
        dat_in_o.set(U::<4>::from(if host_dat { dat } else { card.dat_out() }));
    }
    stop();
    // What the card holds, what memory holds, and what was sent.
    let sent: Vec<u32> = (0..BLOCKS as usize * WORDS)
        .map(|i| mem.read(FROM as usize / 4 + i).raw() as u32)
        .collect();
    let held: Vec<u32> = card.blocks
        [FIRST as usize * 512..(FIRST + BLOCKS) as usize * 512]
        .chunks(4)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let back: Vec<u32> = (0..BLOCKS as usize * WORDS)
        .map(|i| mem.read(INTO as usize / 4 + i).raw() as u32)
        .collect();
    println!("cycles with both engines running: {both}");
    println!(
        "memory read back: {:08x} {:08x} .. {:08x}",
        back[0],
        back[1],
        back[back.len() - 1]
    );
    assert_eq!(both, 0, "the two engines never run at once");
    assert_eq!(held, sent, "the card holds the blocks sent from memory");
    assert_eq!(back, sent, "and memory holds them again, read back");
    let net = Sd::lowered("sd_dma");
    txhdl::netlist::write_netlists_from_env(&[&net]);
}
