// SPDX-License-Identifier: Apache-2.0
//! A program on another machine moves 16-byte beats through a wide
//! AXI4 port of this design, one beat per Ethernet frame.
//!
//! `ex_hostlink` reaches a 32-bit AXI-Lite bus. FireSim's streams need
//! a 128-bit AXI4 port, which its driver reads and writes with
//! `cpu_managed_axi4_read` and `cpu_managed_axi4_write`. `StreamLink`
//! takes a beat's ask off the wire, `BeatMaster` performs it as a burst
//! of one beat, and the answer goes back in a frame (issue 1984).
//!
//! Behind the port is a queue, as a FireSim stream is: a read takes
//! the next beat out and a write puts one in. The program writes two
//! beats, reads them back, and asks the second read again under the
//! same tag, as it does when an answer is lost. The repeat is answered
//! from what the master kept, so the queue does not lose a beat, which
//! the port's count of transfers shows.
//!
//! The run starts as a host driver does, with a no-op that sets the
//! tag the master keeps and never reaches the port.
//!
//! The two units and the bridge that joins them are lowered, and the
//! build simulates the three netlists against this run under nvc and
//! Verilator.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, Clock, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{Ar, Aw, Resp, B, R, W};
use txhdl_parts::eth::EthByte;
use txhdl_parts::remote::stream::{
    BeatAnswer, BeatAsk, BeatMaster, StreamBridge, StreamLink,
    KIND_STREAM_ANSWER, KIND_STREAM_ASK, STREAM_FRAME_LEN,
};

/// This design's device number on the wire.
const DEVICE: usize = 5;

/// A frame in which the program asks, padded as a MAC pads.
fn ask_frame(tag: u8, write: bool, addr: u64, beat: u128) -> Vec<u8> {
    let mut f = vec![0xffu8; 6];
    f.extend_from_slice(&[0x02, 0, 0, 0, 0, 0x10]);
    let kind = KIND_STREAM_ASK as u8;
    f.extend_from_slice(&[0x88, 0xb6, kind, DEVICE as u8, tag]);
    f.push(write as u8);
    f.extend_from_slice(&addr.to_be_bytes());
    f.extend_from_slice(&beat.to_le_bytes());
    let strb: u16 = if write { 0xffff } else { 0 };
    f.extend_from_slice(&strb.to_be_bytes());
    f.resize(60, 0);
    f
}

fn main() {
    let (ask_tx, ask_rx) = chan::<BeatAsk, DefaultClock>();
    let (ans_tx, ans_rx) = chan::<BeatAnswer, DefaultClock>();
    let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
    let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
    let (aw_tx, aw_rx) = chan::<Aw<64, 4>, DefaultClock>();
    let (ar_tx, ar_rx) = chan::<Ar<64, 4>, DefaultClock>();
    let (w_tx, w_rx) = chan::<W<128, 16>, DefaultClock>();
    let (b_tx, b_rx) = chan::<B<4>, DefaultClock>();
    let (r_tx, r_rx) = chan::<R<128, 4>, DefaultClock>();
    let mut link = StreamLink::<DEVICE>::default();
    let mut master = BeatMaster::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        // Every port of both lowered units, under the name each gives
        // it; `ask` and `back` are a port of both.
        wave.add("rx", &in_rx);
        wave.add("tx", &out_rx);
        wave.add("ask", &ask_rx);
        wave.add("back", &ans_rx);
        wave.add("aw", &aw_rx);
        wave.add("ar", &ar_rx);
        wave.add("w", &w_rx);
        wave.add("b", &b_rx);
        wave.add("r", &r_rx);
        wave.add("link", &link);
        wave.add("master", &master);
        wave.start();
    }

    // A queue of beats behind the port, counting the transfers.
    let seen = Rc::new(RefCell::new(0u32));
    let count = seen.clone();
    let port = async move {
        let mut queue: VecDeque<u128> = VecDeque::new();
        loop {
            DefaultClock::rising().await;
            if aw_rx.peek().is_some() && w_rx.peek().is_some() {
                let a = aw_rx.recv().unwrap();
                queue.push_back(w_rx.recv().unwrap().data.raw());
                *count.borrow_mut() += 1;
                b_tx.send(B {
                    id: a.id,
                    resp: Resp::Okay,
                });
            }
            if let Some(a) = ar_rx.recv() {
                *count.borrow_mut() += 1;
                let (beat, resp) = match queue.pop_front() {
                    Some(v) => (v, Resp::Okay),
                    None => (0, Resp::SlvErr),
                };
                r_tx.send(R {
                    id: a.id,
                    data: U::from(beat),
                    resp,
                    last: Bit::One,
                });
            }
        }
    };
    let mut sim = Running::new(join2(
        join2(
            link.run((in_rx, ans_rx), (ask_tx, out_tx)),
            master.run((b_rx, r_rx, ask_rx), (aw_tx, ar_tx, w_tx, ans_tx)),
        ),
        port,
    ));

    // The program: one ask at a time, each waiting for its answer.
    let one = 0x0f0e0d0c_0b0a0908_07060504_03020100u128;
    let two = 0xfedcba98_76543210_00112233_44556677u128;
    let mut noop = ask_frame(9, false, 0, 0);
    noop[17] = 2;
    let asks = [
        ("no-op", noop),
        ("write one", ask_frame(1, true, 0x200, one)),
        ("write two", ask_frame(2, true, 0x200, two)),
        ("read", ask_frame(3, false, 0x200, 0)),
        ("read", ask_frame(4, false, 0x200, 0)),
        ("read, same tag", ask_frame(4, false, 0x200, 0)),
        ("read, empty", ask_frame(5, false, 0x200, 0)),
    ];
    let mut said = Vec::new();
    let mut frame: Vec<u8> = Vec::new();
    for (what, f) in asks.iter() {
        let mut i = 0;
        let mut answered = false;
        while !answered {
            if i < f.len() && in_tx.ready().to_bool() {
                in_tx.send(EthByte {
                    data: U::from(f[i]),
                    last: Bit::from(i == f.len() - 1),
                });
                i += 1;
            }
            if let Some(b) = out_rx.recv() {
                frame.push(b.data.raw() as u8);
                if b.last.to_bool() {
                    let f = &frame;
                    if f.len() >= STREAM_FRAME_LEN as usize
                        && f[14] == KIND_STREAM_ANSWER as u8
                    {
                        let mut b = [0u8; 16];
                        b.copy_from_slice(&f[26..42]);
                        said.push(format!(
                            "{:4}  {:14}  {:3}    {:032x}  {}  saw {}",
                            now(),
                            what,
                            f[16],
                            u128::from_le_bytes(b),
                            if f[17] & 1 == 1 { "error" } else { "ok   " },
                            *seen.borrow()
                        ));
                        answered = true;
                    }
                    frame.clear();
                }
            }
            sim.cycle();
        }
    }
    stop();
    println!(
        "   t  ask             tag    beat answered                     result"
    );
    for line in &said {
        println!("{line}");
    }
    let net = StreamLink::<DEVICE>::lowered("stream_link");
    let master = BeatMaster::lowered("beat_master");
    let bridge = StreamBridge::<DEVICE>::lowered("stream_bridge");
    txhdl::netlist::write_netlists_from_env(&[&net, &master, &bridge]);
}
