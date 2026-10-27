// SPDX-License-Identifier: Apache-2.0
//! A program on another machine reads and writes a wide AXI4 port of
//! this design, one beat per frame (issue 1984).
//!
//! [`HostLink`](super::host::HostLink) and
//! [`LiteMaster`](super::host::LiteMaster) carry 32-bit words to an
//! AXI-Lite port. FireSim's CPU-managed streams need more: its host
//! driver moves them with `cpu_managed_axi4_read` and
//! `cpu_managed_axi4_write`, on an AXI4 port 128 bits wide with 64-bit
//! addresses. [`StreamLink`] and [`BeatMaster`] carry those, one
//! 16-byte beat per frame, and [`StreamBridge`] joins the two.
//!
//! The frames have their own EtherType, [`ETHERTYPE_STREAM`], the
//! other one IEEE leaves for local use. So
//! [`EthShare`](crate::ethshare::EthShare) can split them from
//! [`HostLink`](super::host::HostLink)'s frames by the type alone, and
//! each unit sees only its own.
//!
//! An ask frame, after the 14 bytes of the Ethernet header:
//!
//! | Byte | Field |
//! |---|---|
//! | 14 | the kind, [`KIND_STREAM_ASK`] |
//! | 15 | the device |
//! | 16 | the tag |
//! | 17 | bit 0: 1 for a write, 0 for a read; bit 1: a no-op |
//! | 18 to 25 | the address, most significant byte first |
//! | 26 to 41 | the beat, in memory order: byte 26 is lane 0 |
//! | 42 to 43 | the strobe, a bit per lane, most significant byte first |
//!
//! The answer, [`KIND_STREAM_ANSWER`], has the same layout. Its byte 17
//! is 1 if the transaction failed, and its beat is what a read read.
//!
//! A stream read takes data out of a queue and a stream write puts it
//! in, so neither may be performed twice. As with
//! [`LiteMaster`](super::host::LiteMaster), an ask that repeats the
//! tag of the last transfer is answered from what was kept of it,
//! the beat included.
//!
//! A program starts with a tag it can't know the board has not kept
//! from an earlier run, and no stream transfer is harmless to repeat
//! or to skip. So its first ask is a no-op, bit 1 of byte 17: the
//! master answers it without touching the port and keeps its tag as
//! the last one. Whether or not that tag was a repeat, every tag after
//! it differs from the one kept.
use txhdl::comp::{chan, join2, mux, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

use crate::bus::axi::{Ar, Aw, BurstKind, Resp, B, R, W};
use crate::eth::EthByte;

/// The EtherType of the stream frames: IEEE's second local
/// experimental type, next to the remote peripheral's `0x88b5`.
pub const ETHERTYPE_STREAM: u32 = 0x88b6;
/// The kind byte of a frame in which a program asks.
pub const KIND_STREAM_ASK: u32 = 0x03;
/// The kind byte of a frame that answers a program.
pub const KIND_STREAM_ANSWER: u32 = 0x83;
/// How many bytes of a frame are meaningful; the MAC pads it to 60.
pub const STREAM_FRAME_LEN: u32 = 44;

/// One beat a program asks for.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct BeatAsk {
    /// The transfer's sequence number, which its answer repeats.
    pub tag: U<8>,
    /// One for a write, zero for a read.
    pub write: Bit,
    /// The byte address of the beat.
    pub addr: U<64>,
    /// The beat to write, lane 0 in the low byte; zero on a read.
    pub data: U<128>,
    /// Which lanes the write covers; zero on a read.
    pub strb: U<16>,
    /// One for a no-op: answered and its tag kept, the port untouched.
    pub noop: Bit,
}

/// The answer to one [`BeatAsk`].
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct BeatAnswer {
    /// The tag of the transfer answered.
    pub tag: U<8>,
    /// The beat a read read; zero for a write.
    pub data: U<128>,
    /// One if the port answered with an error.
    pub err: Bit,
}

// begin{beatstate}
/// An AXI4 host that performs one [`BeatAsk`] at a time as a burst of
/// one beat, and answers each, repeating the last answer to an ask
/// that repeats its tag.
#[derive(Trace, Default)]
pub struct BeatMaster {
    /// Whether a transfer is on the port.
    pub busy: Reg<Bit>,
    /// Whether it is a write.
    pub writing: Reg<Bit>,
    /// Whether its address has gone, on `aw` or `ar`.
    pub addr_sent: Reg<Bit>,
    /// Whether a write's beat has gone, on `w`.
    pub data_sent: Reg<Bit>,
    /// Its tag.
    pub tag: Reg<U<8>>,
    /// Its address.
    pub addr: Reg<U<64>>,
    /// A write's beat.
    pub data: Reg<U<128>>,
    /// A write's strobe.
    pub strb: Reg<U<16>>,
    /// Whether an answer is waiting to go.
    pub answering: Reg<Bit>,
    /// Whether the last fields hold an answer.
    pub have_last: Reg<Bit>,
    /// The tag of the last transfer performed.
    pub last_tag: Reg<U<8>>,
    /// The beat it read; zero for a write.
    pub last_data: Reg<U<128>>,
    /// Whether it failed.
    pub last_err: Reg<Bit>,
}
// end{beatstate}

/// The address phase of a burst of one 16-byte beat at `addr`.
#[lower]
fn one_beat(addr: U<64>) -> Aw<64, 4> {
    Aw {
        id: U::<4>::from(0u8),
        addr,
        len: U::<8>::from(0u8),
        size: U::<3>::from(4u8),
        burst: BurstKind::Incr,
        lock: Bit::Zero,
        cache: U::<4>::from(0u8),
        prot: U::<3>::from(0u8),
        qos: U::<4>::from(0u8),
        region: U::<4>::from(0u8),
    }
}

// begin{beatrun}
#[lower]
impl Unit for BeatMaster {
    async fn run(
        &mut self,
        (b, r, ask): (Rx<B<4>>, Rx<R<128, 4>>, Rx<BeatAsk>),
        (aw, ar, w, back): (
            Tx<Aw<64, 4>>,
            Tx<Ar<64, 4>>,
            Tx<W<128, 16>>,
            Tx<BeatAnswer>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let busy = self.busy.get();
            let writing = self.writing.get();
            let addr_sent = self.addr_sent.get();
            let data_sent = self.data_sent.get();
            let answering = self.answering.get();
            // As LiteMaster: an ask is taken when the port is idle and
            // no answer waits; a repeat of the last tag is answered
            // again, any other ask is performed.
            let head = ask.head();
            let take = !busy & !answering & ask.peek().is_some();
            let _ = ask.recv_if(take);
            // A no-op is never a repeat: answered as one, it would bring
            // back the kept answer, an error included, at the start of a
            // run.
            let repeat = self.have_last.get()
                & (head.tag == self.last_tag.get())
                & !head.noop;
            let start = take & !repeat & !head.noop;
            let noop = take & head.noop;
            let send_aw = busy & writing & !addr_sent & aw.ready();
            let send_w = busy & writing & !data_sent & w.ready();
            let send_ar = busy & !writing & !addr_sent & ar.ready();
            // The response is taken only once the request has gone
            // whole.
            let bh = b.head();
            let rh = r.head();
            let got_b =
                busy & writing & addr_sent & data_sent & b.peek().is_some();
            let got_r = busy & !writing & addr_sent & r.peek().is_some();
            let _ = b.recv_if(got_b);
            let _ = r.recv_if(got_r);
            let done = got_b | got_r;
            let failed = mux(
                got_b,
                Bit::from(bh.resp != Resp::Okay),
                Bit::from(rh.resp != Resp::Okay),
            );
            let beat = mux(got_b, U::<128>::from(0u32), rh.data);
            let say = answering & back.ready();
            with!(self <= {
                start ? { busy: Bit::One, writing: head.write,
                          addr_sent: Bit::Zero, data_sent: Bit::Zero,
                          tag: head.tag, addr: head.addr,
                          data: head.data, strb: head.strb },
                take & repeat ? answering: Bit::One,
                noop ? { answering: Bit::One, have_last: Bit::One,
                         last_tag: head.tag, last_data: U::<128>::from(0u32),
                         last_err: Bit::Zero },
                send_aw | send_ar ? addr_sent: Bit::One,
                send_w ? data_sent: Bit::One,
                done ? { busy: Bit::Zero, answering: Bit::One,
                         have_last: Bit::One, last_tag: self.tag.get(),
                         last_data: beat, last_err: failed },
                say ? answering: Bit::Zero,
            });
            if send_aw.to_bool() {
                aw.send(one_beat(self.addr.get()));
            }
            if send_ar.to_bool() {
                ar.send(one_beat(self.addr.get()));
            }
            if send_w.to_bool() {
                w.send(W {
                    data: self.data.get(),
                    strb: self.strb.get(),
                    last: Bit::One,
                });
            }
            if say.to_bool() {
                back.send(BeatAnswer {
                    tag: self.last_tag.get(),
                    data: self.last_data.get(),
                    err: self.last_err.get(),
                });
            }
        }
    }
}
// end{beatrun}

// begin{streamstate}
/// The wire under a [`BeatMaster`]: asks in as frames, answers out as
/// frames. `DEV` is the device number, as for
/// [`HostLink`](super::host::HostLink).
#[derive(Trace, Default)]
pub struct StreamLink<const DEV: usize> {
    /// Which byte of the frame coming in is next; stops at 63.
    pub got: Reg<U<6>>,
    /// Whether a byte of the frame coming in has said it is not this
    /// device's ask.
    pub alien: Reg<Bit>,
    /// The ask as its bytes arrive.
    pub in_tag: Reg<U<8>>,
    /// Whether it is a write.
    pub in_write: Reg<Bit>,
    /// Whether it is a no-op.
    pub in_noop: Reg<Bit>,
    /// Its address, shifted in a byte at a time.
    pub in_addr: Reg<U<64>>,
    /// Its beat, shifted in from the top so lane 0 ends lowest.
    pub in_data: Reg<U<128>>,
    /// Its strobe.
    pub in_strb: Reg<U<16>>,
    /// Whether an answer frame is going out.
    pub sending: Reg<Bit>,
    /// Which byte of it goes next.
    pub at: Reg<U<6>>,
    /// The answer being sent.
    pub out_tag: Reg<U<8>>,
    /// Its beat.
    pub out_data: Reg<U<128>>,
    /// Whether it says the transfer failed.
    pub out_err: Reg<Bit>,
}
// end{streamstate}

/// The byte at `at` of the frame that carries an answer.
#[lower]
fn stream_answer_byte(
    at: U<6>,
    dev: U<8>,
    tag: U<8>,
    data: U<128>,
    err: Bit,
) -> U<8> {
    let ff = U::<8>::from(0xffu8);
    let zero = U::<8>::from(0u8);
    select!(at.raw() => {
        0..=5 => ff,
        6 => U::<8>::from(0x02u8),
        7..=10 => zero,
        11 => dev,
        12 => U::<8>::from(0x88u8),
        13 => U::<8>::from(0xb6u8),
        14 => U::<8>::from(KIND_STREAM_ANSWER),
        15 => dev,
        16 => tag,
        17 => err.zext::<8>(),
        26 => data.slice::<0, 8>(),
        27 => data.slice::<8, 8>(),
        28 => data.slice::<16, 8>(),
        29 => data.slice::<24, 8>(),
        30 => data.slice::<32, 8>(),
        31 => data.slice::<40, 8>(),
        32 => data.slice::<48, 8>(),
        33 => data.slice::<56, 8>(),
        34 => data.slice::<64, 8>(),
        35 => data.slice::<72, 8>(),
        36 => data.slice::<80, 8>(),
        37 => data.slice::<88, 8>(),
        38 => data.slice::<96, 8>(),
        39 => data.slice::<104, 8>(),
        40 => data.slice::<112, 8>(),
        41 => data.slice::<120, 8>(),
        _ => zero,
    })
}

// begin{streamrun}
#[lower]
impl<const DEV: usize> Unit for StreamLink<DEV> {
    async fn run(
        &mut self,
        (rx, back): (Rx<EthByte>, Rx<BeatAnswer>),
        (ask, tx): (Tx<BeatAsk>, Tx<EthByte>),
    ) {
        loop {
            DefaultClock::rising().await;
            let dev = U::<8>::from(DEV as u32);
            let got = self.got.get();
            let alien = self.alien.get();
            let inb = rx.head();
            let took = rx.peek().is_some();
            let _ = rx.recv_if(took);
            let value = inb.data;
            let bad = (got == 12) & (value != U::<8>::from(0x88u8))
                | (got == 13) & (value != U::<8>::from(0xb6u8))
                | (got == 14) & (value != U::<8>::from(KIND_STREAM_ASK))
                | (got == 15) & (value != dev);
            // As HostLink's: a frame that ends by byte 43 is not a whole
            // ask, since the strobe's last byte is held a cycle after
            // it arrives. The MAC pads every frame to sixty bytes, so a
            // real ask always goes on past it.
            let short = took & inb.last & (got < STREAM_FRAME_LEN);
            let ends = took & inb.last;
            let asking = ends & !alien & !bad & !short & ask.ready();
            // The multi-byte fields are shift registers: the address
            // and the strobe in from the bottom, most significant byte
            // first; the beat in from the top, lane 0 first.
            let in_addr = (18..=25).contains(&got.raw());
            let in_data = (26..=41).contains(&got.raw());
            let in_strb = (42..=43).contains(&got.raw());
            let sending = self.sending.get();
            let at = self.at.get();
            let ah = back.head();
            let take = !sending & back.peek().is_some();
            let _ = back.recv_if(take);
            let byte = stream_answer_byte(
                at,
                dev,
                self.out_tag.get(),
                self.out_data.get(),
                self.out_err.get(),
            );
            let put = sending & tx.ready();
            let last = at == STREAM_FRAME_LEN - 1;
            with!(self <= {
                took & !inb.last & (got != 63) ? got: got + 1,
                took & inb.last ? got: 0,
                took & bad ? alien: Bit::One,
                took & inb.last ? alien: Bit::Zero,
                took & (got == 16) ? in_tag: value,
                took & (got == 17) ? { in_write: value.bit(0),
                                       in_noop: value.bit(1) },
                took & in_addr ? in_addr:
                    self.in_addr.get().slice::<0, 56>().concat::<_, 64>(value),
                took & in_data ? in_data:
                    value.concat::<_, 128>(self.in_data.get().slice::<8, 120>()),
                took & in_strb ? in_strb:
                    self.in_strb.get().slice::<0, 8>().concat::<_, 16>(value),
                take ? { sending: Bit::One, at: 0, out_tag: ah.tag,
                         out_data: ah.data, out_err: ah.err },
                put & !last ? at: at + 1,
                put & last ? { sending: Bit::Zero, at: 0 },
            });
            if asking.to_bool() {
                ask.send(BeatAsk {
                    tag: self.in_tag.get(),
                    write: self.in_write.get(),
                    noop: self.in_noop.get(),
                    addr: self.in_addr.get(),
                    data: self.in_data.get(),
                    strb: self.in_strb.get(),
                });
            }
            if put.to_bool() {
                tx.send(EthByte {
                    data: byte,
                    last: Bit::from(last),
                });
            }
        }
    }
}
// end{streamrun}

// begin{streambridge}
/// [`StreamLink`] and [`BeatMaster`] as one unit: frames in and out on
/// one side, the AXI4 port on the other. Joined inside the unit for
/// the reason [`HostBridge`](super::host::HostBridge) is.
#[derive(Trace, Default)]
pub struct StreamBridge<const DEV: usize> {
    /// The frames.
    pub link: StreamLink<DEV>,
    /// The port.
    pub master: BeatMaster,
}

#[lower]
impl<const DEV: usize> Unit for StreamBridge<DEV> {
    async fn run(
        &mut self,
        (rx, b, r): (Rx<EthByte>, Rx<B<4>>, Rx<R<128, 4>>),
        (tx, aw, ar, w): (
            Tx<EthByte>,
            Tx<Aw<64, 4>>,
            Tx<Ar<64, 4>>,
            Tx<W<128, 16>>,
        ),
    ) {
        let (ask_tx, ask_rx) = chan::<BeatAsk, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<BeatAnswer, DefaultClock>();
        join2(
            self.link.run((rx, ans_rx), (ask_tx, tx)),
            self.master.run((b, r, ask_rx), (aw, ar, w, ans_tx)),
        )
        .await;
    }
}
// end{streambridge}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use txhdl::comp::Running;

    /// An ask frame, padded as the MAC pads.
    fn ask_frame(
        dev: u8,
        tag: u8,
        write: bool,
        addr: u64,
        beat: u128,
        strb: u16,
    ) -> Vec<u8> {
        let mut f = vec![0xffu8; 6];
        f.extend_from_slice(&[0x02, 0, 0, 0, 0, 0x10]);
        f.extend_from_slice(&[0x88, 0xb6, KIND_STREAM_ASK as u8, dev, tag]);
        f.push(write as u8);
        f.extend_from_slice(&addr.to_be_bytes());
        f.extend_from_slice(&beat.to_le_bytes());
        f.extend_from_slice(&strb.to_be_bytes());
        f.resize(60, 0);
        f
    }

    /// What one answer frame says: device, tag, beat and error.
    fn answer_of(f: &[u8]) -> Option<(u8, u8, u128, bool)> {
        if f.len() < STREAM_FRAME_LEN as usize
            || f[12] != 0x88
            || f[13] != 0xb6
            || f[14] != KIND_STREAM_ANSWER as u8
        {
            return None;
        }
        let mut b = [0u8; 16];
        b.copy_from_slice(&f[26..42]);
        Some((f[15], f[16], u128::from_le_bytes(b), f[17] & 1 == 1))
    }

    /// What a port in front of a stream saw.
    #[derive(Default)]
    struct Seen {
        /// Each write: address, beat, strobe.
        writes: Vec<(u64, u128, u16)>,
        /// Each read's address.
        reads: Vec<u64>,
        /// Each address phase's length and size.
        shapes: Vec<(u8, u8)>,
    }

    /// The frames sent to a `StreamBridge` with a queue behind it: a
    /// read takes the next beat of `queue` and a write records itself.
    /// An address at or above 0x1000 answers `SlvErr`.
    fn serve(
        frames: &[Vec<u8>],
        queue: Vec<u128>,
    ) -> (Vec<(u8, u8, u128, bool)>, Rc<RefCell<Seen>>) {
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
        let (aw_tx, aw_rx) = chan::<Aw<64, 4>, DefaultClock>();
        let (ar_tx, ar_rx) = chan::<Ar<64, 4>, DefaultClock>();
        let (w_tx, w_rx) = chan::<W<128, 16>, DefaultClock>();
        let (b_tx, b_rx) = chan::<B<4>, DefaultClock>();
        let (r_tx, r_rx) = chan::<R<128, 4>, DefaultClock>();
        let mut bridge = StreamBridge::<5>::default();
        let seen = Rc::new(RefCell::new(Seen::default()));
        let log = seen.clone();
        let port = async move {
            let mut q = queue.into_iter();
            let mut _mem: HashMap<u64, u128> = HashMap::new();
            loop {
                DefaultClock::rising().await;
                if aw_rx.peek().is_some() && w_rx.peek().is_some() {
                    let a = aw_rx.recv().unwrap();
                    let wb = w_rx.recv().unwrap();
                    let addr = a.addr.raw() as u64;
                    let mut s = log.borrow_mut();
                    s.shapes.push((a.len.raw() as u8, a.size.raw() as u8));
                    s.writes.push((addr, wb.data.raw(), wb.strb.raw() as u16));
                    b_tx.send(B {
                        id: a.id,
                        resp: if addr < 0x1000 {
                            Resp::Okay
                        } else {
                            Resp::SlvErr
                        },
                    });
                }
                if let Some(a) = ar_rx.recv() {
                    let addr = a.addr.raw() as u64;
                    let mut s = log.borrow_mut();
                    s.shapes.push((a.len.raw() as u8, a.size.raw() as u8));
                    s.reads.push(addr);
                    let ok = addr < 0x1000;
                    let beat = if ok { q.next().unwrap_or(0) } else { 0 };
                    r_tx.send(R {
                        id: a.id,
                        data: U::from(beat),
                        resp: if ok { Resp::Okay } else { Resp::SlvErr },
                        last: Bit::One,
                    });
                }
            }
        };
        let mut sim = Running::new(join2(
            bridge.run((in_rx, b_rx, r_rx), (out_tx, aw_tx, ar_tx, w_tx)),
            port,
        ));
        let mut answers = Vec::new();
        let mut cur: Vec<u8> = Vec::new();
        for f in frames {
            let mut i = 0;
            let before = answers.len();
            for _ in 0..400 {
                if i < f.len() && in_tx.ready().to_bool() {
                    in_tx.send(EthByte {
                        data: U::from(f[i]),
                        last: Bit::from(i == f.len() - 1),
                    });
                    i += 1;
                }
                if let Some(b) = out_rx.recv() {
                    cur.push(b.data.raw() as u8);
                    if b.last.to_bool() {
                        if let Some(a) = answer_of(&cur) {
                            answers.push(a);
                        }
                        cur.clear();
                    }
                }
                sim.cycle();
                if i == f.len() && answers.len() > before {
                    break;
                }
            }
        }
        (answers, seen)
    }

    /// A no-op ask.
    fn noop_frame(tag: u8) -> Vec<u8> {
        let mut f = ask_frame(5, tag, false, 0, 0, 0);
        f[17] = 2;
        f
    }

    const BEAT: u128 = 0x0f0e0d0c_0b0a0908_07060504_03020100;

    #[test]
    fn a_write_reaches_the_port_whole() {
        let addr = 0x0000_0000_0000_0230u64;
        let (got, seen) =
            serve(&[ask_frame(5, 1, true, addr, BEAT, 0xffff)], vec![]);
        assert_eq!(got, vec![(5, 1, 0, false)]);
        let s = seen.borrow();
        assert_eq!(s.writes, vec![(addr, BEAT, 0xffff)]);
        assert_eq!(s.shapes, vec![(0, 4)], "one beat of 16 bytes");
    }

    #[test]
    fn reads_take_the_queue_in_order() {
        let (got, seen) = serve(
            &[
                ask_frame(5, 1, false, 0x200, 0, 0),
                ask_frame(5, 2, false, 0x200, 0, 0),
            ],
            vec![BEAT, !BEAT],
        );
        assert_eq!(got, vec![(5, 1, BEAT, false), (5, 2, !BEAT, false)]);
        assert_eq!(seen.borrow().reads, vec![0x200, 0x200]);
    }

    #[test]
    fn a_repeated_read_is_answered_again_and_not_performed_again() {
        // The program lost the answer and asks again: the queue must
        // not lose a beat.
        let r = ask_frame(5, 7, false, 0x200, 0, 0);
        let (got, seen) = serve(
            &[r.clone(), r, ask_frame(5, 8, false, 0x200, 0, 0)],
            vec![BEAT, !BEAT],
        );
        assert_eq!(
            got,
            vec![
                (5, 7, BEAT, false),
                (5, 7, BEAT, false),
                (5, 8, !BEAT, false)
            ]
        );
        assert_eq!(seen.borrow().reads.len(), 2, "the repeat was not read");
    }

    #[test]
    fn a_repeated_write_is_not_performed_again() {
        let wf = ask_frame(5, 3, true, 0x100, BEAT, 0xffff);
        let (got, seen) = serve(&[wf.clone(), wf], vec![]);
        assert_eq!(got, vec![(5, 3, 0, false), (5, 3, 0, false)]);
        assert_eq!(seen.borrow().writes.len(), 1);
    }

    #[test]
    fn a_partial_write_keeps_its_strobe() {
        let (_, seen) =
            serve(&[ask_frame(5, 1, true, 0x40, BEAT, 0x00f1)], vec![]);
        assert_eq!(seen.borrow().writes[0].2, 0x00f1);
    }

    #[test]
    fn a_wide_address_arrives_whole() {
        let addr = 0x1234_5678_9abc_def0u64;
        let (got, seen) =
            serve(&[ask_frame(5, 1, false, addr, 0, 0)], vec![BEAT]);
        assert_eq!(got, vec![(5, 1, 0, true)], "out of range: an error");
        assert_eq!(seen.borrow().reads, vec![addr]);
    }

    #[test]
    fn other_frames_are_ignored() {
        // Another device, the control frames' type, and a short frame.
        let other = ask_frame(6, 1, true, 0x40, BEAT, 0xffff);
        let mut control = ask_frame(5, 1, true, 0x40, BEAT, 0xffff);
        control[13] = 0xb5;
        let mut short = ask_frame(5, 1, true, 0x40, BEAT, 0xffff);
        short.truncate(STREAM_FRAME_LEN as usize);
        let good = ask_frame(5, 9, false, 0x40, 0, 0);
        let (got, seen) = serve(&[other, control, short, good], vec![BEAT]);
        assert_eq!(got, vec![(5, 9, BEAT, false)]);
        assert!(seen.borrow().writes.is_empty());
    }

    #[test]
    fn a_noop_is_answered_and_never_reaches_the_port() {
        // The first ask of a run: whatever tag the master kept, the
        // reads after the no-op are performed.
        let (got, seen) = serve(
            &[
                noop_frame(9),
                ask_frame(5, 10, false, 0x200, 0, 0),
                noop_frame(10),
                ask_frame(5, 11, false, 0x200, 0, 0),
            ],
            vec![BEAT, !BEAT],
        );
        assert_eq!(
            got,
            vec![
                (5, 9, 0, false),
                (5, 10, BEAT, false),
                (5, 10, 0, false),
                (5, 11, !BEAT, false)
            ],
            "a no-op on the kept tag is a no-op, not the repeat"
        );
        assert_eq!(seen.borrow().reads, vec![0x200, 0x200]);
        assert!(seen.borrow().writes.is_empty());
    }

    #[test]
    fn a_noop_on_a_failed_transfers_tag_does_not_bring_back_the_error() {
        // The last run ended on a refused read under tag 4; the next
        // run's no-op happens to draw tag 4.
        let (got, _) = serve(
            &[
                ask_frame(5, 4, false, 0x2000, 0, 0),
                noop_frame(4),
                ask_frame(5, 5, false, 0x200, 0, 0),
            ],
            vec![BEAT],
        );
        assert_eq!(
            got,
            vec![(5, 4, 0, true), (5, 4, 0, false), (5, 5, BEAT, false)]
        );
    }
}
