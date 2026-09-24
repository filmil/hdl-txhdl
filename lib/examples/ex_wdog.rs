// SPDX-License-Identifier: Apache-2.0
//! The watchdog: a program that keeps saying it is still there, and
//! then stops.
//!
//! A host client on an AXI4 link reaches the watchdog through the
//! AXI-Lite bridge and does what a program does with one. It sets a
//! timeout, turns the watchdog on, and refreshes it with the key; the
//! count reloads each time and the reset line stays low.
//!
//! Then it stops refreshing, and the count runs out. The warning comes
//! first, because warning is on: the interrupt rises and the count
//! reloads, which gives a program that is merely slow one more timeout
//! to notice. Nothing refreshes it, so the second timeout asks for the
//! reset.
//!
//! Last it shows the two ways a refresh can be a failure rather than a
//! refresh. A wrong key is one: a program writing whatever it finds
//! over whatever it reaches must not be able to refresh a watchdog by
//! accident. A refresh while the window is shut is the other: a
//! program looping tightly on its own refresh is as stuck as one that
//! has stopped, and a plain countdown cannot tell them apart.
//!
//! The block is lowered, and the build simulates its netlist against
//! this run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, Host, Link, Rd, Reply, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1, LitePort};
use txhdl_parts::wdog::{
    Wdog, CTRL_ENABLE, CTRL_LOCK, CTRL_WARN, CTRL_WINDOW, STATUS_FAILED,
    STATUS_WARNED,
};

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the watchdog at `0x1000`.
type Bridge = LiteBridge1<32, 32, 4, 2, 0x1000, 0xf000>;

/// The word a refresh carries.
const KEY: usize = 0x5744_4f47;

type Block = Wdog<KEY>;

/// The watchdog's words.
const R_CTRL: u32 = 0x1000;
const R_LOAD: u32 = 0x1004;
const R_COUNT: u32 = 0x1008;
const R_FEED: u32 = 0x100c;
const R_STATUS: u32 = 0x1010;
const R_SILL: u32 = 0x1014;

/// The client that drives the watchdog.
type Client = Host<32, 32, 4, 2, 4>;

/// One word out of the watchdog.
async fn read(host: &Client, at: u32) -> Reply<32> {
    host.read(Rd::at(at, 1)).await.done().await
}

/// The timeout, in cycles. A read over the bus takes about twenty six
/// of them, so the timeout is several reads long: short enough that
/// the run is not all waiting, long enough that a program can see the
/// count fall between two reads.
const TIMEOUT: u32 = 120;

/// The window opens when the count has fallen to half the timeout. A
/// refresh while more than half the time is left is too early.
const SILL: u32 = TIMEOUT / 2;

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
    let (rst_req_out, rst_req) = signal::<Bit, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut block = Block::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("rst_req", &rst_req);
        wave.add("irq", &irq);
        wave.add("block", &block);
        wave.start();
    }

    let reset_line = rst_req.clone();
    let warn_line = irq.clone();

    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        // A timeout, then on, with the warning enabled.
        host.write(Wr::at(R_LOAD), &word(TIMEOUT))
            .await
            .done()
            .await;
        host.write(Wr::at(R_FEED), &word(KEY as u32))
            .await
            .done()
            .await;
        host.write(Wr::at(R_CTRL), &word(CTRL_ENABLE | CTRL_WARN))
            .await
            .done()
            .await;
        println!("{:3}  the watchdog is on, timeout {TIMEOUT}", now());

        // The count falls on its own, and a refresh puts it back. Two
        // reads with nothing between them show the fall; a third after
        // a refresh shows the reload. A read costs about twenty six
        // cycles, which is why the timeout is several times that.
        let first = read(&host, R_COUNT).await.data[0].raw();
        let fallen = read(&host, R_COUNT).await.data[0].raw();
        host.write(Wr::at(R_FEED), &word(KEY as u32))
            .await
            .done()
            .await;
        let fed = read(&host, R_COUNT).await.data[0].raw();
        println!("{:3}  count {first}, then {fallen}, then {fed} fed", now());
        assert!(fallen < first, "the count fell between two reads");
        assert!(fed > fallen, "the refresh put it back");
        assert_eq!(line(&reset_line), 0, "and asked for no reset");

        // Now stop refreshing. The warning comes first.
        while line(&warn_line) == 0 {
            let _ = read(&host, R_COUNT).await;
        }
        let warned = read(&host, R_STATUS).await.data[0].raw();
        println!("{:3}  the warning: status {warned:#04x}", now());
        assert_eq!(warned, STATUS_WARNED as u128, "warned and not failed");
        assert_eq!(line(&reset_line), 0, "a warning is not a reset");

        // Still nothing refreshes it, so the second timeout resets.
        while line(&reset_line) == 0 {
            let _ = read(&host, R_COUNT).await;
        }
        let failed = read(&host, R_STATUS).await.data[0].raw();
        println!("{:3}  the reset asked for: status {failed:#04x}", now());
        assert_eq!(
            failed,
            (STATUS_WARNED | STATUS_FAILED) as u128,
            "warned, then failed"
        );

        // Clear both, refresh, and check the watchdog is running again
        // with nothing held against it.
        host.write(Wr::at(R_STATUS), &word(3)).await.done().await;
        host.write(Wr::at(R_FEED), &word(KEY as u32))
            .await
            .done()
            .await;
        let clean = read(&host, R_STATUS).await.data[0].raw();
        assert_eq!(clean, 0, "the status was cleared");

        // A wrong key is a failure, not a refresh.
        host.write(Wr::at(R_FEED), &word(0)).await.done().await;
        let wrong = read(&host, R_STATUS).await.data[0].raw();
        println!("{:3}  a wrong key: status {wrong:#04x}", now());
        assert_eq!(wrong, STATUS_FAILED as u128, "the wrong key failed it");

        // In window mode a refresh above the sill is a failure too.
        // The sill is half the timeout, so a refresh one read after
        // another still has most of the time left and is early.
        host.write(Wr::at(R_STATUS), &word(3)).await.done().await;
        host.write(Wr::at(R_SILL), &word(SILL)).await.done().await;
        host.write(Wr::at(R_CTRL), &word(CTRL_ENABLE | CTRL_WINDOW))
            .await
            .done()
            .await;
        host.write(Wr::at(R_FEED), &word(KEY as u32))
            .await
            .done()
            .await;
        let early = read(&host, R_COUNT).await.data[0].raw();
        host.write(Wr::at(R_FEED), &word(KEY as u32))
            .await
            .done()
            .await;
        let shut = read(&host, R_STATUS).await.data[0].raw();
        println!(
            "{:3}  a refresh at {early}, above the sill: {shut:#04x}",
            now()
        );
        assert_eq!(shut, STATUS_FAILED as u128, "too early is a failure");

        // The lock: once it is set, the watchdog cannot be turned off.
        host.write(Wr::at(R_STATUS), &word(3)).await.done().await;
        host.write(Wr::at(R_CTRL), &word(CTRL_ENABLE | CTRL_LOCK))
            .await
            .done()
            .await;
        host.write(Wr::at(R_CTRL), &word(0)).await.done().await;
        let locked = read(&host, R_CTRL).await.data[0].raw();
        println!(
            "{:3}  a write of zero to a locked ctrl: {locked:#04x}",
            now()
        );
        assert_eq!(
            locked,
            (CTRL_ENABLE | CTRL_LOCK) as u128,
            "the lock held the watchdog on"
        );
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, lb, lr), (law, lar, lw, b, r)),
        ),
        join2(block.run(bus, (rst_req_out, irq_out)), client),
    ));

    println!("  t  what the program saw");
    for _ in 0..1600 {
        sim.cycle();
    }
    stop();
    let net = Block::lowered("wdog");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}

/// A line as a word, for printing and for waiting on.
fn line(s: &txhdl::comp::In<Bit>) -> u8 {
    s.get().to_bool() as u8
}
