// SPDX-License-Identifier: Apache-2.0
//! A frame's length, found again after the clock crossing.
//!
//! `FrameIn` must know a frame's length before the store engine
//! starts. `EthRx` knows it, but on the board the receiver is in the
//! top on the PHY's clock and only the bytes cross to the core's, so
//! the length never arrives. `FrameLen` finds it again: it holds a
//! frame whole and hands it on with its count, which is what `EthRx`
//! offers.
//!
//! So the question this run answers is whether `FrameLen` is a
//! drop-in for the receiver's output side:
//!
//! ```text
//! words -- FrameOut --bytes-- FrameLen --bytes, len-- FrameIn -- words
//! ```
//!
//! Two frames, 101 and 67 bytes, the same two `ex_ethdma` sends through
//! the real MAC, so the two runs can be read side by side: neither
//! length divides by four, and the second frame is what catches bytes
//! read past the first.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, signal, Clock, DefaultClock, Mem, Reg, Running, Rx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{with, Trace};
use txhdl_parts::eth::EthByte;
use txhdl_parts::ethdma::{FrameIn, FrameLen, FrameOut};

const BYTES: [usize; 2] = [101, 67];

const fn words_of(n: usize) -> usize {
    n.div_ceil(4)
}

/// Where the words that come back are put.
///
/// Not lowered: it is the test's end of the channel.
#[derive(Trace, Default)]
pub struct Sink<const N: usize> {
    pub mem: Mem<U<32>, N>,
    pub at: Reg<U<8>>,
}

impl<const N: usize> Unit<Rx<U<32>>, ()> for Sink<N> {
    async fn run(&mut self, inp: Rx<U<32>>, _out: ()) {
        loop {
            DefaultClock::rising().await;
            let take = Bit::from(inp.peek().is_some());
            let v = inp.recv_if(take).unwrap_or_default();
            with!(self <= {
                take ? {
                    mem.at(self.at.get()): v,
                    at: self.at.get() + 1,
                },
            });
        }
    }
}

fn word_at(f: usize, i: usize) -> u32 {
    0xc0de_0000 | ((f as u32) << 12) | (i as u32)
}

fn tail_of(n: usize, w: u32) -> u32 {
    match n % 4 {
        0 => w,
        1 => w & 0xff,
        2 => w & 0xffff,
        _ => w & 0xff_ffff,
    }
}

fn main() {
    let (src_tx, src_rx) = chan::<U<32>, DefaultClock>();
    let (mid_tx, mid_rx) = chan::<EthByte, DefaultClock>();
    let (inp_tx, inp) = chan::<EthByte, DefaultClock>();
    let (back_tx, back_rx) = chan::<U<32>, DefaultClock>();

    let (bytes_o, bytes) = signal::<U<16>, DefaultClock>();
    let (go_o, go) = signal::<Bit, DefaultClock>();
    let (run_o, running) = signal::<Bit, DefaultClock>();
    // The word count a fetch engine would read; nothing fetches here.
    let (nwords_o, _nwords) = signal::<U<16>, DefaultClock>();
    let (len_o, len) = signal::<U<16>, DefaultClock>();
    let (count_o, count) = signal::<U<16>, DefaultClock>();
    let (store_o, store) = signal::<Bit, DefaultClock>();
    let (which_o, which) = signal::<U<1>, DefaultClock>();
    // No store engine here, so nothing holds the next frame off.
    let (_hold_o, hold) = signal::<Bit, DefaultClock>();

    let mut fout = FrameOut::default();
    let mut flen = FrameLen::default();
    let mut fin = FrameIn::default();
    let mut sink = Sink::<64>::default();
    let landed = sink.mem.clone();

    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // FrameLen's own ports, under the names the unit gives them.
        w.add("inp", &mid_rx);
        w.add("out", &inp);
        w.add("len", &len);
        w.add("framelen", &flen);
        w.start();
    }

    bytes_o.set(U::<16>::from(BYTES[0] as u32));
    go_o.set(Bit::One);

    let mut sim = Running::new(join2(
        join2(
            fout.run((src_rx, bytes, go), (mid_tx, run_o, nwords_o)),
            flen.run(mid_rx, (inp_tx, len_o)),
        ),
        join2(
            fin.run((inp, len, hold), (back_tx, count_o, store_o, which_o)),
            sink.run(back_rx, ()),
        ),
    ));

    // The words go in from here, between steps, and the next frame is
    // started once the last has been measured and stored.
    let mut frame = 0usize;
    let mut next = 0usize;
    let mut stored = 0usize;
    let mut was_storing = false;
    for _ in 0..4000 {
        let take = words_of(BYTES[frame]);
        if next < take && src_tx.ready().to_bool() {
            src_tx.send(U::<32>::from(word_at(frame, next)));
            next += 1;
        }
        sim.cycle();
        if next >= take {
            go_o.set(Bit::Zero);
        }
        let storing = store.get().to_bool();
        if was_storing && !storing {
            stored += 1;
        }
        was_storing = storing;
        if stored == frame + 1
            && next >= take
            && !running.get().to_bool()
            && frame + 1 < BYTES.len()
        {
            frame += 1;
            next = 0;
            bytes_o.set(U::<16>::from(BYTES[frame] as u32));
            go_o.set(Bit::One);
        }
    }

    // The words, checked where they landed.
    let mut wrong = 0;
    let mut base = 0usize;
    for (f, n) in BYTES.iter().enumerate() {
        let w = words_of(*n);
        for i in 0..w {
            let got = landed.read(U::<8>::from((base + i) as u32)).raw() as u32;
            let want = if i + 1 == w {
                tail_of(*n, word_at(f, i))
            } else {
                word_at(f, i)
            };
            if got != want {
                if wrong < 4 {
                    println!("frame {f} word {i} came back {got:#010x}, wanted {want:#010x}");
                }
                wrong += 1;
            }
        }
        base += w;
    }
    let total: usize = BYTES.iter().map(|n| words_of(*n)).sum();
    println!("{} frames measured and stored, {total} words", stored);
    assert_eq!(stored, BYTES.len(), "both frames were measured and stored");
    assert_eq!(wrong, 0, "{wrong} of {total} words came back wrong");
    // The length FrameIn was given is the length FrameLen measured, so
    // the last one is the second frame's.
    assert_eq!(
        count.get().raw() as usize,
        BYTES[1],
        "the length FrameIn was told for the last frame"
    );
    assert_eq!(which.get().raw(), 0, "and the slots alternated");
    println!("each frame's length was found again after the crossing");

    stop();
    txhdl::netlist::write_vhdl_from_env(&FrameLen::lowered("framelen"));
}
