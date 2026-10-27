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
