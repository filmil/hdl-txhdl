// SPDX-License-Identifier: Apache-2.0
//! Ethernet: a MAC in two halves and an AXI-Lite peripheral in front
//! of them.
//!
//! [`EthTx`] and [`EthRx`] speak GMII, a byte per cycle with a valid
//! line. A board wrapper makes GMII of an RGMII PHY such as the
//! JL2121 on the AX7A200B, whose nibbles come on both clock edges.
//! Each half is a unit of one clock, so that on a board each can run
//! on the clock its side of the PHY keeps: the transmit clock the FPGA
//! makes, and the receive clock the PHY recovers from the line.
//!
//! Both halves store a whole frame before they act on it. The
//! transmitter takes a frame's bytes from a channel at whatever pace
//! they come and sends the frame only when its last byte is in, since
//! a frame on the wire cannot pause. The receiver takes a frame off
//! the wire as it comes, checks its frame check sequence, and only
//! then offers its bytes on a channel, since the wire cannot be held
//! off. So the channels between the MAC and [`EthLite`] may be slow,
//! and on a board they are where the clock domains are crossed.
//!
//! The frame a client sends is the destination address, the source
//! address, the type or length and the payload. The transmitter adds
//! the preamble, the start of frame delimiter, the padding up to sixty
//! bytes and the four bytes of the frame check sequence, a CRC-32; the
//! receiver strips the preamble and the check sequence and drops a
//! frame whose check fails.
use txhdl::comp::{
    join2, mux, until, Clock, DefaultClock, In, Mem, Out, Reg, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, regmap, with, Trace};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

// begin{byte}
/// A byte of a frame, on the channels between the MAC and its client:
/// the byte, and whether it is the frame's last.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct EthByte {
    /// The byte.
    pub data: U<8>,
    /// The frame's last byte.
    pub last: Bit,
}
// end{byte}

/// The CRC-32 register after the reset: every bit set.
pub const CRC_INIT: u32 = 0xffff_ffff;
/// The polynomial, reflected, as a register that shifts right takes
/// it: the CRC-32 of Ethernet, of ZIP and of PNG.
pub const CRC_POLY: u32 = 0xedb8_8320;
/// The register after a whole frame and its own check sequence have
/// gone through it, when nothing was corrupted on the way.
pub const CRC_RESIDUE: u32 = 0xdebb_20e3;
/// The same register complemented. The receiver compared this while
/// the lowering wrote a constant of 2 to the 31 or more as an unsized
/// integer, which is issue 130; it compares the residue itself now.
pub const CRC_GOOD: u32 = !CRC_RESIDUE;
/// The largest frame a half stores, check sequence included.
pub const FRAME_MAX: usize = 2048;

// begin{crc}
/// One step of the CRC-32 register: it shifts right, and the
/// polynomial goes in whenever the bit that leaves is set.
#[lower]
fn crc_step(crc: U<32>) -> U<32> {
    let shifted = crc >> 1;
    mux(crc.bit(0), shifted ^ U::<32>::from(CRC_POLY), shifted)
}

/// One byte into the register, as Ethernet sends its bits, least
/// significant first: the byte goes in by exclusive or and the
/// register takes eight steps.
///
/// This was written as the 32 sums the eight steps work out to, and is
/// written as the eight steps again. The lowering used to paste an
/// argument's text wherever the body read it, and `crc_step` reads its
/// argument twice, so the eighth step held the register's expression
/// 256 times and one unit took minutes to compile. The lowering now
/// gives such a value a wire of the netlist and reads it by name,
/// which is issue 126.
#[lower]
fn crc_byte(crc: U<32>, d: U<8>) -> U<32> {
    let c0 = crc_step(crc ^ d.zext::<32>());
    let c1 = crc_step(c0);
    let c2 = crc_step(c1);
    let c3 = crc_step(c2);
    let c4 = crc_step(c3);
    let c5 = crc_step(c4);
    let c6 = crc_step(c5);
    crc_step(c6)
}
// end{crc}

/// The CRC-32 of `bytes`, as it goes into the check sequence: the same
/// register as the hardware's, complemented at the end. For the tests
/// and for a client that builds frames in Rust.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut c = CRC_INIT;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 {
                (c >> 1) ^ 0xedb8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// What the transmitter puts on the wire for a frame: the preamble,
/// the delimiter, the frame padded to sixty bytes, and its check
/// sequence, least significant byte first. The model the tests hold
/// the hardware to.
pub fn wire_bytes(frame: &[u8]) -> Vec<u8> {
    let mut body = frame.to_vec();
    body.resize(body.len().max(60), 0);
    let fcs = crc32(&body);
    let mut out = vec![0x55; 7];
    out.push(0xd5);
    out.extend_from_slice(&body);
    out.extend_from_slice(&fcs.to_le_bytes());
    out
}

/// The transmit half of the MAC. It stores a frame's bytes from `tx`
/// until the last, then sends the frame on `txd` with `tx_en` high:
/// seven bytes of preamble and the delimiter, the frame, zeros up to
/// sixty bytes, and the check sequence; then twelve bytes of gap with
/// `tx_en` low before the next frame may begin. A frame longer than
/// [`FRAME_MAX`] bytes keeps overwriting its last byte.
// begin{txstate}
#[derive(Trace, Default)]
pub struct EthTx {
    /// The frame being stored, or being sent.
    pub frame: Mem<U<8>, FRAME_MAX>,
    /// How many of its bytes are stored.
    pub fill: Reg<U<11>>,
    /// Its last byte is stored, and it is being sent.
    pub whole: Reg<Bit>,
    /// The CRC-32 register over the frame and its padding.
    pub crc: Reg<U<32>>,
    /// Up for the one cycle after a frame's gap, which is when the
    /// store lets the next frame in.
    pub sent: Reg<Bit>,
    /// Frames sent.
    pub frames: Reg<U<16>>,
}
// end{txstate}

// begin{tx}
#[lower]
impl Unit for EthTx {
    /// Two processes. The store takes a byte a cycle from `tx` while
    /// no frame is whole, and lets go of the frame when the sender
    /// says it is sent. The sender is the wire's protocol written as
    /// the sequence it is: wait for a whole frame, the preamble and
    /// the delimiter, the frame's bytes, zeros up to sixty, the check
    /// sequence, the gap, one byte a cycle, each a turn of a loop.
    async fn run(
        &mut self,
        tx: Rx<EthByte>,
        (txd, tx_en): (Out<U<8>>, Out<Bit>),
    ) {
        join2(
            async {
                loop {
                    DefaultClock::rising().await;
                    let fill = self.fill.get();
                    let storing = !self.whole.get();
                    let offered = tx.head();
                    let take = storing & tx.peek().is_some();
                    let _ = tx.recv_if(storing);
                    let last_slot = fill == 2047;
                    with!(self <= {
                        take & !last_slot ? fill: fill + 1,
                        take & offered.last ? whole: Bit::One,
                        self.sent.get() ? {
                            fill: U::<11>::from(0u8),
                            whole: Bit::Zero,
                        },
                    });
                    if take.to_bool() {
                        self.frame.at(fill).set(offered.data);
                    }
                }
            },
            async {
                loop {
                    // A whole frame.
                    until(DefaultClock::rising, || self.whole.get().to_bool())
                        .await;
                    // Seven bytes of preamble and the delimiter, with
                    // the check sequence register put at its start.
                    for i in 0..8 {
                        txd.set(mux(
                            Bit::from(i == 7),
                            U::<8>::from(0xd5u8),
                            U::<8>::from(0x55u8),
                        ));
                        tx_en.set(Bit::One);
                        self.crc.set(U::<32>::from(CRC_INIT));
                        DefaultClock::rising().await;
                    }
                    // The frame, its check sequence folded a byte at a
                    // time.
                    for i in 0..self.fill.get().raw() as usize {
                        let byte = self.frame.read(i);
                        txd.set(byte);
                        self.crc.set(crc_byte(self.crc.get(), byte));
                        DefaultClock::rising().await;
                    }
                    // Zeros up to the sixty bytes a frame must have.
                    if self.fill.get() < 60 {
                        for _ in self.fill.get().raw() as usize..60 {
                            txd.set(U::<8>::from(0u8));
                            self.crc.set(crc_byte(
                                self.crc.get(),
                                U::<8>::from(0u8),
                            ));
                            DefaultClock::rising().await;
                        }
                    }
                    // The check sequence, least significant byte
                    // first.
                    txd.set((!self.crc.get()).slice::<0, 8>());
                    DefaultClock::rising().await;
                    txd.set((!self.crc.get()).slice::<8, 8>());
                    DefaultClock::rising().await;
                    txd.set((!self.crc.get()).slice::<16, 8>());
                    DefaultClock::rising().await;
                    txd.set((!self.crc.get()).slice::<24, 8>());
                    DefaultClock::rising().await;
                    // Twelve bytes of gap, and the frame is sent.
                    for _ in 0..12 {
                        txd.set(U::<8>::from(0u8));
                        tx_en.set(Bit::Zero);
                        DefaultClock::rising().await;
                    }
                    with!(self <= {
                        sent: Bit::One,
                        frames: self.frames.get() + 1,
                    });
                    DefaultClock::rising().await;
                    self.sent.set(Bit::Zero);
                }
            },
        )
        .await;
    }
}
// end{tx}

/// The receive half of the MAC. It waits for `rx_dv` with the start of
/// frame delimiter on `rxd`, stores the frame's bytes while `rx_dv`
/// stays high, and at its end checks the frame check sequence. A frame
/// that passes, and is longer than its check sequence, is offered on
/// `rx` without the check sequence, its last byte marked; one that
/// fails, or that had `rx_er` high, is dropped and counted. A frame
/// that arrives while the previous one is still being offered is
/// dropped too, since the wire cannot wait; so is the rest of a frame
/// whose start was missed.
// begin{rxstate}
#[derive(Trace, Default)]
pub struct EthRx {
    /// The frame being received, or being offered.
    pub frame: Mem<U<8>, FRAME_MAX>,
    /// Bytes received, check sequence included.
    pub len: Reg<U<11>>,
    /// A frame is being received: the delimiter has passed and
    /// `rx_dv` has not yet fallen.
    pub receiving: Reg<Bit>,
    /// A frame is stored and good, and is being offered.
    pub full: Reg<Bit>,
    /// The CRC-32 register over the frame and its check sequence.
    pub crc: Reg<U<32>>,
    /// `rx_er` was high during this frame.
    pub bad: Reg<Bit>,
    /// `rx_dv` is high on a frame this half is not receiving, which is
    /// ignored until the line goes idle.
    pub skip: Reg<Bit>,
    /// Up for the one cycle after the last byte is taken, which is
    /// when the receiver lets the next frame in.
    pub given: Reg<Bit>,
    /// Frames offered.
    pub frames: Reg<U<16>>,
    /// Frames dropped: a failed check, an error, or no room.
    pub dropped: Reg<U<16>>,
}
// end{rxstate}

// begin{rx}
#[lower]
impl Unit<(In<U<8>>, In<Bit>, In<Bit>), (Tx<EthByte>, Out<U<16>>)> for EthRx {
    /// Two processes. The receiver watches the wire every cycle: it
    /// hunts for the delimiter, stores a byte a cycle while `rx_dv`
    /// is high, checks the frame at its end, and marks it full when
    /// it passes. The offerer is the sequence: wait for a full frame,
    /// then its bytes without the check sequence, one per turn, each
    /// held until the consumer takes it, and then the frame is given
    /// back.
    async fn run(
        &mut self,
        (rxd, rx_dv, rx_er): (In<U<8>>, In<Bit>, In<Bit>),
        (rx, rx_len): (Tx<EthByte>, Out<U<16>>),
    ) {
        join2(
            async {
                loop {
                    DefaultClock::rising().await;
                    let len = self.len.get();
                    let crc = self.crc.get();
                    let d = rxd.get();
                    let dv = rx_dv.get();
                    let er = rx_er.get();
                    let receiving = self.receiving.get();
                    let full = self.full.get();
                    // Hunting: the delimiter with `rx_dv` high starts a
                    // frame; the preamble before it is passed over, and
                    // anything else on a busy line is the middle of a
                    // frame whose start was missed.
                    let hunting = !receiving & !full & !self.skip.get();
                    let sfd = hunting & dv & Bit::from(d == 0xd5);
                    let stray = hunting
                        & dv
                        & Bit::from(d != 0xd5)
                        & Bit::from(d != 0x55);
                    // Receiving: a byte a cycle while `rx_dv` is high.
                    let store = receiving & dv;
                    let dv_end = receiving & !dv;
                    let good = !self.bad.get()
                        & Bit::from(crc == CRC_RESIDUE)
                        & Bit::from(len > 4);
                    // A frame on the line while this half cannot take it.
                    let busy_line = dv & full;
                    let last_slot = len == 2047;
                    with!(self <= {
                        stray | busy_line ? skip: Bit::One,
                        !dv ? skip: Bit::Zero,
                        sfd ? {
                            receiving: Bit::One,
                            len: U::<11>::from(0u8),
                            crc: U::<32>::from(CRC_INIT),
                            bad: Bit::Zero,
                        },
                        store & !last_slot ? len: len + 1,
                        store ? {
                            crc: crc_byte(crc, d),
                            bad: self.bad.get() | er,
                        },
                        dv_end ? receiving: Bit::Zero,
                        dv_end & good ? full: Bit::One,
                        dv_end & !good ? dropped: self.dropped.get() + 1,
                        self.given.get() ? full: Bit::Zero,
                        busy_line & !self.skip.get() ?
                            dropped: self.dropped.get() + 1,
                    });
                    if store.to_bool() {
                        self.frame.at(len).set(d);
                    }
                }
            },
            async {
                loop {
                    // A full frame.
                    until(DefaultClock::rising, || self.full.get().to_bool())
                        .await;
                    // The frame's length, for anything that must know
                    // it before it has consumed the frame: it reads
                    // the payload while the frame is offered and zero
                    // at every other time.
                    rx_len.set((self.len.get() - 4).resize::<16>());
                    DefaultClock::rising().await;
                    // The bytes without the check sequence, each held
                    // until the consumer takes it.
                    for i in 0..(self.len.get() - 4).raw() as usize {
                        rx.send(EthByte {
                            data: self.frame.read(i),
                            last: Bit::from(
                                i + 1 == (self.len.get() - 4).raw() as usize,
                            ),
                        });
                        until(DefaultClock::rising, || rx.ready().to_bool())
                            .await;
                    }
                    rx_len.set(U::<16>::from(0u8));
                    with!(self <= {
                        given: Bit::One,
                        frames: self.frames.get() + 1,
                    });
                    DefaultClock::rising().await;
                    self.given.set(Bit::Zero);
                }
            },
        )
        .await;
    }
}
// end{rx}

// The Ethernet peripheral's registers.
// begin{lite}
regmap! { regs (regs_read, regs_we, regs_re), 2: [
    (0, status, ro, "what the two halves can do", [
        (rx, 0, 1, ro, 0, "a received byte is waiting"),
        (tx, 1, 1, ro, 0, "the transmitter has room for a byte"),
    ]),
    (1, txbyte, wo, "a byte to send; answered when it is taken", [
        (data, 0, 8, wo, 0, "the byte"),
        (last, 8, 1, wo, 0, "the frame's last byte"),
    ]),
    (2, rxbyte, rc, "the oldest received byte; a read takes it", [
        (data, 0, 8, ro, 0, "the byte"),
        (last, 8, 1, ro, 0, "the frame's last byte"),
        (valid, 9, 1, ro, 0, "a byte was there"),
    ]),
] }

/// The Ethernet peripheral, on AXI-Lite: three words, a byte a
/// transaction, as `regs` states them.
///
/// A write to `txbyte` is answered when the transmitter has taken the
/// byte, so a client that writes a frame byte by byte is held off
/// rather than losing bytes. A read of `rxbyte` with `valid` high takes
/// the byte. The interrupt line is high while a received byte is
/// waiting. The bridge in front sends the peripheral only its own
/// range, so it checks no more of the address than the word.
#[derive(Trace, Default)]
pub struct EthLite {}

#[lower]
impl Unit for EthLite {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (rx, tx, irq): (Rx<EthByte>, Tx<EthByte>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let rsel = arh.addr.slice::<2, 2>();
            let wsel = awh.addr.slice::<2, 2>();
            let waiting = Bit::from(rx.peek().is_some());
            let rxh = rx.head();
            // A write to the transmit word waits for the transmitter.
            let to_tx = regs_we(Bit::One, wsel).bit(1);
            let wroom = !to_tx | tx.ready();
            let wgo = bus.b.ready()
                & Bit::from(bus.aw.peek().is_some())
                & Bit::from(bus.w.peek().is_some())
                & wroom;
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let rgo = bus.r.ready() & Bit::from(bus.ar.peek().is_some());
            let _ = bus.ar.recv_if(bus.r.ready());
            // A read of the receive word takes the byte, if one waits.
            let _ = rx.recv_if(regs_re(rgo, rsel).bit(2));
            let status = regs_status_pack(waiting, tx.ready());
            let received = regs_rxbyte_pack(rxh.data, rxh.last, waiting);
            let word = regs_read(rsel, status, U::<32>::from(0u32), received);
            if regs_we(wgo, wsel).bit(1).to_bool() {
                tx.send(EthByte {
                    data: regs_txbyte_data(wh.data),
                    last: regs_txbyte_last(wh.data),
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
            if rgo.to_bool() {
                bus.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            irq.set(waiting);
        }
    }
}
// end{lite}

/// The MAC against the model: a frame sent is on the wire as
/// [`wire_bytes`] says, a frame received comes back without its
/// framing, and a frame whose check fails is dropped.
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{chan, join2, signal, Running};

    /// A frame of `n` bytes: two addresses, a type, and a count.
    fn frame(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 + 3) as u8).collect()
    }

    /// `crc_byte` against the register stepped a bit at a time by
    /// plain Rust that spells the polynomial out, on every byte from
    /// a few registers.
    #[test]
    fn the_byte_wide_crc_is_the_bitwise_one() {
        for start in [0u32, CRC_INIT, 0x1234_5678, 0x8000_0001] {
            for byte in 0..=255u32 {
                let mut c = start ^ byte;
                for _ in 0..8 {
                    c = if c & 1 == 1 {
                        (c >> 1) ^ 0xedb8_8320
                    } else {
                        c >> 1
                    };
                }
                let got = crc_byte(U::from(start), U::from(byte));
                assert_eq!(got.raw() as u32, c, "{start:#x}, {byte:#x}");
            }
        }
    }

    #[test]
    fn the_crc_of_a_known_string() {
        // The check value every CRC-32 catalogue gives.
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        // A frame followed by its own check sequence leaves the
        // residue in the register.
        let body = frame(60);
        let mut all = body.clone();
        all.extend_from_slice(&crc32(&body).to_le_bytes());
        assert_eq!(!crc32(&all), CRC_RESIDUE);
    }

    /// Frames sent through the transmitter, and received back through
    /// the receiver on the same wire, one cycle later; the bytes on
    /// the wire are recorded.
    fn loopback(
        frames: &[Vec<u8>],
        corrupt: Option<usize>,
        hold: usize,
    ) -> (Vec<u8>, Vec<Vec<u8>>, u128) {
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
        let (txd_out, txd) = signal::<U<8>, DefaultClock>();
        let (en_out, en) = signal::<Bit, DefaultClock>();
        let (rxd_out, rxd) = signal::<U<8>, DefaultClock>();
        let (dv_out, dv) = signal::<Bit, DefaultClock>();
        let (er_out, er) = signal::<Bit, DefaultClock>();
        let mut mac_tx = EthTx::default();
        let mut mac_rx = EthRx::default();
        let dropped = mac_rx.dropped;
        // The length the receiver reports is not what this test
        // checks; it reads the bytes and counts them itself.
        let (rxlen_out, _rxlen) = signal::<U<16>, DefaultClock>();
        let mut sim = Running::new(join2(
            mac_tx.run(in_rx, (txd_out, en_out)),
            mac_rx.run((rxd, dv, er), (out_tx, rxlen_out)),
        ));
        let mut queue: Vec<EthByte> = Vec::new();
        for f in frames {
            for (i, b) in f.iter().enumerate() {
                queue.push(EthByte {
                    data: U::from(*b),
                    last: Bit::from_bool(i + 1 == f.len()),
                });
            }
        }
        let wire = Rc::new(RefCell::new(Vec::new()));
        let mut got: Vec<Vec<u8>> = vec![Vec::new()];
        let mut on_wire = 0usize;
        for t in 0..(frames.len() * 300 + hold) {
            if !queue.is_empty() && in_tx.ready().to_bool() {
                in_tx.send(queue.remove(0));
            }
            let taken = if t >= hold { out_rx.recv() } else { None };
            if let Some(b) = taken {
                got.last_mut().unwrap().push(b.data.raw() as u8);
                if b.last.to_bool() {
                    got.push(Vec::new());
                }
            }
            sim.cycle();
            // The wire, a cycle late, with one byte flipped if asked.
            let mut byte = txd.get().raw() as u8;
            if en.get().to_bool() {
                if Some(on_wire) == corrupt {
                    byte ^= 0x10;
                }
                on_wire += 1;
                wire.borrow_mut().push(txd.get().raw() as u8);
            }
            rxd_out.set(U::from(byte));
            dv_out.set(en.get());
            er_out.set(Bit::Zero);
        }
        got.pop();
        let w = wire.borrow().clone();
        (w, got, dropped.get().raw())
    }

    #[test]
    fn a_frame_goes_out_as_the_model_says_and_comes_back() {
        let frames = vec![frame(20), frame(100)];
        let (wire, got, dropped) = loopback(&frames, None, 0);
        let mut want = wire_bytes(&frames[0]);
        want.extend(wire_bytes(&frames[1]));
        assert_eq!(wire, want, "the bytes on the wire");
        // The short frame comes back padded, the long one as it went.
        let mut padded = frames[0].clone();
        padded.resize(60, 0);
        assert_eq!(got, vec![padded, frames[1].clone()]);
        assert_eq!(dropped, 0);
    }

    /// The padding's edge: a frame one byte short of the minimum, one
    /// at it, and one past it.
    #[test]
    fn frames_at_the_minimum_length() {
        let frames = vec![frame(59), frame(60), frame(61)];
        let (wire, got, dropped) = loopback(&frames, None, 0);
        let want: Vec<u8> = frames.iter().flat_map(|f| wire_bytes(f)).collect();
        assert_eq!(wire, want, "the bytes on the wire");
        let mut padded = frames[0].clone();
        padded.push(0);
        assert_eq!(got, vec![padded, frames[1].clone(), frames[2].clone()]);
        assert_eq!(dropped, 0);
    }

    /// A bit of a field of a channel's value is a bit of the value, at
    /// the field's offset. Verilog has no bit-select of a part-select,
    /// and what was written for one, `w_data[35:4][8]`, did not
    /// simulate in VHDL as the run did (issue 129).
    #[test]
    fn a_bit_of_a_channel_field_is_one_select_in_both_netlists() {
        let v = EthLite::verilog("eth_lite");
        let h = EthLite::vhdl("eth_lite");
        // `bus_w`, since the link is one port and its channels are
        // named for it (issue 483); the select is what is checked.
        assert!(
            v.contains("{bus_w_data[11:4], bus_w_data[12]}"),
            "one select each"
        );
        assert!(!v.contains("]["), "no select of a select");
        assert!(
            h.contains("bus_w_data(11 downto 4) & bus_w_data(12)"),
            "the same in VHDL"
        );
    }

    /// The receiver compares the CRC register with the residue,
    /// `0xdebb_20e3`, which is above 2 to the 31. Verilog reads an
    /// unsized decimal as a 32-bit signed integer and VHDL guarantees
    /// a natural only to 2^31 - 1, so the netlist writes the constant
    /// at the width of what it is compared with: sized hexadecimal in
    /// Verilog, a bit string in VHDL (issue 130).
    #[test]
    fn the_crc_residue_is_a_sized_constant_in_both_netlists() {
        let v = EthRx::verilog("eth_rx");
        let h = EthRx::vhdl("eth_rx");
        assert!(v.contains("32'hdebb20e3"), "sized in Verilog");
        assert!(!v.contains("3736805603"), "no unsized decimal");
        let bits = format!("{:032b}", CRC_RESIDUE);
        assert!(
            h.contains(&format!("unsigned'(\"{bits}\")")),
            "a bit string in VHDL"
        );
        assert!(!h.contains("to_unsigned(3736805603"), "not a natural");
    }

    #[test]
    fn a_corrupted_frame_is_dropped() {
        let frames = vec![frame(64), frame(64), frame(64)];
        // A byte in the middle of the second frame's data.
        let (_, got, dropped) = loopback(&frames, Some(72 + 8 + 30), 0);
        assert_eq!(got, vec![frames[0].clone(), frames[2].clone()]);
        assert_eq!(dropped, 1);
    }

    /// A frame that arrives while the one before it is still being
    /// offered is dropped and counted, and the one offered is intact.
    #[test]
    fn a_frame_on_a_busy_receiver_is_dropped() {
        let frames = vec![frame(64), frame(64)];
        let (_, got, dropped) = loopback(&frames, None, 400);
        assert_eq!(got, vec![frames[0].clone()]);
        assert_eq!(dropped, 1);
    }
}
