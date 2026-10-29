// SPDX-License-Identifier: Apache-2.0
//! The Ethernet host link of FireSim's Rocket design on the AX7A200B
//! (issue 1988): the Verilog of `rocket_link`, on standard output.
//!
//! One wire, two users, split by EtherType in `EthShare`: the control
//! frames (0x88b5) to `HostLink` and `LiteMaster`, which perform them on FPGATop's
//! AXI4-Lite control port, and every other frame to `StreamBridge`,
//! which takes the stream frames (0x88b6) and performs them on the
//! 128-bit `cpu_managed` port. They are joined inside one unit, so
//! the lowering buffers the channels between them; joined by wires in
//! the top, each channel's valid would depend on its own ready.
//!
//! In front of the split, `ResetTap` passes every frame on and watches
//! for a reset frame (issue 2007): kind 0x04 on the control EtherType.
//! FireSim expects a fresh simulator for each run, and the board top
//! uses the tap's `reset` to reset the simulator and this link.
use txhdl::comp::{chan, join2, Clock, DefaultClock, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{Ar, Aw, B, R, W};
use txhdl_parts::bus::axi_lite::LiteHostPort;
use txhdl_parts::eth::EthByte;
use txhdl_parts::ethshare::EthShare;
use txhdl_parts::remote::host::{HostLink, LiteMaster};
use txhdl_parts::remote::stream::StreamBridge;
use txhdl_parts::remote::{Answer, Ask};

/// The design's device number on the wire; the driver's `+device=`.
const DEVICE: usize = 5;

/// The control frames' EtherType, `EthShare`'s first user.
const CONTROL: usize = 0x88b5;

/// The kind byte of a control frame that asks for a reset. The frame
/// carries the device number at byte 15, as an ask does, and gets no
/// answer: the reset takes the link with it.
const KIND_RESET: u32 = 0x04;

/// Passes every frame on, and raises `reset` for a cycle at the end of
/// a reset frame for device `DEV`. It reads the frame as it goes, so it
/// sees a reset frame even while the units behind it are stuck.
#[derive(Trace, Default)]
pub struct ResetTap<const DEV: usize> {
    /// Which byte of the frame going through is next. It stops at 63,
    /// as `HostLink`'s does.
    pub got: Reg<U<6>>,
    /// Whether a byte of the frame has said it is not a reset for this
    /// device.
    pub alien: Reg<Bit>,
    /// Whether a reset frame ended on the cycle before; `reset`'s value.
    pub asked: Reg<Bit>,
}

#[lower]
impl<const DEV: usize> Unit for ResetTap<DEV> {
    async fn run(
        &mut self,
        rx: Rx<EthByte>,
        (tx, reset): (Tx<EthByte>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            // From a register, so the top can take it across to another
            // clock.
            reset.set(self.asked.get());
            let dev = U::<8>::from(DEV as u32);
            let got = self.got.get();
            let inb = rx.head();
            let pass = Bit::from(rx.peek().is_some()) & tx.ready();
            let _ = rx.recv_if(pass);
            let value = inb.data;
            let bad = (got == 12) & (value != U::<8>::from(0x88u8))
                | (got == 13) & (value != U::<8>::from(0xb5u8))
                | (got == 14) & (value != U::<8>::from(KIND_RESET))
                | (got == 15) & (value != dev);
            // A frame that ends before its device byte is not a reset.
            let whole = Bit::from(got >= 15);
            let ends = pass & inb.last;
            let asked = ends & !self.alien.get() & !bad & whole;
            with!(self <= {
                pass & !inb.last & (got != 63) ? got: got + 1,
                ends ? got: 0,
                pass & bad ? alien: Bit::One,
                ends ? alien: Bit::Zero,
                asked: asked,
            });
            if pass.to_bool() {
                tx.send(inb);
            }
        }
    }
}

/// The shared wire and both bridges.
#[derive(Trace, Default)]
pub struct RocketLink<const DEV: usize> {
    /// Watches for the reset frame.
    pub tap: ResetTap<DEV>,
    /// The wire, split by EtherType.
    pub share: EthShare<CONTROL>,
    /// The control frames.
    pub link: HostLink<DEV>,
    /// The control port. HostLink and LiteMaster rather than HostBridge,
    /// since a bundle port is passed to a child only as a bare argument.
    pub master: LiteMaster,
    /// The stream port.
    pub stream: StreamBridge<DEV>,
}

#[lower]
impl<const DEV: usize> Unit for RocketLink<DEV> {
    async fn run(
        &mut self,
        (rx, b, r): (Rx<EthByte>, Rx<B<4>>, Rx<R<128, 4>>),
        (tx, bus, aw, ar, w, reset): (
            Tx<EthByte>,
            LiteHostPort<32, 32, 4>,
            Tx<Aw<64, 4>>,
            Tx<Ar<64, 4>>,
            Tx<W<128, 16>>,
            Out<Bit>,
        ),
    ) {
        let (t_tx, t_rx) = chan::<EthByte, DefaultClock>();
        let (c_in_tx, c_in_rx) = chan::<EthByte, DefaultClock>();
        let (c_out_tx, c_out_rx) = chan::<EthByte, DefaultClock>();
        let (s_in_tx, s_in_rx) = chan::<EthByte, DefaultClock>();
        let (s_out_tx, s_out_rx) = chan::<EthByte, DefaultClock>();
        let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<Answer, DefaultClock>();
        join2(
            join2(
                join2(
                    self.tap.run(rx, (t_tx, reset)),
                    self.share.run(
                        (t_rx, c_out_rx, s_out_rx),
                        (tx, c_in_tx, s_in_tx),
                    ),
                ),
                self.stream.run((s_in_rx, b, r), (s_out_tx, aw, ar, w)),
            ),
            join2(
                self.link.run((c_in_rx, ans_rx), (ask_tx, c_out_tx)),
                self.master.run(bus, (ask_rx, ans_tx)),
            ),
        )
        .await;
    }
}

fn main() {
    print!("{}", RocketLink::<DEVICE>::verilog("rocket_link"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use txhdl::comp::{link, signal, Running};
    use txhdl_parts::bus::axi::Resp;
    use txhdl_parts::bus::axi_lite::{LiteB, LitePort, LiteR};

    /// A control ask, as the driver sends one.
    fn control(tag: u8, write: bool, addr: u32, data: u32) -> Vec<u8> {
        let mut f = vec![0xffu8; 6];
        f.extend_from_slice(&[0x02, 0, 0, 0, 0, 0x10, 0x88, 0xb5, 0x02]);
        f.extend_from_slice(&[DEVICE as u8, tag, write as u8]);
        f.extend_from_slice(&addr.to_be_bytes());
        f.extend_from_slice(&data.to_be_bytes());
        f.push(if write { 0xf } else { 0 });
        f.resize(60, 0);
        f
    }

    /// A stream ask: one beat.
    fn stream(tag: u8, write: bool, addr: u64, beat: u128) -> Vec<u8> {
        let mut f = vec![0xffu8; 6];
        f.extend_from_slice(&[0x02, 0, 0, 0, 0, 0x10, 0x88, 0xb6, 0x03]);
        f.extend_from_slice(&[DEVICE as u8, tag, write as u8]);
        f.extend_from_slice(&addr.to_be_bytes());
        f.extend_from_slice(&beat.to_le_bytes());
        let strb: u16 = if write { 0xffff } else { 0 };
        f.extend_from_slice(&strb.to_be_bytes());
        f.resize(60, 0);
        f
    }

    /// What an answer says: its EtherType's low byte, tag, word or beat,
    /// and error.
    fn answer(f: &[u8]) -> (u8, u8, u128, bool) {
        let v = if f[13] == 0xb5 {
            u32::from_be_bytes([f[22], f[23], f[24], f[25]]) as u128
        } else {
            let mut b = [0u8; 16];
            b.copy_from_slice(&f[26..42]);
            u128::from_le_bytes(b)
        };
        (f[13], f[16], v, f[17] & 1 == 1)
    }

    #[test]
    fn control_and_stream_frames_share_the_wire() {
        let (host, per) = link::<LitePort![ADDR = 32, DATA = 32, STRB = 4]>();
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
        let (aw_tx, aw_rx) = chan::<Aw<64, 4>, DefaultClock>();
        let (ar_tx, ar_rx) = chan::<Ar<64, 4>, DefaultClock>();
        let (w_tx, w_rx) = chan::<W<128, 16>, DefaultClock>();
        let (b_tx, b_rx) = chan::<B<4>, DefaultClock>();
        let (r_tx, r_rx) = chan::<R<128, 4>, DefaultClock>();
        let (reset_o, reset_i) = signal::<Bit, DefaultClock>();
        let mut unit = RocketLink::<DEVICE>::default();
        // A register file on the control port and a queue on the
        // stream port, as FPGATop's two ports behave.
        let ports = async move {
            let mut regs: HashMap<u32, u32> = HashMap::new();
            let mut queue: VecDeque<u128> = VecDeque::new();
            loop {
                DefaultClock::rising().await;
                if per.aw.peek().is_some() && per.w.peek().is_some() {
                    let a = per.aw.recv().unwrap().addr.raw() as u32;
                    regs.insert(a, per.w.recv().unwrap().data.raw() as u32);
                    per.b.send(LiteB { resp: Resp::Okay });
                }
                if let Some(ar) = per.ar.recv() {
                    let a = ar.addr.raw() as u32;
                    per.r.send(LiteR {
                        data: U::from(*regs.get(&a).unwrap_or(&0)),
                        resp: Resp::Okay,
                    });
                }
                if aw_rx.peek().is_some() && w_rx.peek().is_some() {
                    let a = aw_rx.recv().unwrap();
                    queue.push_back(w_rx.recv().unwrap().data.raw());
                    b_tx.send(B {
                        id: a.id,
                        resp: Resp::Okay,
                    });
                }
                if let Some(a) = ar_rx.recv() {
                    r_tx.send(R {
                        id: a.id,
                        data: U::from(queue.pop_front().unwrap_or(0)),
                        resp: Resp::Okay,
                        last: Bit::One,
                    });
                }
            }
        };
        let mut sim = Running::new(join2(
            unit.run(
                (in_rx, b_rx, r_rx),
                (out_tx, host, aw_tx, ar_tx, w_tx, reset_o),
            ),
            ports,
        ));
        let beat = 0x0f0e0d0c_0b0a0908_07060504_03020100u128;
        // Interleaved, one at a time as the driver sends them.
        let frames = [
            control(1, true, 0x40, 0xc0ffee),
            stream(1, true, 0x2000, beat),
            control(2, false, 0x40, 0),
            stream(2, false, 0x2000, 0),
        ];
        let mut got = Vec::new();
        let mut cur: Vec<u8> = Vec::new();
        for f in frames.iter() {
            let mut i = 0;
            let before = got.len();
            for _ in 0..600 {
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
                        got.push(answer(&cur));
                        cur.clear();
                    }
                }
                sim.cycle();
                if i == f.len() && got.len() > before {
                    break;
                }
            }
        }
        assert_eq!(
            got,
            vec![
                (0xb5, 1, 0, false),
                (0xb6, 1, 0, false),
                (0xb5, 2, 0xc0ffee, false),
                (0xb6, 2, beat, false),
            ]
        );
        assert!(!reset_i.get().to_bool());
    }

    /// A reset frame for `dev`, padded as the MAC pads it.
    fn reset_frame(dev: u8) -> Vec<u8> {
        let mut f = vec![0xffu8; 6];
        f.extend_from_slice(&[0x02, 0, 0, 0, 0, 0x10, 0x88, 0xb5, 0x04, dev]);
        f.resize(60, 0);
        f
    }

    /// Sends `frames` through a tap whose output is slow every third
    /// cycle. Returns the bytes that came out, and the cycles on which
    /// `reset` was high, counted from the first.
    fn tap(frames: &[Vec<u8>]) -> (Vec<u8>, Vec<usize>) {
        let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
        let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
        let (reset_o, reset_i) = signal::<Bit, DefaultClock>();
        let mut unit = ResetTap::<DEVICE>::default();
        let mut sim = Running::new(unit.run(in_rx, (out_tx, reset_o)));
        let bytes: Vec<(u8, bool)> = frames
            .iter()
            .flat_map(|f| {
                f.iter().enumerate().map(|(i, b)| (*b, i == f.len() - 1))
            })
            .collect();
        let (mut i, mut out, mut high) = (0, Vec::new(), Vec::new());
        for cycle in 0..bytes.len() * 3 + 20 {
            if i < bytes.len() && in_tx.ready().to_bool() {
                in_tx.send(EthByte {
                    data: U::from(bytes[i].0),
                    last: Bit::from(bytes[i].1),
                });
                i += 1;
            }
            if cycle % 3 != 2 {
                if let Some(b) = out_rx.recv() {
                    out.push(b.data.raw() as u8);
                }
            }
            sim.cycle();
            if reset_i.get().to_bool() {
                high.push(cycle);
            }
        }
        (out, high)
    }

    #[test]
    fn a_reset_frame_raises_reset_for_one_cycle_and_passes_on() {
        let frames = vec![control(1, false, 0, 0), reset_frame(DEVICE as u8)];
        let (out, high) = tap(&frames);
        assert_eq!(out, frames.concat());
        assert_eq!(high.len(), 1);
    }

    #[test]
    fn other_frames_do_not_raise_reset() {
        let mut short = reset_frame(DEVICE as u8);
        short.truncate(15);
        let mut stream_kind = reset_frame(DEVICE as u8);
        stream_kind[13] = 0xb6;
        let frames = vec![
            reset_frame(DEVICE as u8 + 1),
            short,
            stream_kind,
            control(1, true, 0, 0),
            stream(1, false, 0, 0),
        ];
        let (out, high) = tap(&frames);
        assert_eq!(out, frames.concat());
        assert!(high.is_empty());
    }

    #[test]
    fn a_reset_frame_after_a_long_frame_still_counts() {
        // A long frame's later bytes are never read as a header.
        let mut long = vec![0u8; 200];
        long[76..80].copy_from_slice(&[0x88, 0xb5, 0x04, DEVICE as u8]);
        let frames = vec![long, reset_frame(DEVICE as u8)];
        let (out, high) = tap(&frames);
        assert_eq!(out, frames.concat());
        assert_eq!(high.len(), 1);
    }
}
