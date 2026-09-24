// SPDX-License-Identifier: Apache-2.0
//! One Ethernet wire, two users, split by EtherType.
//!
//! The board has one Ethernet port and two things that want it. This
//! run is the unit between them, with frames of both kinds arriving
//! and both users sending at once, and every frame checked where it
//! landed, whole and in order.
//!
//! Five frames arrive on the wire, and each case is there for a
//! reason:
//!
//! * two of the first user's type and one of another type, which is
//!   the split itself;
//! * one that ends after ten bytes, before its type is known, which
//!   must still go somewhere rather than hold the wire for ever;
//! * one of exactly fourteen bytes, whose last byte is also the second
//!   byte of its type, which is the one moment the unit learns the
//!   frame's user and that the frame is over in the same cycle.
//!
//! And both users send two frames each, offered in the same cycle, so
//! the frames on the wire must come out whole and taking turns.
use std::collections::VecDeque;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, Clock, DefaultClock, Mem, Reg, Running, Rx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::Trace;
use txhdl_parts::eth::EthByte;
use txhdl_parts::ethshare::EthShare;

/// The remote peripheral's EtherType, which IEEE leaves for local use.
const REMOTE: usize = 0x88b5;

/// Where one output's bytes are put, each with its frame's end, so a
/// run can be checked against what landed.
///
/// Not lowered: it is the test's end of a channel, not hardware.
#[derive(Trace, Default)]
pub struct Sink<const N: usize> {
    pub mem: Mem<U<9>, N>,
    pub at: Reg<U<10>>,
}

impl<const N: usize> Unit<Rx<EthByte>, ()> for Sink<N> {
    async fn run(&mut self, inp: Rx<EthByte>, _out: ()) {
        loop {
            DefaultClock::rising().await;
            if let Some(b) = inp.recv_if(Bit::One) {
                let v =
                    (b.data.raw() as u32) | ((b.last.to_bool() as u32) << 8);
                self.mem.write(self.at.get(), U::<9>::from(v));
                self.at.set(self.at.get() + 1);
            }
        }
    }
}

/// A frame of `n` bytes of type `ty`, every byte distinct and marked
/// by `tag`, so a byte that landed in the wrong frame says whose it
/// was. A frame shorter than fourteen bytes has no type.
fn frame(ty: u16, n: usize, tag: u8) -> Vec<u8> {
    let mut f: Vec<u8> = (0..n).map(|i| tag.wrapping_add(i as u8)).collect();
    if n >= 14 {
        f[12] = (ty >> 8) as u8;
        f[13] = ty as u8;
    }
    f
}

/// The frames a sink received, split at each marked last byte.
fn frames_in<const N: usize>(m: &Mem<U<9>, N>, count: u128) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for i in 0..count {
        let v = m.read(U::<10>::from(i as u32)).raw() as u32;
        cur.push((v & 0xff) as u8);
        if v & 0x100 != 0 {
            out.push(std::mem::take(&mut cur));
        }
    }
    assert!(cur.is_empty(), "a frame with no last byte: {cur:?}");
    out
}

/// A queue of bytes to feed a channel, a frame at a time, the last
/// byte of each marked.
fn queue(frames: &[Vec<u8>]) -> VecDeque<(u8, bool)> {
    let mut q = VecDeque::new();
    for f in frames {
        for (i, b) in f.iter().enumerate() {
            q.push_back((*b, i + 1 == f.len()));
        }
    }
    q
}

fn main() {
    let (wire_in_tx, wire_in) = chan::<EthByte, DefaultClock>();
    let (a_in_tx, a_in) = chan::<EthByte, DefaultClock>();
    let (b_in_tx, b_in) = chan::<EthByte, DefaultClock>();
    let (wire_out, wire_out_rx) = chan::<EthByte, DefaultClock>();
    let (a_out, a_out_rx) = chan::<EthByte, DefaultClock>();
    let (b_out, b_out_rx) = chan::<EthByte, DefaultClock>();

    // What arrives on the wire.
    let r0 = frame(REMOTE as u16, 20, 0x90);
    let r1 = frame(0x0800, 30, 0xa0);
    let r2 = frame(REMOTE as u16, 16, 0xb0);
    // Ends before its type is known, and ADVERSARIALLY so: its last
    // byte is 0xb5, the low half of the first user's type, and the
    // frame before it left 0x88 in the high half. A unit that read the
    // type without checking the frame had reached it would see 0x88b5
    // and send this to the wrong user. The first version of this run
    // used bytes that happened not to form that type, and passed with
    // the check removed.
    let r3 = frame(0, 10, 0xac);
    let r4 = frame(REMOTE as u16, 14, 0xd0); // ends on its type
                                             // What each user sends.
    let a0 = frame(REMOTE as u16, 20, 0x10);
    let a1 = frame(REMOTE as u16, 18, 0x30);
    let b0 = frame(0x0800, 25, 0x50);
    let b1 = frame(0x0806, 15, 0x70);

    let mut wq =
        queue(&[r0.clone(), r1.clone(), r2.clone(), r3.clone(), r4.clone()]);
    let mut aq = queue(&[a0.clone(), a1.clone()]);
    let mut bq = queue(&[b0.clone(), b1.clone()]);

    let mut share = EthShare::<REMOTE>::default();
    let mut to_a = Sink::<256>::default();
    let mut to_b = Sink::<256>::default();
    let mut to_wire = Sink::<256>::default();
    let (ma, ca) = (to_a.mem.clone(), to_a.at);
    let (mb, cb) = (to_b.mem.clone(), to_b.at);
    let (mw, cw) = (to_wire.mem.clone(), to_wire.at);

    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // Every channel under the name the unit's port has.
        w.add("wire_in", &wire_in);
        w.add("a_in", &a_in);
        w.add("b_in", &b_in);
        w.add("wire_out", &wire_out_rx);
        w.add("a_out", &a_out_rx);
        w.add("b_out", &b_out_rx);
        w.add("ethshare", &share);
        w.start();
    }

    let mut sim = Running::new(join2(
        share.run((wire_in, a_in, b_in), (wire_out, a_out, b_out)),
        join2(
            join2(to_a.run(a_out_rx, ()), to_b.run(b_out_rx, ())),
            to_wire.run(wire_out_rx, ()),
        ),
    ));

    // Every byte is offered from here, between steps rather than
    // inside one, so nothing races the unit within a cycle.
    let feed = |q: &mut VecDeque<(u8, bool)>, tx: &txhdl::comp::Tx<EthByte>| {
        if let Some(&(b, l)) = q.front() {
            if tx.ready().to_bool() {
                tx.send(EthByte {
                    data: U::<8>::from(b as u32),
                    last: Bit::from_bool(l),
                });
                q.pop_front();
            }
        }
    };
    for _ in 0..800 {
        feed(&mut wq, &wire_in_tx);
        feed(&mut aq, &a_in_tx);
        feed(&mut bq, &b_in_tx);
        sim.cycle();
    }

    let got_a = frames_in(&ma, ca.get().raw());
    let got_b = frames_in(&mb, cb.get().raw());
    let got_w = frames_in(&mw, cw.get().raw());
    println!(
        "the first user took {} frames, the second {}, the wire {}",
        got_a.len(),
        got_b.len(),
        got_w.len()
    );
    assert!(
        wq.is_empty() && aq.is_empty() && bq.is_empty(),
        "every byte was taken"
    );

    // The split: the first user's type to the first user, whole and
    // in order, including the frame that ended on its type.
    assert_eq!(got_a, vec![r0, r2, r4], "the first user's frames");
    // Everything else to the second, including the frame that ended
    // before it had a type.
    assert_eq!(got_b, vec![r1, r3], "the second user's frames");
    println!("each arriving frame reached its own user, whole");

    // The merge: four frames, whole, taking turns. The first user goes
    // first, then the one that did not send last.
    assert_eq!(
        got_w,
        vec![a0, b0, a1, b1],
        "the frames on the wire, in turn"
    );
    println!("both users' frames went out whole, taking turns");

    stop();
    txhdl::netlist::write_vhdl_from_env(&EthShare::<REMOTE>::lowered(
        "ethshare",
    ));
}
