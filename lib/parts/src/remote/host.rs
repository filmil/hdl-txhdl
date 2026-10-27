// SPDX-License-Identifier: Apache-2.0
//! The other direction of [`RemoteLink`](super::eth::RemoteLink): a
//! program on another machine reads and writes this design's bus.
//!
//! [`RemoteLink`](super::eth::RemoteLink) carries the board's
//! transactions out to a program that answers them. [`HostLink`] and
//! [`LiteMaster`] carry a program's transactions in: the program sends
//! an [`Ask`](super::Ask) in a frame, [`LiteMaster`] performs it as an
//! AXI-Lite host on the design's bus, and the [`Answer`](super::Answer)
//! goes back in a frame. What FireSim's host driver does over PCIe,
//! reading and writing the simulator's control registers, it can do
//! over this instead (issue 1931).
//!
//! The frame is [`RemoteLink`](super::eth::RemoteLink)'s, at the same
//! offsets, with two kinds of its own: [`KIND_HOST_ASK`] for an ask
//! from the program and [`KIND_HOST_ANSWER`] for the answer. So the
//! two directions can share a wire and a program, and neither mistakes
//! the other's frames for its own.
//!
//! A frame can be lost, so the program sends an ask again when its
//! answer does not come. [`LiteMaster`] keeps the tag and the answer
//! of the last transaction it performed, and an ask with that tag again
//! is answered from them rather than performed twice: a write repeated
//! on a register with a side effect would do the effect twice.
use txhdl::comp::{chan, join2, mux, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use super::eth::{ETHERTYPE, FRAME_LEN};
use super::{Answer, Ask};
use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteAw, LiteHostPort, LiteW};
use crate::eth::EthByte;

/// The kind byte of a frame in which a program asks.
pub const KIND_HOST_ASK: u32 = 0x02;
/// The kind byte of a frame that answers a program.
pub const KIND_HOST_ANSWER: u32 = 0x82;

// The type's two bytes, as the frame carries them.
const _: () = assert!(ETHERTYPE == 0x88b5);

// begin{masterstate}
/// An AXI-Lite host that performs one [`Ask`](super::Ask) at a time
/// and answers each, repeating the last answer to an ask that repeats
/// its tag.
#[derive(Trace, Default)]
pub struct LiteMaster {
    /// Whether a transaction is on the bus.
    pub busy: Reg<Bit>,
    /// Whether it is a write.
    pub writing: Reg<Bit>,
    /// Whether its address has gone, on `aw` or `ar`.
    pub addr_sent: Reg<Bit>,
    /// Whether a write's word has gone, on `w`.
    pub data_sent: Reg<Bit>,
    /// Its tag.
    pub tag: Reg<U<8>>,
    /// Its address.
    pub addr: Reg<U<32>>,
    /// A write's word.
    pub data: Reg<U<32>>,
    /// A write's strobe.
    pub strb: Reg<U<4>>,
    /// Whether an answer is waiting to go.
    pub answering: Reg<Bit>,
    /// Whether the last fields hold an answer: a register starts at
    /// zero, and before the first transaction no tag is a repeat.
    pub have_last: Reg<Bit>,
    /// The tag of the last transaction performed.
    pub last_tag: Reg<U<8>>,
    /// The word it read; zero for a write.
    pub last_data: Reg<U<32>>,
    /// Whether it failed.
    pub last_err: Reg<Bit>,
}
// end{masterstate}

// begin{masterrun}
#[lower]
impl Unit for LiteMaster {
    async fn run(
        &mut self,
        bus: LiteHostPort<32, 32, 4>,
        (ask, back): (Rx<Ask>, Tx<Answer>),
    ) {
        loop {
            DefaultClock::rising().await;
            let busy = self.busy.get();
            let writing = self.writing.get();
            let addr_sent = self.addr_sent.get();
            let data_sent = self.data_sent.get();
            let answering = self.answering.get();
            // An ask is taken when nothing is on the bus and nothing is
            // waiting to be answered. One that repeats the last tag is
            // answered again; any other is performed.
            let head = ask.head();
            let take = !busy & !answering & ask.peek().is_some();
            let _ = ask.recv_if(take);
            let repeat =
                self.have_last.get() & (head.tag == self.last_tag.get());
            let start = take & !repeat;
            // The address and, for a write, the word, each sent once,
            // each when its channel has room.
            let send_aw = busy & writing & !addr_sent & bus.aw.ready();
            let send_w = busy & writing & !data_sent & bus.w.ready();
            let send_ar = busy & !writing & !addr_sent & bus.ar.ready();
            // The response ends the transaction. It is taken only once
            // the request has gone whole, so a response from a bus that
            // misbehaves can't end a write whose word is still waiting.
            let bh = bus.b.head();
            let rh = bus.r.head();
            let got_b =
                busy & writing & addr_sent & data_sent & bus.b.peek().is_some();
            let got_r = busy & !writing & addr_sent & bus.r.peek().is_some();
            let _ = bus.b.recv_if(got_b);
            let _ = bus.r.recv_if(got_r);
            let done = got_b | got_r;
            let failed = mux(
                got_b,
                Bit::from(bh.resp != Resp::Okay),
                Bit::from(rh.resp != Resp::Okay),
            );
            let word = mux(got_b, U::<32>::from(0u32), rh.data);
            let say = answering & back.ready();
            with!(self <= {
                start ? { busy: Bit::One, writing: head.write,
                          addr_sent: Bit::Zero, data_sent: Bit::Zero,
                          tag: head.tag, addr: head.addr,
                          data: head.data, strb: head.strb },
                take & repeat ? answering: Bit::One,
                send_aw | send_ar ? addr_sent: Bit::One,
                send_w ? data_sent: Bit::One,
                done ? { busy: Bit::Zero, answering: Bit::One,
                         have_last: Bit::One, last_tag: self.tag.get(),
                         last_data: word, last_err: failed },
                say ? answering: Bit::Zero,
            });
            if send_aw.to_bool() {
                bus.aw.send(LiteAw {
                    addr: self.addr.get(),
                    prot: U::<3>::from(0u8),
                });
            }
            if send_w.to_bool() {
                bus.w.send(LiteW {
                    data: self.data.get(),
                    strb: self.strb.get(),
                });
            }
            if send_ar.to_bool() {
                bus.ar.send(LiteAw {
                    addr: self.addr.get(),
                    prot: U::<3>::from(0u8),
                });
            }
            if say.to_bool() {
                back.send(Answer {
                    tag: self.last_tag.get(),
                    data: self.last_data.get(),
                    err: self.last_err.get(),
                });
            }
        }
    }
}
// end{masterrun}

// begin{linkstate}
/// The wire under a [`LiteMaster`]: asks in as frames, answers out as
/// frames.
///
/// `DEV` is the device number, which every ask this accepts must carry
/// and every answer it sends carries, as for
/// [`RemoteLink`](super::eth::RemoteLink).
#[derive(Trace, Default)]
pub struct HostLink<const DEV: usize> {
    /// Which byte of the frame coming in is next. It stops at 63
    /// rather than wrapping, so a long frame's later bytes are never
    /// read as its header or its fields again: frames longer than 64
    /// bytes (ARP, IPv6, LLDP) are ordinary on a wire.
    pub got: Reg<U<6>>,
    /// Whether a byte of the frame coming in has said it is not this
    /// device's ask. Held the polarity round, as `RemoteLink`'s is.
    pub alien: Reg<Bit>,
    /// The ask as its bytes arrive.
    pub in_tag: Reg<U<8>>,
    /// Whether it is a write.
    pub in_write: Reg<Bit>,
    /// Its address.
    pub in_addr: Reg<U<32>>,
    /// Its word.
    pub in_data: Reg<U<32>>,
    /// Its strobe.
    pub in_strb: Reg<U<4>>,
    /// Whether an answer frame is going out.
    pub sending: Reg<Bit>,
    /// Which byte of it goes next.
    pub at: Reg<U<6>>,
    /// The answer being sent.
    pub out_tag: Reg<U<8>>,
    /// Its word.
    pub out_data: Reg<U<32>>,
    /// Whether it says the transaction failed.
    pub out_err: Reg<Bit>,
}
// end{linkstate}

/// The byte at `at` of the frame that carries an answer to a program.
#[lower]
fn answer_byte(at: U<6>, dev: U<8>, tag: U<8>, data: U<32>, err: Bit) -> U<8> {
    let ff = U::<8>::from(0xffu8);
    let zero = U::<8>::from(0u8);
    select!(at.raw() => {
        // To every port, since the design does not know where the
        // program is; on a cable to one machine that is the machine.
        0..=5 => ff,
        6 => U::<8>::from(0x02u8),
        7..=10 => zero,
        11 => dev,
        12 => U::<8>::from(0x88u8),
        13 => U::<8>::from(0xb5u8),
        14 => U::<8>::from(KIND_HOST_ANSWER),
        15 => dev,
        16 => tag,
        17 => err.zext::<8>(),
        22 => data.slice::<24, 8>(),
        23 => data.slice::<16, 8>(),
        24 => data.slice::<8, 8>(),
        25 => data.slice::<0, 8>(),
        _ => zero,
    })
}

// begin{linkrun}
#[lower]
impl<const DEV: usize> Unit for HostLink<DEV> {
    async fn run(
        &mut self,
        (rx, back): (Rx<EthByte>, Rx<Answer>),
        (ask, tx): (Tx<Ask>, Tx<EthByte>),
    ) {
        loop {
            DefaultClock::rising().await;
            let dev = U::<8>::from(DEV as u32);
            // In. Every byte is taken, and what makes a frame this
            // device's ask is checked as it goes.
            let got = self.got.get();
            let alien = self.alien.get();
            let inb = rx.head();
            let took = rx.peek().is_some();
            let _ = rx.recv_if(took);
            let value = inb.data;
            let bad = (got == 12) & (value != U::<8>::from(0x88u8))
                | (got == 13) & (value != U::<8>::from(0xb5u8))
                | (got == 14) & (value != U::<8>::from(KIND_HOST_ASK))
                | (got == 15) & (value != dev);
            // A frame that ends by byte 26 is not a whole ask: its last
            // field, the strobe, is held a cycle after its byte, so the
            // frame must go on past it. The MAC pads every frame to
            // sixty bytes, so a real ask always does.
            let short = took & inb.last & (got < FRAME_LEN);
            let ends = took & inb.last;
            // The ask goes on when the frame ends whole and the channel
            // has room; with no room it is dropped, and the program
            // asks again, which a channel of one keeps rare.
            let asking = ends & !alien & !bad & !short & ask.ready();
            // Out. An answer is taken when no frame is going out, and
            // one byte leaves per cycle the wire has room.
            let sending = self.sending.get();
            let at = self.at.get();
            let ah = back.head();
            let take = !sending & back.peek().is_some();
            let _ = back.recv_if(take);
            let byte = answer_byte(
                at,
                dev,
                self.out_tag.get(),
                self.out_data.get(),
                self.out_err.get(),
            );
            let put = sending & tx.ready();
            let last = at == FRAME_LEN - 1;
            with!(self <= {
                took & !inb.last & (got != 63) ? got: got + 1,
                took & inb.last ? got: 0,
                took & bad ? alien: Bit::One,
                took & inb.last ? alien: Bit::Zero,
                took & (got == 16) ? in_tag: value,
                took & (got == 17) ? in_write: value.bit(0),
                took & (got == 18) ? in_addr:
                    value.concat::<_, 32>(self.in_addr.get().slice::<0, 24>()),
                took & (got == 19) ? in_addr:
                    self.in_addr.get().slice::<24, 8>()
                        .concat::<_, 16>(value)
                        .concat::<_, 32>(self.in_addr.get().slice::<0, 16>()),
                took & (got == 20) ? in_addr:
                    self.in_addr.get().slice::<16, 16>()
                        .concat::<_, 24>(value)
                        .concat::<_, 32>(self.in_addr.get().slice::<0, 8>()),
                took & (got == 21) ? in_addr:
                    self.in_addr.get().slice::<8, 24>().concat::<_, 32>(value),
                took & (got == 22) ? in_data:
                    value.concat::<_, 32>(self.in_data.get().slice::<0, 24>()),
                took & (got == 23) ? in_data:
                    self.in_data.get().slice::<24, 8>()
                        .concat::<_, 16>(value)
                        .concat::<_, 32>(self.in_data.get().slice::<0, 16>()),
                took & (got == 24) ? in_data:
                    self.in_data.get().slice::<16, 16>()
                        .concat::<_, 24>(value)
                        .concat::<_, 32>(self.in_data.get().slice::<0, 8>()),
                took & (got == 25) ? in_data:
                    self.in_data.get().slice::<8, 24>().concat::<_, 32>(value),
                took & (got == 26) ? in_strb: value.slice::<0, 4>(),
                take ? { sending: Bit::One, at: 0, out_tag: ah.tag,
                         out_data: ah.data, out_err: ah.err },
                put & !last ? at: at + 1,
                put & last ? { sending: Bit::Zero, at: 0 },
            });
            if asking.to_bool() {
                ask.send(Ask {
                    tag: self.in_tag.get(),
                    write: self.in_write.get(),
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
// end{linkrun}

// begin{bridge}
/// [`HostLink`] and [`LiteMaster`] as one unit: frames in and out on one
/// side, an AXI-Lite host on the other.
///
/// The two are joined by their `ask` and `back` channels inside the
/// unit, where the lowering buffers them. Joined by wires instead, in a
/// hand-written top, each channel's valid would depend on its own ready
/// through the other unit, a combinational loop (issue 1931 met it on
/// the board).
#[derive(Trace, Default)]
pub struct HostBridge<const DEV: usize> {
    /// The frames.
    pub link: HostLink<DEV>,
    /// The bus.
    pub master: LiteMaster,
}

#[lower]
impl<const DEV: usize> Unit for HostBridge<DEV> {
    async fn run(
        &mut self,
        rx: Rx<EthByte>,
        (tx, bus): (Tx<EthByte>, LiteHostPort<32, 32, 4>),
    ) {
        let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<Answer, DefaultClock>();
        join2(
            self.link.run((rx, ans_rx), (ask_tx, tx)),
            self.master.run(bus, (ask_rx, ans_tx)),
        )
        .await;
    }
}
// end{bridge}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi_lite::{LiteB, LitePort, LiteR};
    use crate::remote::eth::KIND_ASK;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use txhdl::comp::{chan, join2, link, Running};

    /// A frame in which a program asks, padded as the MAC pads.
    fn ask_frame(
        dev: u8,
        tag: u8,
        write: bool,
        addr: u32,
        data: u32,
    ) -> Vec<u8> {
        let mut f = vec![0xffu8; 6];
        f.extend_from_slice(&[0x02, 0, 0, 0, 0, 0x10]);
        f.extend_from_slice(&[0x88, 0xb5, KIND_HOST_ASK as u8, dev, tag]);
        f.push(write as u8);
        f.extend_from_slice(&addr.to_be_bytes());
        f.extend_from_slice(&data.to_be_bytes());
        f.push(if write { 0xf } else { 0 });
        f.resize(60, 0);
        f
    }

    /// What one answer frame says: its device, tag, word and error,
    /// or nothing if it is not an answer to a program.
    fn answer_of(f: &[u8]) -> Option<(u8, u8, u32, bool)> {
        if f.len() < FRAME_LEN as usize
            || f[12] != 0x88
            || f[13] != 0xb5
            || f[14] != KIND_HOST_ANSWER as u8
        {
            return None;
        }
        let data = u32::from_be_bytes([f[22], f[23], f[24], f[25]]);
        Some((f[15], f[16], data, f[17] & 1 == 1))
    }

    /// The frames sent to a design with the link, the master and a
    /// memory of words behind them, and the answers that come back.
    /// The memory answers `SlvErr` at and above 0x1000. Returns the
    /// answers and how many writes the memory saw.
    fn serve(frames: &[Vec<u8>]) -> (Vec<(u8, u8, u32, bool)>, usize) {
        let (answers, writes, _) = serve_with(frames, 0);
        (answers, writes)
    }

    /// As `serve`, with a memory that takes `delay` cycles to answer,
    /// and the strobe of every write it saw.
    fn serve_with(
        frames: &[Vec<u8>],
        delay: u32,
    ) -> (Vec<(u8, u8, u32, bool)>, usize, Vec<u8>) {
        let (host, per) = link::<LitePort![ADDR = 32, DATA = 32, STRB = 4]>();
        let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<Answer, DefaultClock>();
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
        let mut wire = HostLink::<5>::default();
        let mut master = LiteMaster::default();
        let writes = Rc::new(RefCell::new(0usize));
        let wcount = writes.clone();
        let strobes = Rc::new(RefCell::new(Vec::new()));
        let wstrb = strobes.clone();
        // A memory of words, as a peripheral on the bus.
        let memory = async move {
            let mut words: HashMap<u32, u32> = HashMap::new();
            loop {
                DefaultClock::rising().await;
                if per.aw.peek().is_some() && per.w.peek().is_some() {
                    let a = per.aw.recv().unwrap().addr.raw() as u32;
                    let wb = per.w.recv().unwrap();
                    let w = wb.data.raw() as u32;
                    wstrb.borrow_mut().push(wb.strb.raw() as u8);
                    *wcount.borrow_mut() += 1;
                    for _ in 0..delay {
                        DefaultClock::rising().await;
                    }
                    let ok = a < 0x1000;
                    if ok {
                        words.insert(a, w);
                    }
                    per.b.send(LiteB {
                        resp: if ok { Resp::Okay } else { Resp::SlvErr },
                    });
                }
                if let Some(ar) = per.ar.recv() {
                    for _ in 0..delay {
                        DefaultClock::rising().await;
                    }
                    let a = ar.addr.raw() as u32;
                    let ok = a < 0x1000;
                    per.r.send(LiteR {
                        data: U::from(*words.get(&a).unwrap_or(&0)),
                        resp: if ok { Resp::Okay } else { Resp::SlvErr },
                    });
                }
            }
        };
        let mut sim = Running::new(join2(
            join2(
                wire.run((in_rx, ans_rx), (ask_tx, out_tx)),
                master.run(host, (ask_rx, ans_tx)),
            ),
            memory,
        ));
        let mut answers = Vec::new();
        let mut cur: Vec<u8> = Vec::new();
        // One frame at a time, as a program waits for each answer; a
        // frame that is not answered gets 400 cycles.
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
        let n = *writes.borrow();
        let st = strobes.borrow().clone();
        (answers, n, st)
    }

    #[test]
    fn a_program_writes_and_reads_the_bus() {
        let (got, writes) = serve(&[
            ask_frame(5, 1, true, 0x40, 0xabcd),
            ask_frame(5, 2, false, 0x40, 0),
        ]);
        assert_eq!(got, vec![(5, 1, 0, false), (5, 2, 0xabcd, false)]);
        assert_eq!(writes, 1);
    }

    #[test]
    fn a_repeated_ask_is_answered_again_and_not_performed_again() {
        // The program did not hear the first answer and asks again
        // with the same tag; the write must not happen twice.
        let w = ask_frame(5, 7, true, 0x80, 0x1234);
        let (got, writes) =
            serve(&[w.clone(), w, ask_frame(5, 8, false, 0x80, 0)]);
        assert_eq!(
            got,
            vec![(5, 7, 0, false), (5, 7, 0, false), (5, 8, 0x1234, false)]
        );
        assert_eq!(writes, 1, "the repeat was answered, not performed");
    }

    #[test]
    fn an_error_on_the_bus_comes_back_as_an_error() {
        let (got, _) = serve(&[
            ask_frame(5, 3, true, 0x2000, 1),
            ask_frame(5, 4, false, 0x2000, 0),
        ]);
        assert_eq!(got, vec![(5, 3, 0, true), (5, 4, 0, true)]);
    }

    #[test]
    fn frames_that_are_not_this_devices_asks_are_ignored() {
        // Another device's ask, another protocol's type, and the
        // other direction's ask, which is what a looped wire shows.
        let other = ask_frame(6, 1, true, 0x40, 1);
        let mut wrong_type = ask_frame(5, 1, true, 0x40, 2);
        wrong_type[13] = 0xb6;
        let mut theirs = ask_frame(5, 1, true, 0x40, 3);
        theirs[14] = KIND_ASK as u8;
        let good = ask_frame(5, 9, false, 0x40, 0);
        let (got, writes) = serve(&[other, wrong_type, theirs, good]);
        assert_eq!(got, vec![(5, 9, 0, false)], "only its own is answered");
        assert_eq!(writes, 0);
    }

    #[test]
    fn a_partial_write_keeps_its_strobe() {
        let mut f = ask_frame(5, 1, true, 0x40, 0xabcd);
        f[26] = 0x3;
        let (got, writes, strobes) = serve_with(&[f], 0);
        assert_eq!(got, vec![(5, 1, 0, false)]);
        assert_eq!((writes, strobes), (1, vec![0x3]), "the strobe reaches w");
    }

    #[test]
    fn tag_zero_first_is_performed_not_taken_for_a_repeat() {
        // Before the first transaction nothing is a repeat, though the
        // kept tag starts at zero.
        let (got, writes) = serve(&[ask_frame(5, 0, true, 0x40, 9)]);
        assert_eq!(got, vec![(5, 0, 0, false)]);
        assert_eq!(writes, 1);
    }

    #[test]
    fn a_long_ask_is_taken_whole() {
        // An ask with trailing bytes past 64, where a wrapping count
        // would read the header and the fields again.
        let mut f = ask_frame(5, 1, true, 0x40, 0x5555);
        f.resize(100, 0xaa);
        let (got, writes) = serve(&[f, ask_frame(5, 2, false, 0x40, 0)]);
        assert_eq!(got, vec![(5, 1, 0, false), (5, 2, 0x5555, false)]);
        assert_eq!(writes, 1);
    }

    #[test]
    fn a_frame_that_ends_at_the_strobe_is_not_an_ask() {
        let mut f = ask_frame(5, 1, true, 0x40, 1);
        f.truncate(FRAME_LEN as usize);
        let (got, writes) = serve(&[f, ask_frame(5, 2, false, 0x40, 0)]);
        assert_eq!(got, vec![(5, 2, 0, false)], "only the whole one");
        assert_eq!(writes, 0);
    }

    #[test]
    fn a_long_frame_of_another_kind_is_ignored() {
        // Two hundred bytes of another protocol, laid out so that bytes
        // 76 to 79 spell an ask for this device. It is turned away at
        // byte 12, and nothing later in it can make it one.
        let mut other = vec![0x11u8; 200];
        other[12] = 0x08;
        other[13] = 0x06;
        other[76..80].copy_from_slice(&[0x88, 0xb5, KIND_HOST_ASK as u8, 5]);
        let (got, writes) = serve(&[other, ask_frame(5, 3, false, 0x40, 0)]);
        assert_eq!(got, vec![(5, 3, 0, false)]);
        assert_eq!(writes, 0);
    }

    #[test]
    fn a_slow_bus_is_waited_for() {
        let (got, writes, _) = serve_with(
            &[
                ask_frame(5, 1, true, 0x40, 0x77),
                ask_frame(5, 2, false, 0x40, 0),
            ],
            50,
        );
        assert_eq!(got, vec![(5, 1, 0, false), (5, 2, 0x77, false)]);
        assert_eq!(writes, 1);
    }

    #[test]
    fn the_bridge_is_the_two_units_joined() {
        // The same asks through HostBridge as through the two units.
        let (host, per) = link::<LitePort![ADDR = 32, DATA = 32, STRB = 4]>();
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
        let mut bridge = HostBridge::<5>::default();
        let memory = async move {
            let mut word = 0u32;
            loop {
                DefaultClock::rising().await;
                if per.aw.peek().is_some() && per.w.peek().is_some() {
                    let _ = per.aw.recv();
                    word = per.w.recv().unwrap().data.raw() as u32;
                    per.b.send(LiteB { resp: Resp::Okay });
                }
                if per.ar.recv().is_some() {
                    per.r.send(LiteR {
                        data: U::from(word),
                        resp: Resp::Okay,
                    });
                }
            }
        };
        let mut sim =
            Running::new(join2(bridge.run(in_rx, (out_tx, host)), memory));
        let mut answers = Vec::new();
        let mut cur: Vec<u8> = Vec::new();
        for f in [
            ask_frame(5, 1, true, 0x40, 0x99),
            ask_frame(5, 2, false, 0x40, 0),
        ] {
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
        assert_eq!(answers, vec![(5, 1, 0, false), (5, 2, 0x99, false)]);
    }
}
