// SPDX-License-Identifier: Apache-2.0
//! The Ethernet host link of FireSim's Rocket design on the AX7A200B
//! (issue 1988): the Verilog of `rocket_link`, on standard output.
//!
//! One wire, two users, split by EtherType in `EthShare`: the control
//! frames (0x88b5) to `HostLink` and `LiteMaster`, which perform them on FPGATop's
//! AXI4-Lite control port, and every other frame to `StreamBridge`,
//! which takes the stream frames (0x88b6) and performs them on the
//! 128-bit `cpu_managed` port. The three are joined inside one unit, so
//! the lowering buffers the channels between them; joined by wires in
//! the top, each channel's valid would depend on its own ready.
use txhdl::comp::{chan, join2, DefaultClock, Rx, Tx, Unit};
use txhdl::{lower, Trace};
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

/// The shared wire and both bridges.
#[derive(Trace, Default)]
pub struct RocketLink<const DEV: usize> {
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
        (tx, bus, aw, ar, w): (
            Tx<EthByte>,
            LiteHostPort<32, 32, 4>,
            Tx<Aw<64, 4>>,
            Tx<Ar<64, 4>>,
            Tx<W<128, 16>>,
        ),
    ) {
        let (c_in_tx, c_in_rx) = chan::<EthByte, DefaultClock>();
        let (c_out_tx, c_out_rx) = chan::<EthByte, DefaultClock>();
        let (s_in_tx, s_in_rx) = chan::<EthByte, DefaultClock>();
        let (s_out_tx, s_out_rx) = chan::<EthByte, DefaultClock>();
        let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<Answer, DefaultClock>();
        join2(
            join2(
                self.share
                    .run((rx, c_out_rx, s_out_rx), (tx, c_in_tx, s_in_tx)),
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
    use txhdl::comp::{link, Clock, Running};
    use txhdl::types::{Bit, U};
    use txhdl_parts::bus::axi::Resp;
    use txhdl_parts::bus::axi_lite::{LiteB, LitePort, LiteR};
    use txhdl_parts::LitePort;

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
            unit.run((in_rx, b_rx, r_rx), (out_tx, host, aw_tx, ar_tx, w_tx)),
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
    }
}
