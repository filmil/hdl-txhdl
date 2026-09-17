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
//! external interrupt, and the core traps to `trap`. The handler claims
//! the source, reads one byte, and completes the source. A byte left in
//! the buffer keeps the port's line high, so the controller asks again
//! on the complete, and the next byte is the next interrupt. Once four
//! bytes have come in, the program says what they were and how many
//! interrupts brought them, and stops.
//!
//! The core's pending bit for the external interrupt is set by its line
//! and cleared only by software, so the handler clears it after the
//! complete, when the controller's line has already fallen with the
//! claim; a request still waiting raises it again.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

/// The serial port, as `hello.rs` has it; the third word is the oldest
/// byte received.
const UART: *mut u32 = 0x3000 as *mut u32;
/// The interrupt controller, at the RISC-V base.
const PLIC: usize = 0x0c00_0000;
/// Its words, by the standard offsets: source 1's priority, the enable
/// bits, and claim and complete.
const PRIORITY1: *mut u32 = (PLIC + 4) as *mut u32;
const ENABLE: *mut u32 = (PLIC + 0x2000) as *mut u32;
const CLAIM: *mut u32 = (PLIC + 0x20_0004) as *mut u32;
/// The serial port's receive interrupt is the controller's source 1.
const UART_SOURCE: u32 = 1;
/// How many bytes the program waits for.
const WANT: u32 = 4;

/// The bytes the handler took, how many, and how many interrupts it
/// served. The loop in `main` reads them with `read_volatile`, so the
/// optimiser does not assume they never change.
static mut GOT: [u8; 4] = [0; 4];
static mut COUNT: u32 = 0;
static mut INTERRUPTS: u32 = 0;

fn put(byte: u8) {
    unsafe {
        while UART.add(1).read_volatile() & 1 != 0 {}
        write_volatile(UART, byte as u32);
    }
}

fn say(line: &[u8]) {
    for byte in line {
        put(*byte);
    }
}

/// The interrupt, after the entry stub has saved the registers a call
/// may change. One claim, one byte, one complete.
#[no_mangle]
extern "C" fn on_interrupt() {
    unsafe {
        write_volatile(
            addr_of_mut!(INTERRUPTS),
            read_volatile(addr_of!(INTERRUPTS)) + 1,
        );
        let source = read_volatile(CLAIM);
        if source == UART_SOURCE {
            let byte = UART.add(2).read_volatile() as u8;
            let n = read_volatile(addr_of!(COUNT));
            if n < WANT {
                write_volatile(
                    addr_of_mut!(GOT).cast::<u8>().add(n as usize),
                    byte,
                );
                write_volatile(addr_of_mut!(COUNT), n + 1);
            }
        }
        if source != 0 {
            write_volatile(CLAIM, source);
        }
    }
}

/// The trap vector: save what a call may change, handle, clear the
/// core's pending bit for the external interrupt, restore, return.
/// Nothing else traps in this program, so every trap is the interrupt.
#[no_mangle]
#[link_section = ".text"]
#[unsafe(naked)]
pub extern "C" fn trap() -> ! {
    core::arch::naked_asm!(
        "addi sp, sp, -64",
        "sw ra, 0(sp)",
        "sw t0, 4(sp)",
        "sw t1, 8(sp)",
        "sw t2, 12(sp)",
        "sw t3, 16(sp)",
        "sw t4, 20(sp)",
        "sw t5, 24(sp)",
        "sw t6, 28(sp)",
        "sw a0, 32(sp)",
        "sw a1, 36(sp)",
        "sw a2, 40(sp)",
        "sw a3, 44(sp)",
        "sw a4, 48(sp)",
        "sw a5, 52(sp)",
        "sw a6, 56(sp)",
        "sw a7, 60(sp)",
        "call {handler}",
        // mip.MEIP, bit 11.
        "li t0, 0x800",
        "csrc mip, t0",
        "lw ra, 0(sp)",
        "lw t0, 4(sp)",
        "lw t1, 8(sp)",
        "lw t2, 12(sp)",
        "lw t3, 16(sp)",
        "lw t4, 20(sp)",
        "lw t5, 24(sp)",
        "lw t6, 28(sp)",
        "lw a0, 32(sp)",
        "lw a1, 36(sp)",
        "lw a2, 40(sp)",
        "lw a3, 44(sp)",
        "lw a4, 48(sp)",
        "lw a5, 52(sp)",
        "lw a6, 56(sp)",
        "lw a7, 60(sp)",
        "addi sp, sp, 64",
        "mret",
        handler = sym on_interrupt,
    )
}

/// The entry point, as in `hello.rs`.
#[no_mangle]
#[link_section = ".text.init"]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::asm!(
        "la sp, __stack_top",
        "la t0, __bss_start",
        "la t1, __bss_end",
        "1:",
        "beq t0, t1, 2f",
        "sw zero, 0(t0)",
        "addi t0, t0, 4",
        "j 1b",
        "2:",
        "j {main}",
        main = sym main,
        options(noreturn)
    )
}

#[no_mangle]
extern "C" fn main() -> ! {
    unsafe {
        // The controller: source 1 at priority 1, enabled; the
        // threshold is 0 from the reset.
        write_volatile(PRIORITY1, 1);
        write_volatile(ENABLE, 1 << UART_SOURCE);
        // The core: the vector, the external interrupt enabled, and
        // interrupts on.
        core::arch::asm!(
            "la t0, {trap}",
            "csrw mtvec, t0",
            "li t0, 0x800",
            "csrs mie, t0",
            "csrsi mstatus, 8",
            trap = sym trap,
            out("t0") _,
        );
    }
    say(b"ready\n");
    // The wait. Nothing here reads the serial port.
    while unsafe { read_volatile(addr_of!(COUNT)) } < WANT {}
    unsafe { core::arch::asm!("csrci mstatus, 8") };
    say(b"got ");
    for i in 0..WANT as usize {
        put(unsafe { read_volatile(addr_of!(GOT).cast::<u8>().add(i)) });
    }
    say(b" in ");
    put(b'0' + unsafe { read_volatile(addr_of!(INTERRUPTS)) } as u8);
    say(b" interrupts\n");
    unsafe { core::arch::asm!("ebreak", options(noreturn)) }
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { core::arch::asm!("ebreak", options(noreturn)) }
}
