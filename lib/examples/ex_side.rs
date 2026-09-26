// SPDX-License-Identifier: Apache-2.0
//! A side declared in another crate, passed whole to a child.
//!
//! `LitePort`, the five channels a peripheral holds on an AXI-Lite link,
//! is a struct of ports declared in `txhdl_parts`. A unit in any crate
//! may take it as one side of `run`, and since issue 483 its netlist has
//! the five ports. A unit of units could take it too, but it could not
//! pass the whole side on to a child: `#[lower]` reads the parent's file
//! and cannot see the fields of a struct declared elsewhere, so it could
//! not say which of the parent's ports joins which of the child's.
//!
//! It no longer needs to. The side's ports are listed when `lowered`
//! runs, from the `Ports` the struct derives, in the order it declares
//! them, and the parent's are joined to the child's in that order
//! (issue 498). So `self.gpio.run(bus, ..)` is one join of five
//! channels, and nothing in the parent names them.
//!
//! This is `ex_gpio`'s run with the pins inside a unit of units, and the
//! build simulates the parent's netlist against it under nvc and
//! Verilator.
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, DefaultClock, In, Out, Running, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge, LitePort};
use txhdl_parts::gpio::Gpio;

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the peripheral at `0x1000`.
type Bridge = LiteBridge<1, SideMap, 32, 32, 4, 2>;

/// Where the bridge's one peripheral is: a nibble of the address
/// space at 0x1000.
pub struct SideMap;

impl AddrMap<1> for SideMap {
    const RANGES: [(usize, usize); 1] = [(0x1000, 0xf000)];
}

// begin{unit}
/// Eight pins, inside a unit of units that takes the link whole and
/// passes it on whole.
#[derive(Trace, Default)]
pub struct Wrapped {
    pub gpio: Gpio<8>,
}

#[lower]
impl Unit for Wrapped {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (pins, drive, dirs, irq): (In<U<8>>, Out<U<8>>, Out<U<8>>, Out<Bit>),
    ) {
        self.gpio.run(bus, (pins, drive, dirs, irq)).await;
    }
}
// end{unit}

/// The peripheral's words, as offsets from its base.
const OUT: u32 = 0x1000;
const IN: u32 = 0x1004;
const DIR: u32 = 0x1008;
const IE: u32 = 0x100c;
const KIND: u32 = 0x1010;
const POL: u32 = 0x1014;
const STATUS: u32 = 0x1018;

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
    let (pins_drive, pins) = signal::<U<8>, DefaultClock>();
    let (drive_out, drive) = signal::<U<8>, DefaultClock>();
    let (dirs_out, dirs) = signal::<U<8>, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut wrapped = Wrapped::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("pins", &pins);
        wave.add("drive", &drive);
        wave.add("dirs", &dirs);
        wave.add("irq", &irq);
        wave.add("wrapped", &wrapped);
        wave.start();
    }

    // What the testbench holds on the pins. The client moves it, and
    // the loop below puts it on the wire between one cycle and the
    // next, which is where a switch or another chip would move it: a
    // process that set it during the cycle would race the peripheral
    // reading it, and the netlist, whose port is settled before the
    // edge, would then disagree with the run.
    let held = Rc::new(RefCell::new(0u32));

    let moved = held.clone();
    let seen = irq.clone();
    let out_line = drive.clone();
    let dir_line = dirs.clone();
    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        let put = |a: u32| Wr::at(a);
        // Four outputs, four inputs, and a pattern on the outputs.
        let dir = host.write(put(DIR), &word(0x0f)).await.done().await;
        assert_eq!(dir.resp, Resp::Okay, "the write was answered");
        host.write(put(OUT), &word(0x0a)).await.done().await;
        println!(
            "{:3}  drive {:#04x}  dir {:#04x}",
            now(),
            out_line.get().raw(),
            dir_line.get().raw()
        );

        // A pin moves, and the answer is the synchroniser's.
        *moved.borrow_mut() = 0x10;
        let read = host.read(Rd::at(IN, 1)).await.done().await;
        println!("{:3}  in    {:#04x}", now(), read.data[0].raw());
        assert_eq!(read.resp, Resp::Okay, "the read was answered");
        assert_eq!(read.data[0].raw(), 0x10, "the pin reads back");

        // An interrupt on the rising edge of pin 4.
        host.write(put(STATUS), &word(0xff)).await.done().await;
        host.write(put(KIND), &word(0x10)).await.done().await;
        host.write(put(POL), &word(0x10)).await.done().await;
        host.write(put(IE), &word(0x10)).await.done().await;
        *moved.borrow_mut() = 0x00;
        let fell = host.read(Rd::at(STATUS, 1)).await.done().await;
        println!(
            "{:3}  fell  status {:#04x}  irq {}",
            now(),
            fell.data[0].raw(),
            seen.get().to_bool() as u8
        );
        assert_eq!(fell.data[0].raw(), 0, "a fall is not a rise");
        *moved.borrow_mut() = 0x10;
        let rose = host.read(Rd::at(STATUS, 1)).await.done().await;
        println!(
            "{:3}  rose  status {:#04x}  irq {}",
            now(),
            rose.data[0].raw(),
            seen.get().to_bool() as u8
        );
        assert_eq!(rose.data[0].raw(), 0x10, "a rise fires");
        assert_eq!(seen.get().to_bool() as u8, 1, "and raises the line");

        // A one clears it, and the edge has gone.
        host.write(put(STATUS), &word(0x10)).await.done().await;
        let done = host.read(Rd::at(STATUS, 1)).await.done().await;
        println!(
            "{:3}  clear status {:#04x}  irq {}",
            now(),
            done.data[0].raw(),
            seen.get().to_bool() as u8
        );
        assert_eq!(done.data[0].raw(), 0, "the write cleared it");
        assert_eq!(seen.get().to_bool() as u8, 0, "and the line fell");

        // A level instead: the pin is still high, so the bit comes
        // back in the cycle after it is cleared.
        host.write(put(KIND), &word(0x00)).await.done().await;
        host.write(put(STATUS), &word(0x10)).await.done().await;
        let level = host.read(Rd::at(STATUS, 1)).await.done().await;
        println!(
            "{:3}  level status {:#04x}  irq {}",
            now(),
            level.data[0].raw(),
            seen.get().to_bool() as u8
        );
        assert_eq!(level.data[0].raw(), 0x10, "a level does not clear");
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, [lb], [lr]), ([law], [lar], [lw], b, r)),
        ),
        join2(
            wrapped.run(bus, (pins, drive_out, dirs_out, irq_out)),
            client,
        ),
    ));
    println!("  t  what the program saw");
    for _ in 0..220 {
        sim.cycle();
        pins_drive.set(U::from(*held.borrow()));
    }
    stop();
    let net = Wrapped::lowered("wrapped");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
