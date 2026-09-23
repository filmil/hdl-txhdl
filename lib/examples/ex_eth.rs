// SPDX-License-Identifier: Apache-2.0
//! Ethernet behind AXI-Lite. A host client on an AXI4 link sends three
//! frames to the Ethernet peripheral, through the AXI-Lite bridge, and
//! reads back what the receiver took off the wire. The wire is a loop:
//! what the transmitter sends, the receiver gets a cycle later, except
//! that one byte of the second frame is flipped on the way.
//!
//! A frame goes out in one fixed burst to the transmit word: the
//! bridge makes each beat a transaction of its own at the same
//! address, so each beat is one byte. The bytes come back the same
//! way, in fixed bursts read from the receive word. The first frame is
//! short and comes back padded to sixty bytes, the second is dropped
//! because its check sequence fails, and the third comes back as it
//! went. The transmitter, the receiver and the peripheral are lowered,
//! and the build simulates the three netlists against this run under
//! nvc and under Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, BurstKind, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1};
use txhdl_parts::eth::{EthByte, EthLite, EthRx, EthTx};

/// The link: thirty-two-bit addresses and words, four lanes,
/// two-bit identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the peripheral at `0x1000`.
type Bridge = LiteBridge1<32, 32, 4, 2, 0x1000, 0xf000>;

/// The peripheral's three words.
const STATUS: u32 = 0x1000;
const TRANSMIT: u32 = 0x1004;
const RECEIVE: u32 = 0x1008;

/// A frame of `n` bytes: a broadcast destination, a source, a type,
/// and a count from `seed`.
fn frame(n: usize, seed: u8) -> Vec<u8> {
    let mut f = vec![0xff; 6];
    f.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
    f.extend_from_slice(&[0x88, 0xb5]);
    while f.len() < n {
        f.push(seed.wrapping_add(f.len() as u8));
    }
    f
}

/// A burst at one address, whole words, not moving.
fn fixed_write(addr: u32) -> Wr<32> {
    let mut wr = Wr::at(addr);
    wr.size = U::from(2u8);
    wr.burst = BurstKind::Fixed;
    wr
}

fn fixed_read(addr: u32, n: usize) -> Rd<32> {
    let mut rd = Rd::at(addr, n);
    rd.size = U::from(2u8);
    rd.burst = BurstKind::Fixed;
    rd
}

fn main() {
    let Link {
        host,
        host_in,
        host_out,
        per_in,
        per_out,
        ..
    } = axi::<32, 32, 4, 2, 4>();
    let (aw, ar, w, _, _) = per_in;
    let (_, _, b, r) = per_out;
    let lite = axi_lite::<32, 32, 4>();
    let (law, lar, lw, lb, lr) = lite.host;
    let (paw, par, pw, pb, pr) = lite.per;
    // The byte channels between the peripheral and the MAC.
    let (tx_tx, tx_rx) = chan::<EthByte, DefaultClock>();
    let (rx_tx, rx_rx) = chan::<EthByte, DefaultClock>();
    // The wire: GMII out of the transmitter and into the receiver.
    let (txd_out, txd) = signal::<U<8>, DefaultClock>();
    let (en_out, tx_en) = signal::<Bit, DefaultClock>();
    let (rxd_out, rxd) = signal::<U<8>, DefaultClock>();
    let (dv_out, rx_dv) = signal::<Bit, DefaultClock>();
    let (er_out, rx_er) = signal::<Bit, DefaultClock>();
    // The receive half now says how long the frame it is offering
    // is, which this example does not use: it reads the bytes and
    // counts them itself. It is still recorded, because the netlist
    // is simulated against this trace and a port the trace holds
    // nothing for is a port the testbench cannot drive.
    let (rxlen_out, rx_len) = signal::<U<16>, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut lite_unit = EthLite::default();
    let mut mac_tx = EthTx::default();
    let mut mac_rx = EthRx::default();
    let (sent, dropped) = (mac_tx.frames, mac_rx.dropped);

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("aw", &paw);
        wave.add("ar", &par);
        wave.add("w", &pw);
        wave.add("b", &pb);
        wave.add("r", &pr);
        wave.add("tx", &tx_rx);
        wave.add("rx", &rx_rx);
        wave.add("rx_len", &rx_len);
        wave.add("txd", &txd);
        wave.add("tx_en", &tx_en);
        wave.add("rxd", &rxd);
        wave.add("rx_dv", &rx_dv);
        wave.add("rx_er", &rx_er);
        wave.add("irq", &irq);
        wave.add("lite", &lite_unit);
        wave.add("mac_tx", &mac_tx);
        wave.add("mac_rx", &mac_rx);
        wave.start();
    }

    let frames = [frame(20, 0x10), frame(64, 0x40), frame(72, 0x80)];
    let sent_frames = frames.clone();
    let client = async move {
        let mut back: Vec<Option<Vec<u8>>> = Vec::new();
        for f in &sent_frames {
            // A frame in one fixed burst: a beat per byte, bit 8 on
            // the last.
            let beats: Vec<U<32>> = f
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let last = if i + 1 == f.len() { 0x100 } else { 0 };
                    U::from(*b as u32 | last)
                })
                .collect();
            let wr = host.write(fixed_write(TRANSMIT), &beats).await;
            assert_eq!(wr.done().await.resp, Resp::Okay);
            println!("t={:>5} sent {} bytes", now(), f.len());
            // Read it back, sixteen beats at a time, a beat with bit 9
            // clear being no byte; a frame that has not come back after
            // twelve bursts was dropped.
            let mut bytes: Vec<u8> = Vec::new();
            let mut whole = false;
            for _ in 0..12 {
                let r = host.read(fixed_read(RECEIVE, 16)).await.done().await;
                for word in r.data {
                    let v = word.raw() as u32;
                    if v & 0x200 != 0 {
                        bytes.push(v as u8);
                        whole |= v & 0x100 != 0;
                    }
                }
                if whole {
                    break;
                }
            }
            if whole {
                println!("t={:>5} received {} bytes", now(), bytes.len());
                back.push(Some(bytes));
            } else {
                println!("t={:>5} received nothing", now());
                back.push(None);
            }
        }
        let st = host.read(fixed_read(STATUS, 1)).await.done().await;
        println!("t={:>5} status {:#x}", now(), st.data[0].raw());
        let mut padded = frames[0].clone();
        padded.resize(60, 0);
        assert_eq!(back, vec![Some(padded), None, Some(frames[2].clone())]);
        println!("two frames back, the corrupted one dropped");
    };

    let hardware = join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, lb, lr), (law, lar, lw, b, r)),
        ),
        join2(
            lite_unit.run((paw, par, pw, rx_rx), (pb, pr, tx_tx, irq_out)),
            join2(
                mac_tx.run(tx_rx, (txd_out, en_out)),
                mac_rx.run((rxd, rx_dv, rx_er), (rx_tx, rxlen_out)),
            ),
        ),
    );
    let mut sim = Running::new(join2(hardware, client));
    // The wire, a cycle late. Frames are counted by `tx_en` rising,
    // and one byte of the second frame's data is flipped.
    let (mut frame_no, mut at, mut was_on) = (0, 0, false);
    for _ in 0..1900 {
        sim.cycle();
        let on = tx_en.get().to_bool();
        if on && !was_on {
            frame_no += 1;
            at = 0;
        }
        let mut byte = txd.get().raw() as u8;
        if on {
            if frame_no == 2 && at == 8 + 20 {
                byte ^= 0x01;
            }
            at += 1;
        }
        was_on = on;
        rxd_out.set(U::from(byte));
        dv_out.set(Bit::from_bool(on));
        er_out.set(Bit::Zero);
    }
    stop();
    println!(
        "frames sent {}, dropped by the receiver {}",
        sent.get().raw(),
        dropped.get().raw()
    );
    assert_eq!((sent.get().raw(), dropped.get().raw()), (3, 1));
    let lite_net = EthLite::lowered("eth_lite");
    let tx_net = EthTx::lowered("eth_tx");
    let rx_net = EthRx::lowered("eth_rx");
    txhdl::netlist::write_netlists_from_env(&[&lite_net, &tx_net, &rx_net]);
}
