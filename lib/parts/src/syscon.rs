// SPDX-License-Identifier: Apache-2.0
//! The system control block: who this chip is, why it last restarted,
//! and how to make it restart again.
//!
//! Every program that runs on more than one build wants three things
//! the hardware alone can answer. Which design am I running on. Why
//! did I start: was the power just applied, did somebody press the
//! button, did the watchdog give up, or did the last program ask to
//! start again. And how do I hand a word to whatever runs next.
//!
//! | Offset | Name | What it is |
//! |---|---|---|
//! | `0x00` | `id` | the design's identifier, a constant |
//! | `0x04` | `version` | its version, a constant |
//! | `0x08` | `stamp0` | the build stamp, low word, a constant |
//! | `0x0c` | `stamp1` | the build stamp, high word, a constant |
//! | `0x10` | `cause` | why the system last reset; write one to clear |
//! | `0x14` | `reset` | write `KEY` to reset the system |
//! | `0x18` | `scratch` | a word that survives a reset |
//!
//! The four constants come from the type's parameters, so a build can
//! put a commit in the hardware and a program can read it back and
//! say which bitstream it is running on.
//!
//! `cause` has a bit per reason, and they accumulate: a system that
//! was reset by the button and then by software has both, until a
//! program writes ones to clear them. That is deliberate. A cause
//! register that only held the last reason would lose the interesting
//! case, which is the one where two things went wrong.
//!
//! `scratch` and `cause` are the two registers a reset does not
//! touch, which is what makes them useful: a bootloader leaves a word
//! in `scratch` and resets into the program that reads it.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

/// Why the system reset, a bit each.
pub const CAUSE_POWER: u32 = 1;
/// The button was pressed.
pub const CAUSE_BUTTON: u32 = 2;
/// The watchdog gave up.
pub const CAUSE_WATCHDOG: u32 = 4;
/// A program asked.
pub const CAUSE_SOFTWARE: u32 = 8;

// begin{state}
/// The system control block.
///
/// `ID` and `VERSION` say what the design is. `STAMP0` and `STAMP1`
/// are two words a build puts in, a commit and a date for instance.
/// `KEY` is the word that has to be written to `reset` for a reset to
/// happen, so that a wild store does not restart the machine.
#[derive(Trace, Default)]
pub struct Syscon<
    const ID: usize,
    const VERSION: usize,
    const STAMP0: usize,
    const STAMP1: usize,
    const KEY: usize,
> {
    /// Why the system last reset, one bit per reason, accumulating.
    pub cause: Reg<U<4>>,
    /// A word that no reset clears.
    pub scratch: Reg<U<32>>,
    /// Cycles of reset left to drive, after a program asked for one.
    pub hold: Reg<U<4>>,
}
// end{state}

// begin{run}
#[lower]
impl<
        const ID: usize,
        const VERSION: usize,
        const STAMP0: usize,
        const STAMP1: usize,
        const KEY: usize,
    > Unit for Syscon<ID, VERSION, STAMP0, STAMP1, KEY>
{
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (por, button, wdog, srst): (In<Bit>, In<Bit>, In<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let cause = self.cause.get();
            let scratch = self.scratch.get();
            let hold = self.hold.get();
            let por = por.get();
            let button = button.get();
            let wdog = wdog.get();
            // The bus.
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let rsel = arh.addr.slice::<2, 3>();
            let wsel = awh.addr.slice::<2, 3>();
            let rgo = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let written = wh.data;
            // A reset happens when the key is written and not
            // otherwise, so that a store to the wrong address cannot
            // restart the machine.
            let asked =
                wgo & (wsel == 5) & (written == U::<32>::from(KEY as u32));
            let holding = hold != 0;
            // What the outside is doing to us, and what we are doing
            // to ourselves.
            let now = por.zext::<4>()
                | (button.zext::<4>() << 1)
                | (wdog.zext::<4>() << 2)
                | (asked.zext::<4>() << 3);
            let clearing =
                mux(wgo & (wsel == 4), cause & !written.slice::<0, 4>(), cause);
            let word = select!(rsel.raw() => {
                0 => U::<32>::from(ID as u32),
                1 => U::<32>::from(VERSION as u32),
                2 => U::<32>::from(STAMP0 as u32),
                3 => U::<32>::from(STAMP1 as u32),
                4 => cause.zext::<32>(),
                6 => scratch,
                _ => U::<32>::from(0u8),
            });
            with!(self <= {
                cause: clearing | now,
                asked ? hold: U::<4>::from(8u8),
                holding & !asked ? hold: hold - 1,
                wgo & (wsel == 6) ? scratch: written,
            });
            if rgo.to_bool() {
                bus.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
            // The reset the rest of the system takes: whatever the
            // outside is asserting, and the tail of a reset a program
            // asked for.
            srst.set(por | button | wdog | asked | Bit::from(holding));
        }
    }
}
// end{run}
