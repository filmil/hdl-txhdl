// SPDX-License-Identifier: Apache-2.0
//! Words to a frame and back, through the MAC.
//!
//! The engines of issue 151 move 32-bit words, because that is what
//! the bus moves. A frame is a count of bytes, and not usually a
//! whole number of words. `FrameOut` and `FrameIn` are that
//! difference, and this run is the two of them with the real MAC
//! between, so the question it answers is the one that matters: do
//! the words that went out come back?
//!
//! ```text
//! words -- FrameOut --bytes-- EthTx --wire-- EthRx --bytes-- FrameIn -- words
//! ```
//!
//! The frame is 101 bytes, which is deliberate. It is above the 60
//! that the transmitter pads to, so no padding is in the way, and it
//! is not a multiple of four, so its last word holds one real byte
//! over three that are not. A run whose length divided by four would
//! never touch the case the two units exist for.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, signal, Clock, DefaultClock, Mem, Reg, Running, Rx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{with, Trace};
use txhdl_parts::eth::EthRxLines;
use txhdl_parts::eth::EthTxLines;
use txhdl_parts::eth::{EthByte, EthRx, EthTx};
use txhdl_parts::ethdma::{FrameIn, FrameOut};

/// Two frames, and neither length is a whole number of words.
///
/// Both are above the 60 bytes the transmitter pads to, so no padding
/// is in the way. 101 leaves one real byte in its last word and 67
/// leaves three, so the two partial widths that are not one are both
/// covered rather than the same case twice.
///
/// TWO frames rather than one, because one does not catch the fault
/// this pair exists to prevent. A `FrameOut` that reads whole words
/// rather than stopping at the byte count still marks the frame's
/// last byte in the right place, so the frame on the wire is the
/// right length and every check of it passes. The bytes it read past
/// the frame stay in the transmitter, which has seen no last byte
/// for them, and they go out in front of the NEXT frame. Only a
/// second frame can see them.
///
/// A third, of 77 bytes, comes while `FrameIn` is told that no slot is
/// free, as the register block tells it while the driver holds both
/// (issue 1313): its bytes are taken and none of them land, and it is
/// counted as dropped.
const BYTES: [usize; 3] = [101, 67, 77];

/// The frames that land; the ones after are dropped.
const KEPT: usize = 2;

/// The words a frame of `n` bytes takes, its last one partial.
const fn words_of(n: usize) -> usize {
    n.div_ceil(4)
}

/// Where the words that come back are put, so the run can be checked
/// against what landed rather than against what went past.
///
/// Not lowered: it is the test's end of the wire and not hardware.
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

/// The word at index `i` of frame `f`: the frame and the index in the
/// low half, over a mark in the high half, so a word that came back
/// in the wrong place, or from the wrong frame, says where it came
/// from rather than merely being wrong.
fn word_at(f: usize, i: usize) -> u32 {
    0xc0de_0000 | ((f as u32) << 12) | (i as u32)
}

/// What the last word of a frame of `n` bytes should come back as:
/// its real bytes, and nothing above them.
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
    let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
    let (rx_tx, rx_rx) = chan::<EthByte, DefaultClock>();
    let (back_tx, back_rx) = chan::<U<32>, DefaultClock>();

    let (bytes_o, bytes) = signal::<U<16>, DefaultClock>();
    let (go_o, go) = signal::<Bit, DefaultClock>();
    let (run_o, running) = signal::<Bit, DefaultClock>();
    let (nwords_o, nwords) = signal::<U<16>, DefaultClock>();
    let (txd_o, txd) = signal::<U<8>, DefaultClock>();
    let (en_o, tx_en) = signal::<Bit, DefaultClock>();
    let (rxd_o, rxd) = signal::<U<8>, DefaultClock>();
    let (dv_o, rx_dv) = signal::<Bit, DefaultClock>();
    let (er_o, rx_er) = signal::<Bit, DefaultClock>();
    let (rxlen_o, rx_len) = signal::<U<16>, DefaultClock>();
    // The receiver's drops by cause, which nothing here reads.
    let (rxcnt_o, _rxcnt) = signal::<U<48>, DefaultClock>();
    let (inbytes_o, in_bytes) = signal::<U<16>, DefaultClock>();
    let (ingo_o, in_go) = signal::<Bit, DefaultClock>();
    let (which_o, which) = signal::<U<1>, DefaultClock>();
    // No store engine here, so nothing holds the next frame off.
    let (_hold_o, hold) = signal::<Bit, DefaultClock>();
    let (noslot_o, no_slot) = signal::<Bit, DefaultClock>();
    let (drops_o, drops) = signal::<U<32>, DefaultClock>();

    let mut fout = FrameOut::default();
    let mut fin = FrameIn::default();
    let mut mac_tx = EthTx::default();
    let mut mac_rx = EthRx::default();
    let mut sink = Sink::<64>::default();
    let landed = sink.mem.clone();
    // The receiver's own counters. One frame must arrive and none be
    // dropped: see the assertion below for why counting frames is
    // not the same check as measuring the one that arrived.
    let frames = mac_rx.frames;
    let dropped = mac_rx.dropped;

    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("inp", &src_rx);
        w.add("bytes", &bytes);
        w.add("go", &go);
        w.add("out", &out_rx);
        w.add("running", &running);
        w.add("nwords", &nwords);
        w.add("rx", &rx_rx);
        w.add("len", &rx_len);
        w.add("words", &back_rx);
        w.add("count", &in_bytes);
        w.add("store", &in_go);
        w.add("which", &which);
        w.add("hold", &hold);
        w.add("no_slot", &no_slot);
        w.add("drops", &drops);
        w.add("frame_out", &fout);
        w.add("frame_in", &fin);
        w.start();
    }

    bytes_o.set(U::<16>::from(BYTES[0] as u32));
    go_o.set(Bit::One);

    let mut sim = Running::new(join2(
        join2(
            fout.run((src_rx, bytes, go), (out_tx, run_o, nwords_o)),
            mac_tx.run(
                out_rx,
                EthTxLines {
                    txd: txd_o,
                    tx_en: en_o,
                },
            ),
        ),
        join2(
            mac_rx.run(
                EthRxLines { rxd, rx_dv, rx_er },
                (rx_tx, rxlen_o, rxcnt_o),
            ),
            join2(
                fin.run(
                    (rx_rx, rx_len, hold, no_slot),
                    (back_tx, inbytes_o, ingo_o, which_o, drops_o),
                ),
                sink.run(back_rx, ()),
            ),
        ),
    ));

    // The words go in from here rather than from a unit, and the wire
    // is modelled here too: both are outside a step rather than
    // inside one, so nothing races the units within a cycle.
    let mut frame = 0usize;
    let mut next = 0usize;
    let mut offered = 0usize;
    for _ in 0..5000 {
        let take = words_of(BYTES[frame]);
        if next < take && src_tx.ready().to_bool() {
            src_tx.send(U::<32>::from(word_at(frame, next)));
            next += 1;
            offered += 1;
        }
        sim.cycle();
        if next >= take {
            go_o.set(Bit::Zero);
        }
        // The next frame starts once this one has gone out and come
        // back, so the two are separate on the wire rather than one
        // run of bytes.
        let done = next >= take
            && !running.get().to_bool()
            && frames.get().raw() as usize == frame + 1;
        if done && frame + 1 < BYTES.len() {
            frame += 1;
            next = 0;
            bytes_o.set(U::<16>::from(BYTES[frame] as u32));
            go_o.set(Bit::One);
            // No slot is free for the frames after the kept ones.
            noslot_o.set(Bit::from_bool(frame >= KEPT));
        }
        // The wire, a cycle late: what the transmitter drove is what
        // the receiver sees.
        let on = tx_en.get().to_bool();
        rxd_o.set(U::<8>::from(txd.get().raw() as u32));
        dv_o.set(Bit::from_bool(on));
        er_o.set(Bit::Zero);
    }

    let sent: usize = BYTES.iter().map(|n| words_of(*n)).sum();
    let total: usize = BYTES[..KEPT].iter().map(|n| words_of(*n)).sum();
    println!(
        "{offered} words offered as {} frames of {:?} bytes",
        BYTES.len(),
        BYTES
    );
    println!(
        "{} frames arrived, {} dropped, the last {} bytes long",
        frames.get().raw(),
        dropped.get().raw(),
        in_bytes.get().raw()
    );

    // Every word of every frame, read back from where it landed.
    let mut wrong = 0;
    let mut base = 0usize;
    for (f, n) in BYTES[..KEPT].iter().enumerate() {
        let w = words_of(*n);
        for i in 0..w {
            let got = landed.read(U::<8>::from((base + i) as u32)).raw() as u32;
            // The last word of a frame keeps only its real bytes.
            let want = if i + 1 == w {
                tail_of(*n, word_at(f, i))
            } else {
                word_at(f, i)
            };
            if got != want {
                if wrong < 6 {
                    println!(
                        "frame {f} word {i} came back {got:#010x}, wanted {want:#010x}"
                    );
                }
                wrong += 1;
            }
        }
        base += w;
    }
    assert_eq!(offered, sent, "every word of every frame was offered");
    assert_eq!(wrong, 0, "{wrong} of {total} words came back wrong");

    // Every frame arrived, and the receiver dropped none. A frame that
    // carried bytes read past the one before it would fail the word
    // check above rather than this, but a frame that never arrived
    // at all would fail only here.
    assert_eq!(
        frames.get().raw() as usize,
        BYTES.len(),
        "every frame arrived"
    );
    assert_eq!(dropped.get().raw(), 0, "and the receiver dropped none");
    // The last was taken with no slot free: counted, and not stored.
    assert_eq!(drops.get().raw(), 1, "the frame with no slot is counted");
    assert_eq!(
        in_bytes.get().raw() as usize,
        BYTES[BYTES.len() - 1],
        "the length the receiver reported for the last frame"
    );
    // The slot alternates: the first frame takes slot one, the second
    // slot zero, so a driver reading one is never read into. The
    // dropped frame takes none.
    assert_eq!(
        which.get().raw(),
        0,
        "the second frame takes the other slot, and the third none"
    );
    // Nothing past the kept frames may be written.
    for i in total..total + 4 {
        let v = landed.read(U::<8>::from(i as u32)).raw();
        assert_eq!(v, 0, "word {i} is past the kept frames and untouched");
    }
    println!(
        "{total} words came back in order, and nothing past them; {} dropped",
        drops.get().raw()
    );

    stop();
    let out_net = FrameOut::lowered("frame_out");
    let in_net = FrameIn::lowered("frame_in");
    txhdl::netlist::write_netlists_from_env(&[&out_net, &in_net]);
}
