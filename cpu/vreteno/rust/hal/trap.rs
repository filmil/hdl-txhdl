// SPDX-License-Identifier: Apache-2.0
//! The trap entry and the dispatcher.
//!
//! The core traps to `mtvec` with the cause in `mcause` and the
//! interrupted instruction in `mepc`. [`install`] points `mtvec` at
//! [`entry`], which saves every register a call may change and `mepc`
//! into a [`Frame`] on the stack, hands the frame to [`dispatch`], and
//! on the way back restores the registers and `mepc` from the frame
//! and returns with `mret`. So a handler is an ordinary Rust function
//! that takes the frame, and one that wants to resume after a
//! faulting or calling instruction moves `frame.mepc` on.
//!
//! An interrupt goes to the handler registered for its line with
//! [`on_interrupt`]; an exception to the one registered for its cause
//! with [`on_exception`]. An interrupt nobody registered for is taken
//! and ignored; an exception nobody registered for halts the core,
//! since a program that traps where it did not expect to has nothing
//! sensible to resume.
//!
//! The core's pending bit for the external interrupt is set by its
//! line and cleared only by software, so the dispatcher clears it after
//! the handler, when the controller's line has already fallen with the
//! claim; a request still waiting raises it again.
use core::ptr::{addr_of, addr_of_mut};

/// What the entry saves: the registers a call may change, in the
/// order the entry stores them, and `mepc`.
#[repr(C)]
pub struct Frame {
    pub ra: u32,
    pub t0: u32,
    pub t1: u32,
    pub t2: u32,
    pub t3: u32,
    pub t4: u32,
    pub t5: u32,
    pub t6: u32,
    pub a0: u32,
    pub a1: u32,
    pub a2: u32,
    pub a3: u32,
    pub a4: u32,
    pub a5: u32,
    pub a6: u32,
    pub a7: u32,
    /// The instruction the trap came from. A handler that wants to
    /// resume after it, an `ecall` say, adds four.
    pub mepc: u32,
}

/// A handler: the frame, and the cause as `mcause` had it.
pub type Handler = fn(&mut Frame, u32);

/// The exception's `mcause` for an environment call from machine mode.
pub const ECALL: u32 = 11;
/// The exception's `mcause` for an illegal instruction.
pub const ILLEGAL: u32 = 2;
/// The exception's `mcause` for `ebreak`.
pub const BREAKPOINT: u32 = 3;
/// The interrupt lines, as `mcause` numbers them below the top bit.
pub const SOFT: u32 = 3;
pub const TIMER: u32 = 7;
pub const EXTERNAL: u32 = 11;

const LINES: usize = 16;
const INTERRUPT: u32 = 1 << 31;

/// The handlers, one per cause and one per line; `None` where nobody
/// registered. Written before interrupts are on and read from the
/// handler, so the accesses are volatile and the tables are never
/// borrowed as references, which is what `static mut` asks of a
/// program on one core with no threads.
static mut EXCEPTIONS: [Option<Handler>; LINES] = [None; LINES];
static mut INTERRUPTS: [Option<Handler>; LINES] = [None; LINES];

/// Handle the exception `cause` with `h`.
pub fn on_exception(cause: u32, h: Handler) {
    unsafe {
        addr_of_mut!(EXCEPTIONS)
            .cast::<Option<Handler>>()
            .add((cause as usize) & (LINES - 1))
            .write_volatile(Some(h));
    }
}

/// Handle the interrupt on `line` with `h`.
pub fn on_interrupt(line: u32, h: Handler) {
    unsafe {
        addr_of_mut!(INTERRUPTS)
            .cast::<Option<Handler>>()
            .add((line as usize) & (LINES - 1))
            .write_volatile(Some(h));
    }
}

fn registered(
    table: *const [Option<Handler>; LINES],
    i: u32,
) -> Option<Handler> {
    unsafe {
        table
            .cast::<Option<Handler>>()
            .add((i as usize) & (LINES - 1))
            .read_volatile()
    }
}

/// Point `mtvec` at the entry. Interrupts stay off until the program
/// turns them on, once its handlers are registered.
pub fn install() {
    unsafe {
        core::arch::asm!(
            "la t0, {entry}",
            "csrw mtvec, t0",
            entry = sym entry,
            out("t0") _,
        )
    }
}

/// The dispatcher, called by the entry with the frame it saved.
#[no_mangle]
extern "C" fn vreteno_hal_dispatch(frame: &mut Frame) {
    let cause: u32;
    unsafe { core::arch::asm!("csrr {0}, mcause", out(reg) cause) }
    if cause & INTERRUPT != 0 {
        let line = cause & !INTERRUPT;
        if let Some(h) = registered(addr_of!(INTERRUPTS), line) {
            h(frame, cause);
        }
        if line == EXTERNAL {
            crate::csr::clear_pending(crate::csr::EXTERNAL);
        }
    } else {
        match registered(addr_of!(EXCEPTIONS), cause) {
            Some(h) => h(frame, cause),
            None => crate::halt(),
        }
    }
}

/// The entry: save, dispatch, restore, return. The frame is 68 bytes
/// and the stack stays aligned to 16, so 80 are taken.
#[no_mangle]
#[unsafe(naked)]
pub extern "C" fn entry() -> ! {
    core::arch::naked_asm!(
        "addi sp, sp, -80",
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
        "csrr t0, mepc",
        "sw t0, 64(sp)",
        "mv a0, sp",
        "call {dispatch}",
        "lw t0, 64(sp)",
        "csrw mepc, t0",
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
        "addi sp, sp, 80",
        "mret",
        dispatch = sym vreteno_hal_dispatch,
    )
}
