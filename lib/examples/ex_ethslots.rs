// SPDX-License-Identifier: Apache-2.0
//! The registers a Zephyr Ethernet driver talks to, exercised the way
//! the driver will talk to them.
//!
//! The map is LiteEth's, so that `eth_litex_liteeth.c` ports rather
//! than being redesigned. This run does what that driver does, in the
//! order it does it, and checks the answers: poll `tx_ready`, set the
//! slot and length, write `tx_start`; and on the other side notice
//! `rx_ev_pending`, read the slot and length, and acknowledge by
//! writing the bit back.
//!
//! The acknowledgement is the part worth a test of its own.
//! `rx_ev_pending` is write-one-to-clear, so a write of zero must do
//! nothing. Hardware that cleared on any write would let a driver
//! acknowledge an interrupt it never looked at, and the frame would
//! be dropped rather than read. Nothing about the register's value
//! says which behaviour it has; only a run that writes zero and
//! checks the bit is still set does.
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, now, signal, Clock, DefaultClock, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::Trace;
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1};
use txhdl_parts::ethslots::EthSlots;

/// Where the four buffers begin: sixteen megabytes into the board's
/// memory, clear of where a program loads.
const BUFS: usize = 0x4100_0000;

/// The map, as LiteEth lays it out.
const BASE: u32 = 0x1000;
const RX_SLOT: u32 = BASE;
const RX_LENGTH: u32 = BASE + 0x04;
const RX_PENDING: u32 = BASE + 0x08;
const RX_ENABLE: u32 = BASE + 0x0c;
const TX_SLOT: u32 = BASE + 0x10;
const TX_LENGTH: u32 = BASE + 0x14;
const TX_START: u32 = BASE + 0x18;
const TX_READY: u32 = BASE + 0x1c;

type HostUnit = AxiHost<32, 32, 4, 2, 4>;
type Bridge = LiteBridge1<32, 32, 4, 2, 0x1000, 0xf000>;

/// The two engines, as a script rather than as the real thing.
///
/// The register block's plain inputs have to be driven by something
/// on the clock. A client inside the simulation cannot do it: it and
/// the unit are both processes of the same step, so whether the unit
/// sees a value set by the client that step is a matter of which
/// polls first, and the trace records the end of the step either
/// way. The netlist's testbench has no such ambiguity, so the two
/// disagree by one step at every change.
///
/// So the stimulus is a unit. It counts cycles and asserts the busy
/// lines on a fixed schedule, which is deterministic, is what the
/// trace records, and is what the testbench replays.
#[derive(Trace, Default)]
pub struct Engines {
    pub at: Reg<U<10>>,
}

impl Unit<(), (Out<Bit>, Out<Bit>, Out<U<16>>, Out<U<1>>)> for Engines {
    async fn run(
        &mut self,
        _i: (),
        (tx_busy, rx_busy, rx_len, rx_which): (
            Out<Bit>,
            Out<Bit>,
            Out<U<16>>,
            Out<U<1>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let at = self.at.get().raw() as u32;
            // Transmit is busy for a stretch after the start would
            // have been written, and receive stores a frame later.
            tx_busy.set(Bit::from((40..70).contains(&at)));
            // Two frames. The second one's store finishes at cycle
            // 208, which is where the client aims an acknowledgement,
            // so that the two land in the same cycle.
            let second = (190..208).contains(&at);
            rx_busy.set(Bit::from((100..130).contains(&at) || second));
            // A different length and a different slot, so that the
            // second frame is distinguishable from the first rather
            // than being a repeat that any stale register satisfies.
            let two = at >= 190;
            rx_len.set(U::<16>::from(if two { 64u32 } else { 342u32 }));
            rx_which.set(U::<1>::from(if two { 0u8 } else { 1u8 }));
            self.at.set(self.at + 1);
        }
    }
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
    let lite = axi_lite::<32, 32, 4>();
    let (law, lar, lw, lb, lr) = lite.host;
    let (paw, par, pw, pb, pr) = lite.per;

    // The engines are not here: this run is about the registers, and
    // what the engines do is checked in ex_dma and ex_dmaw. Their
    // busy lines are driven by hand so that the register block can be
    // asked what it says while a transfer is and is not running.
    let (tx_busy_o, tx_busy) = signal::<Bit, DefaultClock>();
    let (rx_busy_o, rx_busy) = signal::<Bit, DefaultClock>();
    let (rx_len_o, rx_len) = signal::<U<16>, DefaultClock>();
    let (rx_which_o, rx_which) = signal::<U<1>, DefaultClock>();
    let mut engines = Engines::default();
    let (txb_o, tx_base) = signal::<U<32>, DefaultClock>();
    let (txn_o, tx_bytes) = signal::<U<16>, DefaultClock>();
    let (txs_o, tx_start) = signal::<Bit, DefaultClock>();
    let (rxb_o, rx_base) = signal::<U<32>, DefaultClock>();
    let (irq_o, irq) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut slots = EthSlots::<BUFS>::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("aw", &paw);
        wave.add("ar", &par);
        wave.add("w", &pw);
        wave.add("b", &pb);
        wave.add("r", &pr);
        // Every port under the name the port has: the generator
        // reads the trace by those names and refuses what it cannot
        // find.
        wave.add("tx_busy", &tx_busy);
        wave.add("engines", &engines);
        wave.add("rx_busy", &rx_busy);
        wave.add("rx_len", &rx_len);
        wave.add("rx_which", &rx_which);
        wave.add("tx_base", &tx_base);
        wave.add("tx_bytes", &tx_bytes);
        wave.add("tx_start", &tx_start);
        wave.add("rx_base", &rx_base);
        wave.add("irq", &irq);
        wave.add("ethslots", &slots);
        wave.start();
    }

    let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let watch_irq = irq.clone();
    let watch_start = tx_start.clone();
    let watch_base = tx_base.clone();
    let watch_bytes = tx_bytes.clone();
    let watch_rxbase = rx_base.clone();

    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        let get = |a: u32| Rd::at(a, 1);
        let put = |a: u32| Wr::at(a);
        // Waiting to an absolute cycle rather than counting edges:
        // the bus transactions between checks take cycles of their
        // own, so a relative count drifts out of the stub's windows.
        let until = |c: u64| async move {
            while now() / 2 < c {
                DefaultClock::rising().await;
            }
        };

        // Idle, so the driver may hand over a frame.
        let ready = host.read(get(TX_READY)).await.done().await;
        assert_eq!(ready.data[0].raw(), 1, "idle, so ready to take a frame");
        host.write(put(TX_SLOT), &word(1)).await.done().await;
        host.write(put(TX_LENGTH), &word(1517)).await.done().await;
        host.write(put(RX_ENABLE), &word(1)).await.done().await;
        host.write(put(TX_START), &word(1)).await.done().await;
        DefaultClock::rising().await;
        log.borrow_mut().push(format!(
            "tx_start={} base={:#x} bytes={}",
            watch_start.get().to_bool() as u8,
            watch_base.get().raw(),
            watch_bytes.get().raw()
        ));

        // The engine is running, so the driver must be held off.
        until(50).await;
        let busy = host.read(get(TX_READY)).await.done().await;
        assert_eq!(busy.data[0].raw(), 0, "not ready while a frame is going");

        // The receive store is in flight: no interrupt yet, because
        // the frame is not in memory yet.
        until(110).await;
        assert!(!watch_irq.get().to_bool(), "no interrupt while storing");

        // The store has finished, so the frame is in memory and the
        // driver may be told.
        until(140).await;
        assert!(watch_irq.get().to_bool(), "the store finished, so told");
        log.borrow_mut()
            .push(format!("rx_base={:#x}", watch_rxbase.get().raw()));
        let slot = host.read(get(RX_SLOT)).await.done().await;
        let len = host.read(get(RX_LENGTH)).await.done().await;
        assert_eq!(slot.data[0].raw(), 1, "the slot the frame landed in");
        assert_eq!(len.data[0].raw(), 342, "its length in bytes");

        // Write one to clear, both halves.
        host.write(put(RX_PENDING), &word(0)).await.done().await;
        DefaultClock::rising().await;
        let still = host.read(get(RX_PENDING)).await.done().await;
        assert_eq!(
            still.data[0].raw(),
            1,
            "a write of zero must not acknowledge: write one to clear"
        );
        let ok = host.write(put(RX_PENDING), &word(1)).await.done().await;
        assert_eq!(ok.resp, Resp::Okay);
        DefaultClock::rising().await;
        let gone = host.read(get(RX_PENDING)).await.done().await;
        assert_eq!(gone.data[0].raw(), 0, "a write of one acknowledges");
        assert!(!watch_irq.get().to_bool(), "and drops the line");
        log.borrow_mut().push("rw1c holds".into());

        // An acknowledgement in the same cycle as an arrival.
        //
        // This is the case that decides the order of the entries in
        // the register block's `with!`. They apply in order and the
        // last drive of a field wins, so an acknowledgement written
        // after the arrival would clear the pending bit the arrival
        // had just set. The frame would be in memory, correct and
        // complete, and nothing would ever tell the driver about it.
        //
        // The cycle to issue at was measured rather than reasoned,
        // by running the whole sweep against both orders of the
        // `with!`. Only 201 tells them apart:
        //
        //   issue at   199  200  201  202  203
        //   correct     ok   ok   ok   X    X
        //   reordered   ok   ok   X    X    X
        //
        // At 199 and 200 the acknowledgement lands before the
        // arrival, so the arrival sets the bit afterwards either way
        // and the test proves nothing. At 202 and later it lands
        // after the arrival in a cycle of its own, where clearing the
        // bit is correct behaviour and both orders fail. 201 is the
        // single cycle where the two coincide, which is the whole
        // case, so the number is load bearing and not a delay that
        // happened to work.
        until(201).await;
        let ack = host.write(put(RX_PENDING), &word(1)).await.done().await;
        assert_eq!(ack.resp, Resp::Okay);
        until(215).await;
        let after = host.read(get(RX_PENDING)).await.done().await;
        assert_eq!(
            after.data[0].raw(),
            1,
            "a frame arriving as one is acknowledged must not be lost"
        );
        let slot2 = host.read(get(RX_SLOT)).await.done().await;
        let len2 = host.read(get(RX_LENGTH)).await.done().await;
        assert_eq!(slot2.data[0].raw(), 0, "the second frame's slot");
        assert_eq!(len2.data[0].raw(), 64, "and the second frame's length");
        assert!(watch_irq.get().to_bool(), "and the line is up for it");
        log.borrow_mut()
            .push("a frame arriving as one is acknowledged survives".into());
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run(
                (per_in.0, per_in.1, per_in.2, lb, lr),
                (law, lar, lw, per_out.2, per_out.3),
            ),
        ),
        join2(
            // The stub is joined FIRST so that it drives the busy
            // lines before the register block reads them. Two
            // processes of one step have no order but the join's, and
            // a wire written after it is read is a wire the trace and
            // the netlist disagree about: co-simulation reported
            // seven differences, all of them at the two edges of
            // `rx_busy`, until this order was fixed.
            //
            // Nothing here rests on the stub being scheduled first in
            // hardware, because in the design the busy lines come
            // from `LineStore`'s registered `running` and are settled
            // at the edge. The stub has no register to settle in, so
            // the join is what gives it one.
            join2(
                engines.run((), (tx_busy_o, rx_busy_o, rx_len_o, rx_which_o)),
                slots.run(
                    (paw, par, pw, tx_busy, rx_busy, rx_len, rx_which),
                    (pb, pr, txb_o, txn_o, txs_o, rxb_o, irq_o),
                ),
            ),
            client,
        ),
    ));
    for _ in 0..400 {
        sim.cycle();
    }
    for line in seen.borrow().iter() {
        println!("{line}");
    }
    println!("t={} the map answers as the driver expects", now());

    print!("\n{}", EthSlots::<BUFS>::verilog("ethslots"));
    stop();
    txhdl::netlist::write_vhdl_from_env(&EthSlots::<BUFS>::lowered("ethslots"));
}
