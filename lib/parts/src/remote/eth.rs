// SPDX-License-Identifier: Apache-2.0
//! The wire under [`Remote`](super::Remote): its transactions as
//! Ethernet frames, and the answers back.
//!
//! [`RemoteLink`] sits between the peripheral's two channels and the
//! MAC's. An [`Ask`](super::Ask) becomes one frame; a frame that
//! answers it becomes an [`Answer`](super::Answer). Nothing else on
//! the wire is looked at, and nothing else has to be: a frame is for
//! this peripheral when it carries this protocol's type, the answer
//! kind, and this peripheral's device number.
//!
//! That device number is what lets several of these share a wire, and
//! it is a parameter rather than a register because a design knows
//! how many it has. A program on the other side answers each by the
//! number in the frame it was asked under.
//!
//! The frame, 27 bytes before the padding the MAC adds:
//!
//! | Bytes | What |
//! |---|---|
//! | 0 to 6 | destination, broadcast |
//! | 6 to 12 | source, `02:00:00:00:00:<device>`, locally administered |
//! | 12 to 14 | the type, `0x88b5`, which IEEE leaves for local use |
//! | 14 | the kind: 1 asks, 0x81 answers |
//! | 15 | the device |
//! | 16 | the tag, which the answer repeats |
//! | 17 | the flags: bit 0 is a write on an ask, an error on an answer |
//! | 18 to 22 | the address, most significant byte first |
//! | 22 to 26 | the word |
//! | 26 | the strobe |
//!
//! An answer is read at the same offsets, so a program can answer by
//! changing four bytes of what it was sent.
//!
//! Broadcast is what the ask is addressed to, because the peripheral
//! does not know where the program is and the program may move. A
//! design that wants the frame not to reach every port on a switch
//! gives the endpoint's address instead, which is a change to six
//! constants and nothing else.
use txhdl::comp::{Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use super::{Answer, Ask};
use crate::eth::EthByte;

/// The type this protocol uses, which IEEE leaves for local
/// experimental use: no registered protocol can be mistaken for it.
pub const ETHERTYPE: u32 = 0x88b5;
/// The kind byte of a frame that asks.
pub const KIND_ASK: u32 = 0x01;
/// The kind byte of a frame that answers.
pub const KIND_ANSWER: u32 = 0x81;
/// How many bytes a frame carries before the MAC's padding.
pub const FRAME_LEN: u32 = 27;

// begin{ethstate}
/// The wire under a [`Remote`](super::Remote): frames out, frames in.
///
/// `DEV` is the device number, which goes in every frame this sends
/// and must match every frame it accepts, so that several peripherals
/// can share one wire and one program.
#[derive(Trace, Default)]
pub struct RemoteLink<const DEV: usize> {
    /// Whether a frame is going out, and how far through it is.
    pub sending: Reg<Bit>,
    /// Which byte of the frame goes next.
    pub at: Reg<U<6>>,
    /// The transaction being sent, held while it is sent.
    pub tag: Reg<U<8>>,
    /// Whether it is a write.
    pub write: Reg<Bit>,
    /// Its address.
    pub addr: Reg<U<32>>,
    /// Its word.
    pub data: Reg<U<32>>,
    /// Its strobe.
    pub strb: Reg<U<4>>,
    /// Which byte of the frame coming in is next.
    pub got: Reg<U<6>>,
    /// Whether a byte of the frame coming in has said it is not this
    /// peripheral's: the wrong type, the wrong kind, another
    /// device. It is held the polarity round because a register
    /// starts at zero, and a frame is this peripheral's until a byte
    /// of it says otherwise.
    pub alien: Reg<Bit>,
    /// The tag the answer carries.
    pub ans_tag: Reg<U<8>>,
    /// Whether it says the transaction failed.
    pub ans_err: Reg<Bit>,
    /// The word it brings.
    pub ans_word: Reg<U<32>>,
}
// end{ethstate}

/// The byte at `at` of the frame that carries an ask.
#[lower]
fn ask_byte(
    at: U<6>,
    dev: U<8>,
    tag: U<8>,
    write: Bit,
    addr: U<32>,
    data: U<32>,
    strb: U<4>,
) -> U<8> {
    let ff = U::<8>::from(0xffu8);
    let zero = U::<8>::from(0u8);
    select!(at.raw() => {
        // The destination: every port, since the peripheral does not
        // know where the program is.
        0..=5 => ff,
        // The source: locally administered, with the device number in
        // the last byte, so two peripherals differ on the wire.
        6 => U::<8>::from(0x02u8),
        7..=10 => zero,
        11 => dev,
        12 => U::<8>::from(0x88u8),
        13 => U::<8>::from(0xb5u8),
        14 => U::<8>::from(KIND_ASK),
        15 => dev,
        16 => tag,
        17 => write.zext::<8>(),
        18 => addr.slice::<24, 8>(),
        19 => addr.slice::<16, 8>(),
        20 => addr.slice::<8, 8>(),
        21 => addr.slice::<0, 8>(),
        22 => data.slice::<24, 8>(),
        23 => data.slice::<16, 8>(),
        24 => data.slice::<8, 8>(),
        25 => data.slice::<0, 8>(),
        26 => strb.zext::<8>(),
        _ => zero,
    })
}

// A struct literal's field is `name: value` to the lowering, which
// reads no shorthand (issue 235), so `tag: tag` stays as written.
#[allow(clippy::redundant_field_names)]
// begin{ethrun}
#[lower]
impl<const DEV: usize> Unit for RemoteLink<DEV> {
    async fn run(
        &mut self,
        (ask, rx): (Rx<Ask>, Rx<EthByte>),
        (back, tx): (Tx<Answer>, Tx<EthByte>),
    ) {
        loop {
            DefaultClock::rising().await;
            let dev = U::<8>::from(DEV as u32);
            // Out. A transaction is taken when nothing is going out,
            // and then one byte leaves per cycle the wire has room.
            let sending = self.sending.get();
            let at = self.at.get();
            let head = ask.head();
            let take = !sending & ask.peek().is_some();
            let _ = ask.recv_if(take);
            let byte = ask_byte(
                at,
                dev,
                self.tag.get(),
                self.write.get(),
                self.addr.get(),
                self.data.get(),
                self.strb.get(),
            );
            let put = sending & tx.ready();
            let last = at == FRAME_LEN - 1;
            // In. Every byte is taken, since a frame that is not for
            // this peripheral still has to be got out of the way, and
            // what makes one this peripheral's is checked as it goes.
            let got = self.got.get();
            let alien = self.alien.get();
            let inb = rx.head();
            let took = rx.peek().is_some();
            let _ = rx.recv_if(took);
            let value = inb.data;
            // The three bytes that say a frame is an answer for this
            // device. Anything else at those places makes the frame
            // not this peripheral's, and nothing in it is read.
            let bad = (got == 12) & (value != U::<8>::from(0x88u8))
                | (got == 13) & (value != U::<8>::from(0xb5u8))
                | (got == 14) & (value != U::<8>::from(KIND_ANSWER))
                | (got == 15) & (value != dev);
            let ends = took & inb.last;
            let answering = ends & !alien & !bad;
            with!(self <= {
                take ? { sending: Bit::One, at: 0, tag: head.tag,
                         write: head.write, addr: head.addr,
                         data: head.data, strb: head.strb },
                put & !last ? at: at + 1,
                put & last ? { sending: Bit::Zero, at: 0 },
                // A frame's bytes are counted from zero, and the
                // count starts again on the byte after the last.
                took & !inb.last ? got: got + 1,
                took & inb.last ? got: 0,
                took & bad ? alien: Bit::One,
                took & inb.last ? alien: Bit::Zero,
                took & (got == 16) ? ans_tag: value,
                took & (got == 17) ? ans_err: value.bit(0),
                took & (got == 22) ? ans_word:
                    value.concat::<_, 32>(self.ans_word.get().slice::<0, 24>()),
                took & (got == 23) ? ans_word:
                    self.ans_word.get().slice::<24, 8>()
                        .concat::<_, 16>(value)
                        .concat::<_, 32>(self.ans_word.get().slice::<0, 16>()),
                took & (got == 24) ? ans_word:
                    self.ans_word.get().slice::<16, 16>()
                        .concat::<_, 24>(value)
                        .concat::<_, 32>(self.ans_word.get().slice::<0, 8>()),
                took & (got == 25) ? ans_word:
                    self.ans_word.get().slice::<8, 24>().concat::<_, 32>(value),
            });
            if put.to_bool() {
                tx.send(EthByte {
                    data: byte,
                    last: Bit::from(last),
                });
            }
            if answering.to_bool() {
                back.send(Answer {
                    tag: self.ans_tag.get(),
                    data: self.ans_word.get(),
                    err: self.ans_err.get(),
                });
            }
        }
    }
}
// end{ethrun}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi::Resp;
    use crate::bus::axi_lite::{
        axi_lite, LiteAr, LiteAw, LiteHost, LitePort, LiteW,
    };
    use crate::remote::Remote;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use txhdl::comp::{chan, join2, Running};
    use txhdl::types::U;

    /// The bytes of one frame, collected as the link emits them.
    fn frame_of(ask: Ask) -> Vec<u8> {
        let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
        let (byte_tx, byte_rx) = chan::<EthByte, DefaultClock>();
        let (ans_tx, _ans_rx) = chan::<Answer, DefaultClock>();
        let (_in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let mut link = RemoteLink::<3>::default();
        let mut sim =
            Running::new(link.run((ask_rx, in_rx), (ans_tx, byte_tx)));
        let mut sent = false;
        let mut out: Vec<u8> = Vec::new();
        for _ in 0..80 {
            if !sent && ask_tx.ready().to_bool() {
                ask_tx.send(ask);
                sent = true;
            }
            if let Some(b) = byte_rx.recv() {
                out.push(b.data.raw() as u8);
                if b.last.to_bool() {
                    break;
                }
            }
            sim.cycle();
        }
        out
    }

    #[test]
    fn a_transaction_leaves_as_a_frame() {
        let bytes = frame_of(Ask {
            tag: U::from(0x5au8),
            write: Bit::One,
            addr: U::from(0x1234_5678u32),
            data: U::from(0xc0ff_ee11u32),
            strb: U::from(0xfu8),
        });
        assert_eq!(bytes.len(), FRAME_LEN as usize, "{bytes:02x?}");
        assert_eq!(&bytes[0..6], &[0xff; 6], "to every port");
        assert_eq!(&bytes[6..12], &[0x02, 0, 0, 0, 0, 3], "from device 3");
        assert_eq!(&bytes[12..14], &[0x88, 0xb5], "the type");
        assert_eq!(bytes[14], KIND_ASK as u8, "it asks");
        assert_eq!(bytes[15], 3, "the device");
        assert_eq!(bytes[16], 0x5a, "the tag");
        assert_eq!(bytes[17], 1, "a write");
        assert_eq!(&bytes[18..22], &[0x12, 0x34, 0x56, 0x78], "the address");
        assert_eq!(&bytes[22..26], &[0xc0, 0xff, 0xee, 0x11], "the word");
        assert_eq!(bytes[26], 0xf, "the strobe");
    }

    #[test]
    fn a_read_leaves_with_no_word_and_no_strobe() {
        let bytes = frame_of(Ask {
            tag: U::from(7u8),
            write: Bit::Zero,
            addr: U::from(0x2000u32),
            ..Ask::default()
        });
        assert_eq!(bytes[14], KIND_ASK as u8);
        assert_eq!(bytes[17], 0, "not a write");
        assert_eq!(&bytes[22..26], &[0, 0, 0, 0], "no word");
        assert_eq!(bytes[26], 0, "no strobe");
    }

    /// An answer frame, as a program would build it: what it was sent,
    /// with the kind, the flags and the word changed.
    fn answer_frame(dev: u8, tag: u8, data: u32, err: bool) -> Vec<u8> {
        let mut f = vec![0u8; FRAME_LEN as usize];
        f[0..6].copy_from_slice(&[0xff; 6]);
        f[12] = 0x88;
        f[13] = 0xb5;
        f[14] = KIND_ANSWER as u8;
        f[15] = dev;
        f[16] = tag;
        f[17] = err as u8;
        f[22..26].copy_from_slice(&data.to_be_bytes());
        f
    }

    /// Feeds `frames` into the link's receive side and returns every
    /// answer it produced.
    fn answers_from(frames: &[Vec<u8>]) -> Vec<(u8, u32, bool)> {
        let (_ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
        let (byte_tx, _byte_rx) = chan::<EthByte, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<Answer, DefaultClock>();
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let mut link = RemoteLink::<3>::default();
        let mut sim =
            Running::new(link.run((ask_rx, in_rx), (ans_tx, byte_tx)));
        let mut queue: Vec<EthByte> = Vec::new();
        for f in frames {
            for (i, b) in f.iter().enumerate() {
                queue.push(EthByte {
                    data: U::from(*b),
                    last: Bit::from(i + 1 == f.len()),
                });
            }
        }
        let mut out = Vec::new();
        for _ in 0..400 {
            if !queue.is_empty() && in_tx.ready().to_bool() {
                in_tx.send(queue.remove(0));
            }
            if let Some(a) = ans_rx.recv() {
                out.push((
                    a.tag.raw() as u8,
                    a.data.raw() as u32,
                    a.err.to_bool(),
                ));
            }
            sim.cycle();
        }
        out
    }

    #[test]
    fn an_answer_frame_becomes_an_answer() {
        let got = answers_from(&[answer_frame(3, 9, 0xdead_beef, false)]);
        assert_eq!(got, vec![(9, 0xdead_beef, false)]);
        let bad = answers_from(&[answer_frame(3, 10, 0, true)]);
        assert_eq!(bad, vec![(10, 0, true)], "the error comes across");
    }

    #[test]
    fn a_frame_that_is_not_this_devices_is_ignored() {
        // Another device's answer, another protocol's type, and an
        // ask rather than an answer, which is what this peripheral's
        // own frame looks like when the wire is looped.
        let other = answer_frame(4, 9, 1, false);
        let mut wrong_type = answer_frame(3, 9, 2, false);
        wrong_type[13] = 0xb6;
        let mut an_ask = answer_frame(3, 9, 3, false);
        an_ask[14] = KIND_ASK as u8;
        let good = answer_frame(3, 11, 0x55, false);
        let got = answers_from(&[other, wrong_type, an_ask, good]);
        assert_eq!(got, vec![(11, 0x55, false)], "only the one for it");
    }

    /// The whole thing without a wire: a bus, the peripheral, the
    /// link, and a program that answers frames with frames.
    #[test]
    fn the_bus_is_served_by_a_program_that_speaks_frames() {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<Answer, DefaultClock>();
        let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let mut remote = Remote::<2000>::default();
        let mut wire = RemoteLink::<3>::default();
        let host: LiteHost<32, 32, 4> = link.host;
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let client = async move {
            let (aw, ar, w, b, r) = &host;
            // A write, then a read of what it wrote.
            aw.send(LiteAw {
                addr: U::from(0x40u32),
                prot: U::from(0u8),
            });
            w.send(LiteW {
                data: U::from(0xabcdu32),
                strb: U::from(0xfu8),
            });
            let resp = loop {
                DefaultClock::rising().await;
                if let Some(v) = b.recv() {
                    break v.resp;
                }
            };
            assert_eq!(resp, Resp::Okay, "the program took the write");
            ar.send(LiteAr {
                addr: U::from(0x40u32),
                prot: U::from(0u8),
            });
            let got = loop {
                DefaultClock::rising().await;
                if let Some(v) = r.recv() {
                    break v;
                }
            };
            assert_eq!(got.resp, Resp::Okay);
            assert_eq!(got.data.raw(), 0xabcd, "what the program kept");
            *d.borrow_mut() = true;
        };
        let mut sim = Running::new(join2(
            join2(
                remote.run(bus, (ans_rx, ask_tx)),
                wire.run((ask_rx, in_rx), (ans_tx, out_tx)),
            ),
            client,
        ));
        // The program: it reads whole frames off the link's transmit
        // side and writes answer frames back, holding the words it is
        // given, which is all a device has to do.
        let mut words: HashMap<u32, u32> = HashMap::new();
        let mut frame: Vec<u8> = Vec::new();
        let mut reply: Vec<EthByte> = Vec::new();
        for _ in 0..3000 {
            if !reply.is_empty() && in_tx.ready().to_bool() {
                in_tx.send(reply.remove(0));
            }
            if let Some(byte) = out_rx.recv() {
                frame.push(byte.data.raw() as u8);
                if byte.last.to_bool() {
                    let tag = frame[16];
                    let addr =
                        u32::from_be_bytes(frame[18..22].try_into().unwrap());
                    let data =
                        u32::from_be_bytes(frame[22..26].try_into().unwrap());
                    let (word, err) = if frame[17] & 1 == 1 {
                        words.insert(addr, data);
                        (0, false)
                    } else {
                        match words.get(&addr) {
                            Some(v) => (*v, false),
                            None => (0, true),
                        }
                    };
                    let f = answer_frame(3, tag, word, err);
                    let n = f.len();
                    for (i, byte) in f.into_iter().enumerate() {
                        reply.push(EthByte {
                            data: U::from(byte),
                            last: Bit::from(i + 1 == n),
                        });
                    }
                    frame.clear();
                }
            }
            sim.cycle();
            if *done.borrow() {
                return;
            }
        }
        panic!("the bus was never served");
    }
}
