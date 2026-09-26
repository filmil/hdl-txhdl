// SPDX-License-Identifier: Apache-2.0
//! The engines against the bridge they will really talk through.
//!
//! `ex_dma` and `ex_dmaw` measure `LineFetch` and `LineStore` against
//! a memory defined in the example: local, on the peripheral side of
//! the link, and answering a burst in full. That is the right model
//! for measuring an engine, and it is not the path to DDR3.
//!
//! The real path goes through [`txhdl_parts::bus::wb::AxiWb`], which
//! puts each word of a burst on a pipelined Wishbone as its own
//! request. Until issue 471 it answered any read with one beat marked
//! last whatever `len` asked for, and neither engine could tell:
//! `LineFetch` counts beats itself and waited for ever for the rest,
//! and `LineStore` left the beats it had not taken in the channel and
//! wrote nine words of sixty four, quietly.
//!
//! So this run puts each engine behind a real `AxiWb` and a Wishbone
//! memory that answers slowly and calibrates first, which is what the
//! controller does. `ex_dma`'s own comment says a model that answered
//! one beat per request "would prove nothing about bursting"; that
//! was written about the test model, and it described the real
//! peripheral. This is the run that checks the system rather than the
//! model.
//!
//! The two engines are CHAINED rather than tested apart: the fetch
//! engine reads a memory through one bridge, its words feed the store
//! engine, and that writes them to a second memory through a second
//! bridge. So the check at the end, that the far memory holds what
//! the near one held, is a check of both engines and both bridges at
//! once, and a word lost anywhere in the path shows up as a word
//! missing at the end.
//!
//! A bridge and a memory each rather than an arbiter between them: a
//! host sharing the link is the board's business, and one set of
//! Wishbone lines cannot carry two bridges.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer, PerPort};
use txhdl_parts::bus::wb::sim::WbMem;
use txhdl_parts::bus::wb::AxiWb;
use txhdl_parts::bus::wb::WbMaster;
use txhdl_parts::dma::{LineFetch, LineStore};

const ADDR: usize = 32;
const IDB: usize = 2;
const IDS: usize = 4;
/// The Wishbone word address width.
const AW: usize = 28;
const WORDS: usize = 64;
const BEATS: usize = 16;

/// The word at index `i`: its own index, so a word that came back
/// from the wrong place says where it came from.
fn word_at(i: usize) -> u32 {
    0x5100_0000 | (i as u32)
}

fn main() {
    // The reading stack: a fetch engine, a bridge, and a memory that
    // already holds what it should bring back.
    let rd = axi_units::<ADDR, 32, 4, IDB>();
    let (r_issue, _r_wbeat, r_release, r_grant, r_done, r_rdata) =
        rd.host_client;
    let r_bus = PerPort::from(rd.per_client);
    let (rbase_o, r_base) = signal::<U<ADDR>, DefaultClock>();
    let (rwords_o, r_words) = signal::<U<16>, DefaultClock>();
    let (rgo_o, r_go) = signal::<Bit, DefaultClock>();
    let (rrun_o, r_running) = signal::<Bit, DefaultClock>();
    let (r_cyc_o, r_cyc) = signal::<Bit, DefaultClock>();
    let (r_stb_o, r_stb) = signal::<Bit, DefaultClock>();
    let (r_we_o, r_we) = signal::<Bit, DefaultClock>();
    let (r_adr_o, r_adr) = signal::<U<AW>, DefaultClock>();
    let (r_dat_o, r_dat) = signal::<U<32>, DefaultClock>();
    let (r_sel_o, r_sel) = signal::<U<4>, DefaultClock>();
    let (r_stall_o, r_stall) = signal::<Bit, DefaultClock>();
    let (r_ack_o, r_ack) = signal::<Bit, DefaultClock>();
    let (r_rdat_o, r_rdat) = signal::<U<32>, DefaultClock>();

    // A memory that stalls while it calibrates and answers after two
    // cycles, which is nearer the controller than a memory that
    // answers at once.
    let src = WbMem::<AW>::new(2, 10);
    for i in 0..WORDS {
        src.set(i as u128, word_at(i));
    }
    let mut src_mem = src.clone();
    let mut r_host = AxiHost::<ADDR, 32, 4, IDB, IDS>::default();
    let mut r_per = AxiPer::<ADDR, 32, 4, IDB>::default();
    let mut r_bridge = AxiWb::<ADDR, IDB, AW>::default();
    let mut fetch = LineFetch::<ADDR, IDB, BEATS, 16>::default();

    // The writing stack: a store engine, its own bridge, its own
    // memory, and a source of words.
    let wr = axi_units::<ADDR, 32, 4, IDB>();
    let (w_issue, w_wbeat, w_release, w_grant, w_done, _w_rdata) =
        wr.host_client;
    let w_bus = PerPort::from(wr.per_client);
    let (wbase_o, w_base) = signal::<U<ADDR>, DefaultClock>();
    let (wbytes_o, w_bytes) = signal::<U<16>, DefaultClock>();
    let (wgo_o, w_go) = signal::<Bit, DefaultClock>();
    let (wrun_o, w_running) = signal::<Bit, DefaultClock>();
    let (w_cyc_o, w_cyc) = signal::<Bit, DefaultClock>();
    let (w_stb_o, w_stb) = signal::<Bit, DefaultClock>();
    let (w_we_o, w_we) = signal::<Bit, DefaultClock>();
    let (w_adr_o, w_adr) = signal::<U<AW>, DefaultClock>();
    let (w_dat_o, w_dat) = signal::<U<32>, DefaultClock>();
    let (w_sel_o, w_sel) = signal::<U<4>, DefaultClock>();
    let (w_stall_o, w_stall) = signal::<Bit, DefaultClock>();
    let (w_ack_o, w_ack) = signal::<Bit, DefaultClock>();
    let (w_rdat_o, w_rdat) = signal::<U<32>, DefaultClock>();
    let (feed_tx, feed_rx) = chan::<U<32>, DefaultClock>();

    let dst = WbMem::<AW>::new(2, 10);
    let mut dst_mem = dst.clone();
    let mut w_host = AxiHost::<ADDR, 32, 4, IDB, IDS>::default();
    let mut w_per = AxiPer::<ADDR, 32, 4, IDB>::default();
    let mut w_bridge = AxiWb::<ADDR, IDB, AW>::default();
    let mut store = LineStore::<ADDR, IDB, BEATS, 16>::default();

    if let Some(mut wv) = Wave::from_env() {
        wv.clock::<DefaultClock>();
        wv.add("r_running", &r_running);
        wv.add("w_running", &w_running);
        wv.add("r_cyc", &r_cyc);
        wv.add("r_ack", &r_ack);
        wv.add("w_cyc", &w_cyc);
        wv.add("w_ack", &w_ack);
        wv.add("linefetch", &fetch);
        wv.add("linestore", &store);
        wv.start();
    }

    rbase_o.set(U::<ADDR>::from(0u32));
    rwords_o.set(U::<16>::from(WORDS as u32));
    rgo_o.set(Bit::One);
    wbase_o.set(U::<ADDR>::from(0u32));
    wbytes_o.set(U::<16>::from((WORDS * 4) as u32));
    wgo_o.set(Bit::One);

    let reading = join2(
        join2(
            r_host.run(rd.host_in, rd.host_out),
            r_per.run(rd.per_in, rd.per_out),
        ),
        join2(
            join2(
                fetch.run(
                    (r_grant, r_done, r_rdata, r_base, r_words, r_go),
                    (r_issue, r_release, feed_tx, rrun_o),
                ),
                r_bridge.run(
                    r_bus,
                    WbMaster {
                        stall: r_stall,
                        ack: r_ack,
                        rdat: r_rdat,
                        cyc: r_cyc_o,
                        stb: r_stb_o,
                        we: r_we_o,
                        adr: r_adr_o,
                        dat: r_dat_o,
                        sel: r_sel_o,
                    },
                ),
            ),
            src_mem.run(
                (r_cyc, r_stb, r_we, r_adr, r_dat, r_sel),
                (r_stall_o, r_ack_o, r_rdat_o),
            ),
        ),
    );
    let writing = join2(
        join2(
            w_host.run(wr.host_in, wr.host_out),
            w_per.run(wr.per_in, wr.per_out),
        ),
        join2(
            join2(
                store.run(
                    (w_grant, w_done, feed_rx, w_base, w_bytes, w_go),
                    (w_issue, w_wbeat, w_release, wrun_o),
                ),
                w_bridge.run(
                    w_bus,
                    WbMaster {
                        stall: w_stall,
                        ack: w_ack,
                        rdat: w_rdat,
                        cyc: w_cyc_o,
                        stb: w_stb_o,
                        we: w_we_o,
                        adr: w_adr_o,
                        dat: w_dat_o,
                        sel: w_sel_o,
                    },
                ),
            ),
            dst_mem.run(
                (w_cyc, w_stb, w_we, w_adr, w_dat, w_sel),
                (w_stall_o, w_ack_o, w_rdat_o),
            ),
        ),
    );
    let mut sim = Running::new(join2(reading, writing));
    let start = now();
    let cap = 20000u64;
    // Run until both engines have started and both have gone idle,
    // rather than while either is running: at the first cycle neither
    // has started, so the second test is false before it is true and
    // the run would end before it began.
    let mut began = false;
    while now() - start < cap {
        sim.cycle();
        // `go` is a level, so it is dropped once the engine has taken
        // the request; held high it would start the run again as soon
        // as it ended.
        if r_running.get().to_bool() {
            rgo_o.set(Bit::Zero);
        }
        if w_running.get().to_bool() {
            wgo_o.set(Bit::Zero);
        }
        let busy = r_running.get().to_bool() || w_running.get().to_bool();
        began |= busy;
        if began && !busy {
            break;
        }
    }
    let cycles = (now() - start) / 2;
    println!("{WORDS} words through a real bridge in {cycles} cycles");
    // That the run ENDED is its own assertion, and it is the one a
    // peripheral answering too few beats fails first. An engine that
    // counts beats itself waits for the ones that never come, so the
    // fault is a run that does not finish rather than a wrong word;
    // without this the failure would be reported as missing data and
    // the reason would be a guess.
    assert!(
        now() - start < cap,
        "both engines finished: neither is still waiting for a beat"
    );
    assert!(
        !r_running.get().to_bool() && !w_running.get().to_bool(),
        "and both are idle at the end"
    );

    let mut wrong = 0;
    for i in 0..WORDS {
        let got = dst.word(i as u128);
        if got != word_at(i) {
            if wrong < 4 {
                println!(
                    "word {i} landed {got:#010x}, sent {:#010x}",
                    word_at(i)
                );
            }
            wrong += 1;
        }
    }
    assert_eq!(wrong, 0, "{wrong} of {WORDS} words landed wrong");
    // A bound rather than a number, since the memory's latency and
    // its calibration are part of it. What it rules out is a path
    // that spent an address phase per word: sixty four words of
    // sixteen beat bursts through a bridge that serves a word at a
    // time, against a memory that stalls for ten cycles and answers
    // after two.
    let bound = (WORDS * 12) as u64;
    assert!(
        cycles < bound,
        "sixty four words wanted fewer than {bound} cycles, took {cycles}"
    );
    println!("every word of the burst crossed the bridge and landed");
    stop();
}
