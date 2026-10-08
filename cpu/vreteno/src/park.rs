// SPDX-License-Identifier: Apache-2.0
//! Hart 1's boot memory (issue 1408): where the second hart waits until
//! the first starts it.
//!
//! Hart 1 does not run the board's program. It turns on its software
//! interrupt in `mie`, with `mstatus.MIE` left off, so that a `wfi`
//! wakes on it and no trap is taken, and waits. When its `msip` is set
//! it clears it, reads the mailbox, `0xc000` in the timer's window for
//! the address to start at and `0xc004` for an argument, empties its
//! instruction cache with `fence.i`, since hart 0 has just written the
//! code and nothing tells this hart's cache, and jumps there with its
//! number in `a0` and the argument in `a1`. A program that is done
//! jumps to zero and waits again.
//!
//! So starting hart 1 is: write the code, `fence`, write the mailbox,
//! write one to hart 1's `msip` at `0x0200_0004`.
use crate::isa::{
    beq, csrrs, csrrsi, fence_i, jalr, lui, lw, sw, wfi, CLINT_BASE,
    CSR_MHARTID, CSR_MIE,
};

/// The mailbox's page in the timer's window.
pub const MAILBOX: u32 = CLINT_BASE + 0xc000;

/// The words of hart 1's boot memory, from address zero.
pub fn text() -> Vec<u32> {
    vec![
        // mie.MSIE, the software interrupt, which wakes a `wfi`.
        csrrsi(0, CSR_MIE, 8),
        lui(5, CLINT_BASE >> 12),
        lui(6, MAILBOX >> 12),
        // Wait until `msip` says go.
        wfi(),
        lw(7, 5, 4),
        beq(7, 0, -8),
        sw(0, 5, 4),
        // Where to start and with what, and who this is.
        lw(28, 6, 0),
        lw(11, 6, 4),
        csrrs(10, CSR_MHARTID, 0),
        fence_i(),
        jalr(0, 28, 0),
    ]
}
