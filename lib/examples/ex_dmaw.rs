// SPDX-License-Identifier: Apache-2.0
//! Writing memory in bursts, from a channel.
//!
//! `LineStore` is the mirror of `LineFetch` and the half an Ethernet
//! receiver wants: words arrive on a channel and are written to
//! memory as bursts, rather than the core taking each one and storing
//! it.
//!
//! The run below pushes sixty-four words through sixteen-beat bursts
//! into a memory on the far side of a real link, then reads the
//! memory back and checks it. Reading it back is the point: a write
//! path that is only watched on the bus can be wrong in ways that
//! look fine going past, so the assertion is on what the memory
//! holds afterwards rather than on what the beats looked like.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, mux, now, signal, Clock, DefaultClock, Mem, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{
    axi_units, Answer, AxiHost, AxiPer, PerPort, Resp, R,
};
use txhdl_parts::dma::LineStore;

const ADDR: usize = 32;
const IDB: usize = 2;
const IDS: usize = 4;
const WORDS: usize = 64;
/// A byte count that is not a whole number of words, which is what a
/// frame usually is: 253 bytes is 64 words of which the last holds
/// one real byte.
const BYTES: usize = 253;
const BEATS: usize = 16;
const N: usize = 256;

/// A memory that takes writes as well as reads.
///
/// A write burst arrives as one request carrying `len` and is
/// followed by `len + 1` beats on the beat channel, answered once
/// when the last of them has landed. A model that answered per beat,
/// or that answered before the beats arrived, would let a broken
/// engine look correct.
#[derive(Trace, Default)]
pub struct Ram<const A: usize, const I: usize, const M: usize> {
    pub px: Mem<U<32>, M>,
    /// Whether a burst is being taken.
    pub busy: Reg<Bit>,
    /// Whether that burst is a write.
    pub wr: Reg<Bit>,
    /// The word the next beat goes to, or comes from.
    pub at: Reg<U<16>>,
    /// Beats still owed or expected.
    pub left: Reg<U<9>>,
    /// The identifier they carry.
    pub rid: Reg<U<I>>,
}

#[lower]
impl<const A: usize, const I: usize, const M: usize> Unit for Ram<A, I, M> {
    async fn run(&mut self, bus: PerPort<A, 32, 4, I>, _out: ()) {
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

            // A read beat goes out when there is room; a write beat is
            // taken when one is offered.
            let rbeat = busy & !wr & bus.r.ready();
            let wbeat = busy & wr & Bit::from(bus.w.peek().is_some());
            let word = bus.w.recv_if(busy & wr).unwrap_or_default();
            // The strobe says which bytes are meant. A model that
            // wrote the whole word would let an engine that ignores
            // the strobe look correct, which is exactly the fault a
            // partial last beat is there to catch.
            let was = self.px.read(self.at.get().slice::<0, 8>());
            let b0 = mux(
                word.strb.bit(0),
                word.data.slice::<0, 8>(),
                was.slice::<0, 8>(),
            );
            let b1 = mux(
                word.strb.bit(1),
                word.data.slice::<8, 8>(),
                was.slice::<8, 8>(),
            );
            let b2 = mux(
                word.strb.bit(2),
                word.data.slice::<16, 8>(),
                was.slice::<16, 8>(),
            );
            let b3 = mux(
                word.strb.bit(3),
                word.data.slice::<24, 8>(),
                was.slice::<24, 8>(),
            );
            let merged = b3
                .concat::<8, 16>(b2)
                .concat::<8, 24>(b1)
                .concat::<8, 32>(b0);
            let at_last = self.left.get() == 1;
            let rlast = rbeat & at_last;
            // The write is answered once, when its last beat lands.
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
                    px.at(self.at.get().slice::<0, 8>()): merged,
                    at: (self.at.get() + 1) & mask,
                    left: self.left.get() - 1,
                },
                rlast ? busy: Bit::Zero,
                wlast ? busy: Bit::Zero,
            });

            if rbeat.to_bool() {
                bus.r.send(R {
                    id: self.rid.get(),
                    data: self.px.read(self.at.get().slice::<0, 8>()),
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
    let link = axi_units::<ADDR, 32, 4, IDB>();
    let (issue, wbeat, release, grant, done, rdata) = link.host_client;
    let bus: PerPort<ADDR, 32, 4, IDB> = link.per_client.into();
    let (base_o, base) = signal::<U<ADDR>, DefaultClock>();
    let (bytes_o, bytes) = signal::<U<16>, DefaultClock>();
    let (go_o, go) = signal::<Bit, DefaultClock>();
    let (run_o, running) = signal::<Bit, DefaultClock>();
    let (src_tx, src_rx) = chan::<U<32>, DefaultClock>();

    let mut ram = Ram::<ADDR, IDB, N>::default();
    // The last word is poisoned with bytes that must survive: only
    // the one real byte of the frame may land there, and a strobe
    // that wrote the whole word would wipe the other three.
    ram.px.write(
        U::<8>::from((WORDS - 1) as u32),
        U::<32>::from(0xaaaa_aa00u32),
    );
    // A second handle on the memory, so the run can be checked
    // against what actually landed rather than against the bus.
    let stored = ram.px.clone();
    let mut host = AxiHost::<ADDR, 32, 4, IDB, IDS>::default();
    let mut per = AxiPer::<ADDR, 32, 4, IDB>::default();
    let mut store = LineStore::<ADDR, IDB, BEATS, 16>::default();

    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("grant", &grant);
        w.add("done", &done);
        w.add("inp", &src_rx);
        w.add("base", &base);
        w.add("bytes", &bytes);
        w.add("go", &go);
        w.add("issue", &issue);
        w.add("wbeat", &wbeat);
        w.add("release", &release);
        w.add("running", &running);
        w.add("linestore", &store);
        w.start();
    }

    base_o.set(U::<ADDR>::from(0u32));
    bytes_o.set(U::<16>::from(BYTES as u32));
    go_o.set(Bit::One);

    let mut sim = Running::new(join2(
        join2(
            host.run(link.host_in, link.host_out),
            per.run(link.per_in, link.per_out),
        ),
        join2(
            store.run(
                (grant, done, src_rx, base, bytes, go),
                (issue, wbeat, release, run_o),
            ),
            ram.run(bus, ()),
        ),
    ));
    let _ = rdata;

    // Each word is its own index, so a word written to the wrong
    // place says which place it came from.
    let mut next = 0usize;
    let start = now();
    let cap = 4000u64;
    // Run until the engine says it has finished, rather than for a
    // fixed window, so that the cycle count is the transfer and not
    // the length of the test.
    while (next < WORDS || running.get().to_bool()) && now() - start < cap {
        if next < WORDS && src_tx.ready().to_bool() {
            src_tx.send(U::<32>::from(next as u32));
            next += 1;
        }
        sim.cycle();
        if next >= WORDS {
            go_o.set(Bit::Zero);
        }
    }
    let cycles = (now() - start) / 2;
    println!("{next} words written in {cycles} cycles");

    // What the memory holds, which is the only thing that settles it.
    let mut wrong = 0;
    for i in 0..WORDS - 1 {
        let v = stored.read(U::<8>::from(i as u32)).raw();
        if v != i as u128 {
            if wrong < 4 {
                println!("word {i} is {v}, wanted {i}");
            }
            wrong += 1;
        }
    }
    assert_eq!(next, WORDS, "every word was offered");
    assert_eq!(wrong, 0, "{wrong} of {WORDS} words landed wrong");
    // The partial last word: byte zero is the frame's, and the three
    // bytes above it are the poison, untouched.
    let tailword = stored.read(U::<8>::from((WORDS - 1) as u32)).raw();
    let want_tail = 0xaaaa_aa00u128 | ((WORDS - 1) as u128 & 0xff);
    assert_eq!(
        tailword, want_tail,
        "the last word should hold one frame byte over three poison bytes"
    );
    println!("the partial last word is {tailword:#010x}, poison intact");
    // Nothing beyond the run may be touched: a burst that ran on past
    // its count would show here and nowhere else.
    for i in WORDS..WORDS + 16 {
        let v = stored.read(U::<8>::from(i as u32)).raw();
        assert_eq!(v, 0, "word {i} is past the run and should be untouched");
    }
    // The same bound as the read direction: a path that spent an
    // address phase per word could not write sixty four in this many
    // cycles, and sixteen-beat bursts need four.
    let bound = (WORDS + 4 * BEATS) as u64;
    assert!(
        cycles < bound,
        "sixty four words wanted fewer than {bound} cycles, took {cycles}"
    );
    println!("{WORDS} words landed, and nothing past them was touched");

    print!(
        "\n{}",
        LineStore::<ADDR, IDB, BEATS, 16>::verilog("linestore")
    );
    stop();
    txhdl::netlist::write_vhdl_from_env(
        &LineStore::<ADDR, IDB, BEATS, 16>::lowered("linestore"),
    );
}
