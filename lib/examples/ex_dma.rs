// SPDX-License-Identifier: Apache-2.0
//! Reading memory in bursts, and what it buys.
//!
//! `LineFetch` is a host on the bus. Told an address and a count, it
//! issues read bursts and puts the words that come back into a
//! channel. The alternative, which is what the video peripheral and
//! the MAC do today, is the core reading a word at a time over
//! AXI-Lite and writing it on: a full address phase per word.
//!
//! The run below fetches sixty-four words through sixteen-beat
//! bursts, so four address phases carry what would otherwise take
//! sixty-four. It checks the words, and it checks the cycles: the
//! assertion at the end is a bound no per-word path can meet, which
//! is the gain stated as something a test can fail rather than a
//! number printed for somebody to read.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, now, signal, Clock, DefaultClock, Mem, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{
    axi_units, Answer, AxiHost, AxiPer, PerPort, Resp, R,
};
use txhdl_parts::dma::LineFetch;

const ADDR: usize = 32;
const IDB: usize = 2;
const IDS: usize = 4;
/// Sixty-four words, in bursts of sixteen.
const WORDS: usize = 64;
const BEATS: usize = 16;
/// How many words the memory holds.
const N: usize = 256;

/// A memory on the peripheral side, read only, which is all the
/// fetcher asks of it.
///
/// A burst arrives as ONE request carrying `len`, beats less one, and
/// is answered with `len + 1` beats on the read channel, the last of
/// them marked. A model that answered one beat per request would
/// make a burst look like a word and the run would prove nothing
/// about bursting, which is the whole of what is being measured here.
#[derive(Trace, Default)]
pub struct Rom<const A: usize, const I: usize, const M: usize> {
    pub px: Mem<U<32>, M>,
    /// Whether a burst is being answered.
    pub busy: Reg<Bit>,
    /// The word the next beat comes from.
    pub at: Reg<U<16>>,
    /// Beats still owed, including the one going out.
    pub left: Reg<U<9>>,
    /// The identifier those beats carry.
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

            // A read is taken when nothing is being answered; a write
            // is taken and answered at once, since the fetcher never
            // writes and this only keeps one from hanging.
            let take = offered & q.read & !busy;
            let write = offered & !q.read & bus.ans.ready();
            let _ = bus.req.recv_if(take | write);
            let _ = bus.w.recv_if(write);

            // A beat goes out whenever one is owed and there is room.
            let beat = busy & bus.r.ready();
            let last = beat & (self.left.get() == 1);

            with!(self <= {
                take ? {
                    busy: Bit::One,
                    at: qat,
                    left: (q.len.resize::<9>() + 1),
                    rid: q.id,
                },
                beat ? { at: (self.at.get() + 1) & mask, left: self.left.get() - 1 },
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
    let (px_tx, px_rx) = txhdl::comp::chan::<U<32>, DefaultClock>();

    // The memory holds its own address in each word, so a word that
    // arrives out of order or from the wrong burst says so.
    let image: Vec<U<32>> = (0..N).map(|i| U::<32>::from(i as u32)).collect();
    let mut rom = Rom::<ADDR, IDB, N> {
        px: Mem::with(&image),
        ..Default::default()
    };
    let mut host = AxiHost::<ADDR, 32, 4, IDB, IDS>::default();
    let mut per = AxiPer::<ADDR, 32, 4, IDB>::default();
    let mut fetch = LineFetch::<ADDR, IDB, BEATS, 16>::default();

    // Every port under the name the port has, and the unit under the
    // entity's name: the testbench generator reads the trace by those
    // names and refuses what it cannot find.
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("grant", &grant);
        w.add("done", &done);
        w.add("rdata", &rdata);
        w.add("base", &base);
        w.add("words", &words);
        w.add("go", &go);
        w.add("issue", &issue);
        w.add("release", &release);
        w.add("out", &px_rx);
        w.add("running", &running);
        w.add("linefetch", &fetch);
        w.start();
    }

    base_o.set(U::<ADDR>::from(0u32));
    words_o.set(U::<16>::from(WORDS as u32));
    go_o.set(Bit::One);

    let mut sim = Running::new(join2(
        join2(
            host.run(link.host_in, link.host_out),
            per.run(link.per_in, link.per_out),
        ),
        join2(
            fetch.run(
                (grant, done, rdata, base, words, go),
                (issue, release, px_tx, run_o),
            ),
            rom.run(bus, ()),
        ),
    ));
    let _ = wbeat;

    let mut got: Vec<u128> = Vec::new();
    let start = now();
    let cap = 4000u64;
    while got.len() < WORDS && now() - start < cap {
        sim.cycle();
        // The run is told to stop asking once it has what it wants.
        if got.len() + 1 >= WORDS {
            go_o.set(Bit::Zero);
        }
        while let Some(v) = px_rx.recv() {
            got.push(v.raw());
        }
    }
    let cycles = (now() - start) / 2;
    println!("{} words in {} cycles", got.len(), cycles);

    // The words, in order, each the address it came from.
    assert_eq!(got.len(), WORDS, "every word arrived");
    for (i, v) in got.iter().enumerate() {
        assert_eq!(*v, i as u128, "word {i} is the one at that address");
    }

    // The gain, as a bound rather than a number to admire. A path
    // that spent an address phase per word could not deliver sixty
    // four words in this many cycles; bursts of sixteen do it in
    // four. The margin is wide because the point is the shape of the
    // traffic, not a cycle count to defend.
    let bound = (WORDS + 4 * BEATS) as u64;
    assert!(
        cycles < bound,
        "sixty four words wanted fewer than {bound} cycles, took {cycles}"
    );

    print!(
        "\n{}",
        LineFetch::<ADDR, IDB, BEATS, 16>::verilog("linefetch")
    );
    stop();
    txhdl::netlist::write_vhdl_from_env(
        &LineFetch::<ADDR, IDB, BEATS, 16>::lowered("linefetch"),
    );
}
