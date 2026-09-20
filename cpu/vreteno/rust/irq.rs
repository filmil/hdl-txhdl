// SPDX-License-Identifier: Apache-2.0
//! Input by interrupt, for Vreteno on the board: the serial port's
//! receive interrupt goes through the platform-level interrupt
//! controller to the core, and the program takes one interrupt per
//! byte instead of polling for input.
//!
//! The program says `ready`, which the terminal answers by typing four
//! bytes, and then waits in a loop that never touches the serial port.
//! Each byte that lands in the port's buffer raises its interrupt line,
//! which is the controller's source 1; the controller raises the core's
//! external interrupt, and the core traps to the crate's entry, which
//! calls the handler registered for that line. The handler claims the
//! source, reads one byte, and completes the source. A byte left in
//! the buffer keeps the port's line high, so the controller asks again
//! on the complete, and the next byte is the next interrupt. Once four
//! bytes have come in, the program says what they were and how many
//! interrupts brought them, and stops.
//!
//! The trap vector, the saving and restoring of registers, and the
//! clearing of the core's pending bit for the external interrupt, which
//! this file once did by hand, are the crate's now (issue 142).
#![no_std]
#![no_main]

use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use vreteno_hal::trap::{self, Frame};
use vreteno_hal::{csr, entry, halt, Plic, Uart};

/// How many bytes the program waits for.
const WANT: u32 = 4;

/// The bytes the handler took, how many, and how many interrupts it
/// served. The loop in `main` reads them with `read_volatile`, so the
/// optimiser does not assume they never change.
static mut GOT: [u8; 4] = [0; 4];
static mut COUNT: u32 = 0;
static mut INTERRUPTS: u32 = 0;

/// The interrupt: one claim, one byte, one complete.
fn on_interrupt(_frame: &mut Frame, _cause: u32) {
    unsafe {
        write_volatile(
            addr_of_mut!(INTERRUPTS),
            read_volatile(addr_of!(INTERRUPTS)) + 1,
        );
    }
    let source = Plic::claim();
    if source == Plic::UART {
        let byte = Uart::take();
        let n = unsafe { read_volatile(addr_of!(COUNT)) };
        if n < WANT {
            unsafe {
                write_volatile(
                    addr_of_mut!(GOT).cast::<u8>().add(n as usize),
                    byte,
                );
                write_volatile(addr_of_mut!(COUNT), n + 1);
            }
        }
    }
    if source != 0 {
        Plic::complete(source);
    }
}

entry!(main);

fn main() -> ! {
    // The controller: source 1 at priority 1, enabled; the threshold
    // is 0 from the reset. The core: the vector, the external
    // interrupt allowed, and interrupts on.
    Plic::enable(Plic::UART, 1);
    trap::install();
    trap::on_interrupt(trap::EXTERNAL, on_interrupt);
    csr::allow(csr::EXTERNAL);
    csr::interrupts_on();
    Uart::say(b"ready\n");
    // The wait. Nothing here reads the serial port.
    while unsafe { read_volatile(addr_of!(COUNT)) } < WANT {}
    csr::interrupts_off();
    Uart::say(b"got ");
    let mut i = 0;
    while i < WANT as usize {
        Uart::put(unsafe { read_volatile(addr_of!(GOT).cast::<u8>().add(i)) });
        i += 1;
    }
    Uart::say(b" in ");
    Uart::put(b'0' + unsafe { read_volatile(addr_of!(INTERRUPTS)) } as u8);
    Uart::say(b" interrupts\n");
    halt()
}
