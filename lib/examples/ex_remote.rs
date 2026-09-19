// SPDX-License-Identifier: Apache-2.0
//! A peripheral whose behaviour is a program: the hardware runs, and
//! what the bus talks to is written in software.
//!
//! `Remote` sits on AXI-Lite where any peripheral would. It answers
//! nothing itself: each transaction leaves on a channel as an `Ask`,
//! and an `Answer` comes back. Here the other end is a few lines of
//! Rust in the same run, a device that holds four words and turns one
//! of them into the count of writes it has seen, and the host cannot
//! tell that from hardware. On a board the same two channels would be
//! frames on a wire and the program would be on another machine,
//! which is what issue 297 is for.
//!
//! The run shows the three things that make it usable rather than a
//! trick. A word written is read back, so the program holds the
//! state. The program is slow, twenty cycles a transaction, and the
//! bus waits rather than breaking. And a program that stops answering
//! does not stop the bus: the transaction is answered `SlvErr` after
//! the peripheral's patience runs out, here forty cycles, and the one
//! after it goes through.
//!
//! The peripheral is lowered, and the build simulates its netlist
//! against this run under nvc and Verilator.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, Clock, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1};
use txhdl_parts::remote::{Answer, Ask, Remote};

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the peripheral at `0x1000`.
type Bridge = LiteBridge1<32, 32, 4, 2, 0x1000, 0xf000>;

/// Where the peripheral is, as the host addresses it.
const BASE: u32 = 0x1000;

/// How long the peripheral waits for the program before it answers
/// the bus itself. Forty cycles is far more than the program here
/// takes and little enough that the run shows the end of it.
const PATIENCE: usize = 40;

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
    let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
    let (ans_tx, ans_rx) = chan::<Answer, DefaultClock>();
    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut remote = Remote::<PATIENCE>::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("aw", &paw);
        wave.add("ar", &par);
        wave.add("w", &pw);
        wave.add("b", &pb);
        wave.add("r", &pr);
        // Named as the peripheral's ports are named, `out` and
        // `back`, because the testbench the build writes for the
        // netlist looks each port up in the trace by its own name.
        wave.add("out", &ask_rx);
        wave.add("back", &ans_rx);
        wave.add("remote", &remote);
        wave.start();
    }

    // How long the program takes to answer, and whether it answers at
    // all: the run changes both as it goes.
    let delay = Rc::new(RefCell::new(2usize));
    let deaf = Rc::new(RefCell::new(false));
    let said: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    // The device, in software. Four words, and the word at 0x0c is
    // how many writes it has been given, which is something no
    // register file does by itself and a program does in a line.
    let (d, f, log) = (delay.clone(), deaf.clone(), said.clone());
    let device = async move {
        let mut words: HashMap<u32, u32> = HashMap::new();
        let mut writes = 0u32;
        loop {
            DefaultClock::rising().await;
            let Some(ask) = ask_rx.recv() else { continue };
            if *f.borrow() {
                log.borrow_mut().push(format!(
                    "{:3}  the program ignored a request",
                    now()
                ));
                continue;
            }
            // The borrow is read out before the waiting, since a
            // borrow held across an await is a borrow held while
            // the client may write it.
            let slow = *d.borrow();
            for _ in 0..slow {
                DefaultClock::rising().await;
            }
            let at = ask.addr.raw() as u32 - BASE;
            let (data, err) = if ask.write.to_bool() {
                writes += 1;
                words.insert(at, ask.data.raw() as u32);
                (0, false)
            } else if at == 0x0c {
                (writes, false)
            } else {
                match words.get(&at) {
                    Some(v) => (*v, false),
                    None => (0, true),
                }
            };
            loop {
                if ans_tx.ready().to_bool() {
                    ans_tx.send(Answer {
                        tag: ask.tag,
                        data: U::from(data),
                        err: Bit::from(err),
                    });
                    break;
                }
                DefaultClock::rising().await;
            }
        }
    };

    let log = said.clone();
    let (slow, mute) = (delay.clone(), deaf.clone());
    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        let say = |s: String| log.borrow_mut().push(s);
        // A word written to the program, and read back from it.
        let ok = host.write(Wr::at(BASE), &word(0xc0ffee)).await.done().await;
        say(format!("{:3}  wrote 0xc0ffee: {:?}", now(), ok.resp));
        let got = host.read(Rd::at(BASE, 1)).await.done().await;
        say(format!(
            "{:3}  read it back: {:?} {:#x}",
            now(),
            got.resp,
            got.data[0].raw()
        ));
        // A word the program computes rather than stores.
        let n = host.read(Rd::at(BASE + 0x0c, 1)).await.done().await;
        say(format!("{:3}  writes so far: {}", now(), n.data[0].raw()));
        // An address the program refuses.
        let bad = host.read(Rd::at(BASE + 4, 1)).await.done().await;
        say(format!(
            "{:3}  an address it has no word for: {:?}",
            now(),
            bad.resp
        ));
        // The program made slow: twenty cycles a transaction, and the
        // bus waits for every one of them.
        *slow.borrow_mut() = 20;
        for i in 0..3u32 {
            host.write(Wr::at(BASE + 16 + 4 * i), &word(i + 1))
                .await
                .done()
                .await;
        }
        let n = host.read(Rd::at(BASE + 0x0c, 1)).await.done().await;
        say(format!(
            "{:3}  after three slow writes, writes so far: {}",
            now(),
            n.data[0].raw()
        ));
        // The program stops answering. The bus is told the device
        // failed rather than waiting for ever.
        *mute.borrow_mut() = true;
        let lost = host.write(Wr::at(BASE), &word(1)).await.done().await;
        say(format!(
            "{:3}  with the program gone: {:?} after {} cycles",
            now(),
            lost.resp,
            PATIENCE
        ));
        // And it comes back.
        *mute.borrow_mut() = false;
        *slow.borrow_mut() = 2;
        let again = host.read(Rd::at(BASE, 1)).await.done().await;
        say(format!(
            "{:3}  and when it answers again: {:?} {:#x}",
            now(),
            again.resp,
            again.data[0].raw()
        ));
        assert_eq!(again.resp, Resp::Okay, "the bus recovered");
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, lb, lr), (law, lar, lw, b, r)),
        ),
        join2(
            remote.run((paw, par, pw, ans_rx.clone()), (pb, pr, ask_tx)),
            join2(device, client),
        ),
    ));
    for _ in 0..600 {
        sim.cycle();
    }
    stop();
    println!("  t  what the bus and the program said");
    for line in said.borrow().iter() {
        println!("{line}");
    }
    let net = Remote::<PATIENCE>::lowered("remote");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
