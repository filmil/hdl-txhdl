// SPDX-License-Identifier: Apache-2.0
//! The Ethernet port's receive interrupt, for Vreteno on the board: a
//! frame arriving raises the port's line, which is the interrupt
//! controller's source 3, and the core takes it, as the Zephyr driver
//! expects to (#786).
//!
//! `ethrx` shows a frame reaching memory by polling the port's pending
//! bit. This shows the same arrival reaching the core as an interrupt.
//! The program lets an arrival raise the line, enables source 3 at the
//! priority the device tree gives it, and waits about twenty seconds,
//! while the machine on the other end of the cable sends a frame. The
//! handler claims the source, reads the frame's length, acknowledges
//! the frame and completes the source. At the end the program says how
//! many interrupts it took and, whatever that was, whether the port's
//! pending bit was set when it gave up. A frame that arrived with no
//! interrupt is a pending bit with a count of zero.
#![no_std]
#![no_main]

use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use vreteno_hal::trap::{self, Frame};
use vreteno_hal::{csr, entry, halt, Plic, Uart};

/// The Ethernet port's registers, on the fifth slot of the page.
const ETH: *mut u32 = 0x3400 as *mut u32;
const RX_LENGTH: usize = vreteno_regs::ethslots::RX_LENGTH / 4;
const RX_PENDING: usize = vreteno_regs::ethslots::RX_EV_PENDING / 4;
const RX_ENABLE: usize = vreteno_regs::ethslots::RX_EV_ENABLE / 4;
/// The port's interrupt, as the device tree names it.
const ETH_SOURCE: u32 = 3;
/// How long to wait, in cycles of the core's 100 MHz: twenty seconds,
/// which the cycle counter's low half holds.
const PATIENCE: u32 = 2_000_000_000;

static mut INTERRUPTS: u32 = 0;
static mut LENGTH: u32 = 0;

/// The interrupt: claim, read the length, acknowledge, complete.
fn on_interrupt(_frame: &mut Frame, _cause: u32) {
    let source = Plic::claim();
    if source == ETH_SOURCE {
        unsafe {
            write_volatile(
                addr_of_mut!(INTERRUPTS),
                read_volatile(addr_of!(INTERRUPTS)) + 1,
            );
            write_volatile(
                addr_of_mut!(LENGTH),
                read_volatile(ETH.add(RX_LENGTH)),
            );
            write_volatile(ETH.add(RX_PENDING), 1);
        }
    }
    if source != 0 {
        Plic::complete(source);
    }
}

entry!(main);

fn main() -> ! {
    unsafe { write_volatile(ETH.add(RX_ENABLE), 1) };
    Plic::enable(ETH_SOURCE, 1);
    trap::install();
    trap::on_interrupt(trap::EXTERNAL, on_interrupt);
    csr::allow(csr::EXTERNAL);
    csr::interrupts_on();
    Uart::say(b"ethirq ready\n");
    let start = csr::cycles();
    while unsafe { read_volatile(addr_of!(INTERRUPTS)) } == 0
        && csr::cycles().wrapping_sub(start) < PATIENCE
    {}
    csr::interrupts_off();
    let pending = unsafe { read_volatile(ETH.add(RX_PENDING)) } & 1;
    Uart::say(b"ethirq ");
    Uart::put_decimal(unsafe { read_volatile(addr_of!(INTERRUPTS)) });
    Uart::say(b" interrupts, length ");
    Uart::put_decimal(unsafe { read_volatile(addr_of!(LENGTH)) });
    Uart::say(b", pending ");
    Uart::put_decimal(pending);
    Uart::put(b'\n');
    halt()
}
