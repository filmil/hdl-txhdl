// SPDX-License-Identifier: Apache-2.0
//! A program on another machine reads and writes this design's bus,
//! with Ethernet frames for the wire.
//!
//! `ex_remote` sends the design's transactions out to a program. This
//! is the other direction: the program asks, in a frame, `HostLink`
//! takes the ask off the wire, `LiteMaster` performs it as an AXI-Lite
//! host on the design's bus, and the answer goes back in a frame. It
//! is how FireSim's host driver reaches the simulator's control
//! registers without PCIe (issue 1931).
//!
//! The bus here has a memory of words on it, which answers `SlvErr`
//! at and above 0x1000. The program writes a word, reads it back, asks the
//! same read again under the same tag, as it does when an answer is
//! lost, and writes where the memory refuses. The repeat is answered
//! from what the master kept and never reaches the bus, which the
//! memory's count of transactions shows.
//!
//! Both units are lowered, and the build simulates their netlists
//! against this run under nvc and Verilator.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, link, now, Clock, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::Resp;
use txhdl_parts::bus::axi_lite::{LiteB, LitePort, LiteR};
use txhdl_parts::eth::EthByte;
use txhdl_parts::remote::eth::FRAME_LEN;
use txhdl_parts::remote::host::{
    HostBridge, HostLink, LiteMaster, KIND_HOST_ANSWER, KIND_HOST_ASK,
};
use txhdl_parts::remote::{Answer, Ask};

/// This design's device number on the wire.
const DEVICE: usize = 5;

/// A frame in which the program asks, padded as a MAC pads.
fn ask_frame(tag: u8, write: bool, addr: u32, data: u32) -> Vec<u8> {
    let mut f = vec![0xffu8; 6];
    f.extend_from_slice(&[0x02, 0, 0, 0, 0, 0x10]);
    let kind = KIND_HOST_ASK as u8;
    f.extend_from_slice(&[0x88, 0xb5, kind, DEVICE as u8, tag]);
    f.push(write as u8);
    f.extend_from_slice(&addr.to_be_bytes());
    f.extend_from_slice(&data.to_be_bytes());
    f.push(if write { 0xf } else { 0 });
    f.resize(60, 0);
    f
}

fn main() {
    let (host, per) = link::<LitePort<32, 32, 4>>();
    let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
    let (ans_tx, ans_rx) = chan::<Answer, DefaultClock>();
    let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
    let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
    let mut wire = HostLink::<DEVICE>::default();
    let mut master = LiteMaster::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        // Every port of both lowered units, under the name each gives
        // it. The asks and the answers are a port of both, `ask` and
        // `back`, so they appear once and serve both testbenches.
        wave.add("rx", &in_rx);
        wave.add("tx", &out_rx);
        wave.add("ask", &ask_rx);
        wave.add("back", &ans_rx);
        wave.add("bus_aw", &per.aw);
        wave.add("bus_ar", &per.ar);
        wave.add("bus_w", &per.w);
        wave.add("bus_b", &per.b);
        wave.add("bus_r", &per.r);
        wave.add("wire", &wire);
        wave.add("master", &master);
        wave.start();
    }

    // A memory of words on the bus, counting the transactions it sees.
    let seen = Rc::new(RefCell::new(0u32));
    let count = seen.clone();
    let memory = async move {
        let mut words: HashMap<u32, u32> = HashMap::new();
        loop {
            DefaultClock::rising().await;
            if per.aw.peek().is_some() && per.w.peek().is_some() {
                let a = per.aw.recv().unwrap().addr.raw() as u32;
                let w = per.w.recv().unwrap().data.raw() as u32;
                *count.borrow_mut() += 1;
                let ok = a < 0x1000;
                if ok {
                    words.insert(a, w);
                }
                let resp = if ok { Resp::Okay } else { Resp::SlvErr };
                per.b.send(LiteB { resp });
            }
            if let Some(ar) = per.ar.recv() {
                *count.borrow_mut() += 1;
                let a = ar.addr.raw() as u32;
                let resp = if a < 0x1000 { Resp::Okay } else { Resp::SlvErr };
                let data = U::from(*words.get(&a).unwrap_or(&0));
                per.r.send(LiteR { data, resp });
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

    // The program: four asks, one at a time, each waiting for its
    // answer, as FireSim's driver waits for a register.
    let asks = [
        ("write 0xc0ffee at 0x40", ask_frame(1, true, 0x40, 0xc0ffee)),
        ("read 0x40", ask_frame(2, false, 0x40, 0)),
        ("read 0x40 again, same tag", ask_frame(2, false, 0x40, 0)),
        ("write at 0x2000", ask_frame(3, true, 0x2000, 7)),
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
                    if f.len() >= FRAME_LEN as usize
                        && f[14] == KIND_HOST_ANSWER as u8
                    {
                        let word =
                            u32::from_be_bytes([f[22], f[23], f[24], f[25]]);
                        said.push(format!(
                            "{:4}  {:26} tag {}  {:#010x}  {}  bus saw {}",
                            now(),
                            what,
                            f[16],
                            word,
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
    println!("   t  the program's ask             answer");
    for line in &said {
        println!("{line}");
    }
    let net = HostLink::<DEVICE>::lowered("host_link");
    let master = LiteMaster::lowered("lite_master");
    // The two joined, as the FireSim top on the AX7A200B instantiates
    // them: the run above joins them by the same channels.
    let bridge = HostBridge::<DEVICE>::lowered("host_bridge");
    txhdl::netlist::write_netlists_from_env(&[&net, &master, &bridge]);
}
