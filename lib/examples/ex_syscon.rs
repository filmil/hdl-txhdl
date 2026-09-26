// SPDX-License-Identifier: Apache-2.0
//! The system control block: who the chip is, why it restarted, and a
//! word that survives the restart.
//!
//! A host client on an AXI4 link reaches the block through the
//! AXI-Lite bridge and does what a program does when it starts. It
//! reads the identifier, the version and the two words of the build
//! stamp, and it reads why it is running: the testbench held the
//! power-on line for the first cycles, so the answer is the power.
//!
//! Then it leaves a word in `scratch`, clears the cause, and asks for
//! a reset by writing the key. The reset line goes high for the eight
//! cycles the block holds it, the cause comes back with the software
//! bit set, and the word in `scratch` is still there, which is the
//! whole point of the register: a bootloader leaves something for the
//! program it starts.
//!
//! Last it presses the button, and finds the button's bit beside the
//! software one, because the causes accumulate until a program clears
//! them. A register that held only the last reason would lose the
//! interesting case, which is the one where two things went wrong.
//!
//! The block is lowered, and the build simulates its netlist against
//! this run under nvc and Verilator.
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, DefaultClock, Running, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, Host, Link, Rd, Reply, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge, LitePort};
use txhdl_parts::syscon::{
    regs, Syscon, CAUSE_BUTTON, CAUSE_POWER, CAUSE_SOFTWARE,
};

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the block at `0x1000`.
type Bridge = LiteBridge<1, SysconMap, 32, 32, 4, 2>;

/// Where the bridge's one peripheral is: a nibble of the address
/// space at 0x1000.
pub struct SysconMap;

impl AddrMap<1> for SysconMap {
    const RANGES: [(usize, usize); 1] = [(0x1000, 0xf000)];
}

/// What this design says it is, and the two words a build would fill
/// in with a commit and a date.
const ID: usize = 0x7478_0001;
const VERSION: usize = 0x0001_0000;
const STAMP0: usize = 0x90d9_f30b;
const STAMP1: usize = 0x2026_0917;
/// The word that has to be written to ask for a reset.
const KEY: usize = 0x5253_5421;

type Block = Syscon<ID, VERSION, STAMP0, STAMP1, KEY>;

/// The block's words: its base, and the offsets its map states.
const BASE: u32 = 0x1000;
const R_ID: u32 = BASE + regs::id;
const R_VERSION: u32 = BASE + regs::version;
const R_STAMP0: u32 = BASE + regs::stamp0;
const R_STAMP1: u32 = BASE + regs::stamp1;
const R_CAUSE: u32 = BASE + regs::cause;
const R_RESET: u32 = BASE + regs::reset;
const R_SCRATCH: u32 = BASE + regs::scratch;

/// The client that drives the block.
type Client = Host<32, 32, 4, 2, 4>;

/// One word out of the block.
async fn read(host: &Client, at: u32) -> Reply<32> {
    host.read(Rd::at(at, 1)).await.done().await
}

/// The word the program leaves for whatever runs next.
const HANDOVER: u32 = 0xb007_0001;

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
    let (por_drive, por) = signal::<Bit, DefaultClock>();
    let (button_drive, button) = signal::<Bit, DefaultClock>();
    let (wdog_drive, wdog) = signal::<Bit, DefaultClock>();
    let (srst_out, srst) = signal::<Bit, DefaultClock>();

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
        wave.add("por", &por);
        wave.add("button", &button);
        wave.add("wdog", &wdog);
        wave.add("srst", &srst);
        wave.add("block", &block);
        wave.start();
    }

    // What the testbench holds on the three lines, moved between
    // cycles from the loop, so that nothing races the block reading
    // them.
    let lines = Rc::new(RefCell::new((false, false, false)));
    let held = lines.clone();
    let reset_line = srst.clone();

    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        // Who am I, and what was I built from.
        let id = read(&host, R_ID).await;
        assert_eq!(id.resp, Resp::Okay, "the read was answered");
        let version = read(&host, R_VERSION).await.data[0].raw();
        let s0 = read(&host, R_STAMP0).await.data[0].raw();
        let s1 = read(&host, R_STAMP1).await.data[0].raw();
        println!(
            "{:3}  id {:#010x}  version {:#010x}",
            now(),
            id.data[0].raw(),
            version
        );
        println!("{:3}  built from {s0:#010x} on {s1:#010x}", now());
        assert_eq!(id.data[0].raw(), ID as u128, "the identifier");
        assert_eq!(s0, STAMP0 as u128, "the build stamp's low word");

        // Why am I running.
        let cause = read(&host, R_CAUSE).await.data[0].raw();
        println!("{:3}  cause {cause:#04x}  (the power came on)", now());
        assert_eq!(cause, CAUSE_POWER as u128, "the power-on cause");

        // Leave a word for whatever runs next, clear the cause, and
        // ask to start again.
        host.write(Wr::at(R_SCRATCH), &word(HANDOVER))
            .await
            .done()
            .await;
        host.write(Wr::at(R_CAUSE), &word(0xf)).await.done().await;
        let cleared = read(&host, R_CAUSE).await.data[0].raw();
        assert_eq!(cleared, 0, "the cause was cleared");
        host.write(Wr::at(R_RESET), &word(KEY as u32))
            .await
            .done()
            .await;
        println!(
            "{:3}  asked for a reset, line is {}",
            now(),
            line(&reset_line)
        );

        // The block does not reset itself, so both survive.
        let after = read(&host, R_CAUSE).await.data[0].raw();
        let kept = read(&host, R_SCRATCH).await.data[0].raw();
        println!("{:3}  cause {after:#04x}  scratch {kept:#010x}", now());
        assert_eq!(after, CAUSE_SOFTWARE as u128, "software asked");
        assert_eq!(kept, HANDOVER as u128, "the word survived the reset");

        // A wrong key does nothing at all.
        host.write(Wr::at(R_RESET), &word(0)).await.done().await;
        let unmoved = read(&host, R_CAUSE).await.data[0].raw();
        assert_eq!(unmoved, CAUSE_SOFTWARE as u128, "a wrong key is ignored");
        println!("{:3}  a wrong key changed nothing", now());

        // The button, beside the software bit: the causes accumulate.
        *held.borrow_mut() = (false, true, false);
        let both = read(&host, R_CAUSE).await.data[0].raw();
        println!("{:3}  cause {both:#04x}  (button and software)", now());
        assert_eq!(
            both,
            (CAUSE_SOFTWARE | CAUSE_BUTTON) as u128,
            "both reasons are kept"
        );
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, [lb], [lr]), ([law], [lar], [lw], b, r)),
        ),
        join2(block.run(bus, (por, button, wdog, srst_out)), client),
    ));

    println!("  t  what the program saw");
    for cycle in 0..420 {
        sim.cycle();
        // The power is held on for the first four cycles, as a board
        // holds it while its clock settles.
        let (_, btn, dog) = *lines.borrow();
        por_drive.set(Bit::from_bool(cycle < 4));
        button_drive.set(Bit::from_bool(btn));
        wdog_drive.set(Bit::from_bool(dog));
    }
    stop();
    let net = Block::lowered("syscon");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}

/// The reset line as a word, for printing.
fn line(s: &txhdl::comp::In<Bit>) -> u8 {
    s.get().to_bool() as u8
}
