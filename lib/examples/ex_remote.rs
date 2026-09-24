// SPDX-License-Identifier: Apache-2.0
//! A peripheral whose behaviour is a program on the other side of a
//! wire: the hardware runs, and what the bus talks to is software.
//!
//! The chain is the whole of it. `Remote` sits on AXI-Lite where any
//! peripheral would and answers nothing itself; each transaction
//! leaves on a channel as an `Ask`. `RemoteLink` turns that into one
//! Ethernet frame. The MAC's transmit half puts the frame on a wire,
//! the receive half at the other end takes it off, and a program
//! reads it, works out the answer, and sends a frame back the same
//! way. Nothing in the design knows that the device it is talking to
//! is twenty lines of Rust.
//!
//! The program here keeps the words it is given and turns one address
//! into the number of writes it has seen, which is a line of software
//! and a register file nobody would build. On a board it would be on
//! another machine, reached through the shim and the tunnel that
//! `//tools/remote` is the other end of, and the design would not
//! change. That is what issue 297 asks for.
//!
//! The run shows what makes it usable rather than a trick: a word
//! written is read back, a word the program computes is read, an
//! address it refuses arrives as `SlvErr` on the bus, a program that
//! stops answering does not stop the bus, and one that comes back is
//! served again.
//!
//! Both units are lowered, and the build simulates their netlists
//! against this run under nvc and Verilator.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1, LitePort};
use txhdl_parts::eth::{EthByte, EthRx, EthTx};
use txhdl_parts::remote::eth::{RemoteLink, FRAME_LEN, KIND_ANSWER};
use txhdl_parts::remote::Remote;

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the peripheral at `0x1000`.
type Bridge = LiteBridge1<32, 32, 4, 2, 0x1000, 0xf000>;

/// Where the peripheral is, as the host addresses it.
const BASE: u32 = 0x1000;

/// Which device this is on the wire, and in the frames.
const DEVICE: usize = 3;

/// How long the peripheral waits for the program before it answers
/// the bus itself. A transaction here is two frames, each stored
/// whole by a MAC before it goes, so a round trip is about seven
/// hundred cycles: a thousand is patience for a wire, and short
/// enough that the run shows the end of it.
const PATIENCE: usize = 1000;

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
    let bus: LitePort<32, 32, 4> = lite.per.into();
    let (ask_tx, ask_rx) = chan::<_, DefaultClock>();
    let (ans_tx, ans_rx) = chan::<_, DefaultClock>();
    // The bytes of a frame, between the link and a MAC at each end.
    let (out_tx, out_rx) = chan::<EthByte, DefaultClock>();
    let (in_tx, in_rx) = chan::<EthByte, DefaultClock>();
    let (prog_tx, prog_rx) = chan::<EthByte, DefaultClock>();
    let (back_tx, back_rx) = chan::<EthByte, DefaultClock>();
    // The two wires, each a byte and a valid line a cycle apart.
    let (atxd_out, atxd) = signal::<U<8>, DefaultClock>();
    let (aen_out, aen) = signal::<Bit, DefaultClock>();
    let (arxd_out, arxd) = signal::<U<8>, DefaultClock>();
    let (adv_out, adv) = signal::<Bit, DefaultClock>();
    let (aer_out, aer) = signal::<Bit, DefaultClock>();
    let (btxd_out, btxd) = signal::<U<8>, DefaultClock>();
    let (ben_out, ben) = signal::<Bit, DefaultClock>();
    let (brxd_out, brxd) = signal::<U<8>, DefaultClock>();
    let (bdv_out, bdv) = signal::<Bit, DefaultClock>();
    let (ber_out, ber) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut remote = Remote::<PATIENCE>::default();
    let mut wire = RemoteLink::<DEVICE>::default();
    let mut mac_tx = EthTx::default();
    // Each receive half says how long the frame it is offering is.
    // This example carries whole frames between two links and never
    // needs the length, so both ports are taken and ignored.
    let (alen_out, _alen) = signal::<U<16>, DefaultClock>();
    let (blen_out, _blen) = signal::<U<16>, DefaultClock>();
    let mut mac_rx = EthRx::default();
    let mut prog_mac_tx = EthTx::default();
    let mut prog_mac_rx = EthRx::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        // Every port of both lowered units, under the name that
        // unit gives it, because the testbench the build writes for
        // a netlist looks each port up in the trace by its own name.
        // Two channels are a port of each unit and so appear twice:
        // the transactions are `out` to the peripheral and `ask` to
        // the link, and the answers are `back` to both.
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("out", &ask_rx);
        wave.add("ask", &ask_rx);
        wave.add("back", &ans_rx);
        wave.add("tx", &out_rx);
        wave.add("rx", &in_rx);
        wave.add("tx_en", &aen);
        wave.add("remote", &remote);
        wave.add("wire", &wire);
        wave.start();
    }

    // Which frame the program will ignore, so that the run shows
    // what happens when it stops answering. Counted rather than
    // timed, so the run says the same thing however long a frame
    // takes.
    const IGNORE: u32 = 5;
    let said: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let log = said.clone();
    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        let say = |s: String| log.borrow_mut().push(s);
        let ok = host.write(Wr::at(BASE), &word(0xc0ffee)).await.done().await;
        say(format!("{:4}  wrote 0xc0ffee: {:?}", now(), ok.resp));
        let got = host.read(Rd::at(BASE, 1)).await.done().await;
        say(format!(
            "{:4}  read it back: {:?} {:#x}",
            now(),
            got.resp,
            got.data[0].raw()
        ));
        let n = host.read(Rd::at(BASE + 0x0c, 1)).await.done().await;
        say(format!(
            "{:4}  a word the program computes, writes so far: {}",
            now(),
            n.data[0].raw()
        ));
        let bad = host.read(Rd::at(BASE + 4, 1)).await.done().await;
        say(format!(
            "{:4}  an address it has no word for: {:?}",
            now(),
            bad.resp
        ));
        let lost = host.write(Wr::at(BASE), &word(1)).await.done().await;
        say(format!(
            "{:4}  with the program gone: {:?} after {} cycles",
            now(),
            lost.resp,
            PATIENCE
        ));
        let again = host.read(Rd::at(BASE, 1)).await.done().await;
        say(format!(
            "{:4}  and when it answers again: {:?} {:#x}",
            now(),
            again.resp,
            again.data[0].raw()
        ));
        assert_eq!(again.resp, Resp::Okay, "the bus recovered");
    };

    let mut sim = Running::new(join2(
        join2(
            join2(
                host_unit.run(host_in, host_out),
                bridge.run((aw, ar, w, lb, lr), (law, lar, lw, b, r)),
            ),
            join2(
                remote.run(bus, (ans_rx, ask_tx)),
                wire.run((ask_rx, in_rx), (ans_tx, out_tx)),
            ),
        ),
        join2(
            join2(
                mac_tx.run(out_rx, (atxd_out, aen_out)),
                mac_rx.run((arxd, adv, aer), (prog_tx, alen_out)),
            ),
            join2(
                join2(
                    prog_mac_tx.run(back_rx, (btxd_out, ben_out)),
                    prog_mac_rx.run((brxd, bdv, ber), (in_tx, blen_out)),
                ),
                client,
            ),
        ),
    ));

    // The program, at the far end of the wire. It reads whole frames,
    // answers the ones for its device, and sends the answer back as a
    // frame, which is exactly what `//tools/remote` does in Go.
    let mut words: HashMap<u32, u32> = HashMap::new();
    let mut writes = 0u32;
    let mut seen = 0u32;
    let mut frame: Vec<u8> = Vec::new();
    let mut reply: Vec<EthByte> = Vec::new();
    for _ in 0..8000 {
        if !reply.is_empty() && back_tx.ready().to_bool() {
            back_tx.send(reply.remove(0));
        }
        if let Some(byte) = prog_rx.recv() {
            frame.push(byte.data.raw() as u8);
            if byte.last.to_bool() {
                seen += 1;
                if seen != IGNORE && frame.len() >= FRAME_LEN as usize {
                    let tag = frame[16];
                    let at =
                        u32::from_be_bytes(frame[18..22].try_into().unwrap())
                            - BASE;
                    let data =
                        u32::from_be_bytes(frame[22..26].try_into().unwrap());
                    let (answer, err) = if frame[17] & 1 == 1 {
                        writes += 1;
                        words.insert(at, data);
                        (0, false)
                    } else if at == 0x0c {
                        (writes, false)
                    } else {
                        match words.get(&at) {
                            Some(v) => (*v, false),
                            None => (0, true),
                        }
                    };
                    // The answer is what it was sent, with three
                    // fields changed: the kind, the flag, the word.
                    let mut out = frame.clone();
                    out.truncate(FRAME_LEN as usize);
                    out[14] = KIND_ANSWER as u8;
                    out[16] = tag;
                    out[17] = err as u8;
                    out[22..26].copy_from_slice(&answer.to_be_bytes());
                    let n = out.len();
                    for (i, byte) in out.into_iter().enumerate() {
                        reply.push(EthByte {
                            data: U::from(byte),
                            last: Bit::from(i + 1 == n),
                        });
                    }
                }
                frame.clear();
            }
        }
        sim.cycle();
        // The two wires, each a cycle behind its transmitter.
        arxd_out.set(atxd.get());
        adv_out.set(aen.get());
        aer_out.set(Bit::Zero);
        brxd_out.set(btxd.get());
        bdv_out.set(ben.get());
        ber_out.set(Bit::Zero);
    }
    stop();
    println!("   t  what the bus and the program said");
    for line in said.borrow().iter() {
        println!("{line}");
    }
    let net = Remote::<PATIENCE>::lowered("remote");
    let link = RemoteLink::<DEVICE>::lowered("remote_link");
    txhdl::netlist::write_netlists_from_env(&[&net, &link]);
    print!("\n{}", net.verilog());
}
