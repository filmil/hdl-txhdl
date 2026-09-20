// SPDX-License-Identifier: Apache-2.0
//! A watchdog timer: the part that resets a machine whose software has
//! stopped making progress.
//!
//! A program that hangs stays hung until somebody presses the button,
//! and on a board nobody is standing next to, nobody does. So the
//! hardware counts down, and software has to keep saying that it is
//! still there; when it stops saying so, the counter reaches zero and
//! the machine restarts.
//!
//! | Offset | Name | What it is |
//! |---|---|---|
//! | `0x00` | `ctrl` | enable, window, warn, lock |
//! | `0x04` | `load` | the timeout, in cycles |
//! | `0x08` | `count` | what is left of it, read only |
//! | `0x0c` | `feed` | write `KEY` to say software is still there |
//! | `0x10` | `status` | warned, failed; write ones to clear |
//! | `0x14` | `sill` | the window opens when `count` falls to this |
//!
//! Three things make it a watchdog rather than a timer, and each is
//! there because a hung program can do more than stop.
//!
//! A refresh carries `KEY`. A program that has lost its way and is
//! writing whatever it finds over whatever it reaches will eventually
//! write this register, and a watchdog it can refresh by accident is a
//! watchdog it cannot fail. A wrong key is not ignored: it is a
//! failure, and it resets the machine at once.
//!
//! In window mode a refresh is too early while `count` is still above
//! `sill`, and too early is also a failure. A program looping tightly
//! on its refresh is as stuck as one that has stopped, and the plain
//! countdown cannot tell the two apart.
//!
//! `lock` can be set and not cleared. After it is set, `ctrl`, `load`
//! and `sill` are read only until the next reset, so a program cannot
//! turn the watchdog off, and neither can a program that has gone
//! wrong.
//!
//! The reset is a request rather than a reset: `rst_req` goes to the
//! system control block of `syscon`, which records the watchdog as the
//! cause and drives the system's reset line. A program that restarts
//! then reads why, which is the whole point of resetting it.
use txhdl::comp::{mux, Clock, DefaultClock, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};

/// `ctrl` bit 0: the watchdog counts.
pub const CTRL_ENABLE: u32 = 1;
/// `ctrl` bit 1: a refresh above `sill` is a failure.
pub const CTRL_WINDOW: u32 = 2;
/// `ctrl` bit 2: the first timeout warns instead of resetting.
pub const CTRL_WARN: u32 = 4;
/// `ctrl` bit 3: `ctrl`, `load` and `sill` are read only from now on.
pub const CTRL_LOCK: u32 = 8;

/// `status` bit 0: the warning timeout has happened.
pub const STATUS_WARNED: u32 = 1;
/// `status` bit 1: the watchdog asked for a reset.
pub const STATUS_FAILED: u32 = 2;

// begin{state}
/// A watchdog timer on AXI-Lite.
///
/// `KEY` is the word a refresh carries. Anything else written to
/// `feed` is a failure rather than a refresh.
#[derive(Trace, Default)]
pub struct Wdog<const KEY: usize> {
    /// Enable, window, warn and lock.
    pub ctrl: Reg<U<4>>,
    /// The timeout a refresh loads.
    pub load: Reg<U<16>>,
    /// What is left of it.
    pub count: Reg<U<16>>,
    /// The window opens when `count` has fallen to this.
    pub sill: Reg<U<16>>,
    /// Warned, and failed.
    pub status: Reg<U<2>>,
    /// Cycles of reset request left to drive.
    pub hold: Reg<U<4>>,
}
// end{state}

// begin{run}
#[lower]
impl<const KEY: usize> Unit for Wdog<KEY> {
    async fn run(
        &mut self,
        (aw, ar, w): (Rx<LiteAw<32>>, Rx<LiteAr<32>>, Rx<LiteW<32, 4>>),
        (b, r, rst_req, irq): (Tx<LiteB>, Tx<LiteR<32>>, Out<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let ctrl = self.ctrl.get();
            let load = self.load.get();
            let count = self.count.get();
            let sill = self.sill.get();
            let status = self.status.get();
            let hold = self.hold.get();
            // The bus.
            let arh = ar.head();
            let awh = aw.head();
            let wh = w.head();
            let rsel = arh.addr.slice::<2, 3>();
            let wsel = awh.addr.slice::<2, 3>();
            let rgo = r.ready() & ar.peek().is_some();
            let _ = ar.recv_if(r.ready());
            let wgo = b.ready() & aw.peek().is_some() & w.peek().is_some();
            let _ = aw.recv_if(wgo);
            let _ = w.recv_if(wgo);
            let written = wh.data;
            let enabled = ctrl.bit(0);
            let window = ctrl.bit(1);
            let warn = ctrl.bit(2);
            let locked = ctrl.bit(3);
            let warned = status.bit(0);
            // A write to `feed`, and whether it was a refresh or a
            // failure. Too early counts as a failure only in window
            // mode; a wrong key counts in either.
            let feeding = wgo & (wsel == 3);
            let right = written == U::<32>::from(KEY as u32);
            let early = window & (count > sill);
            let refresh = feeding & right & !early;
            let misfed = feeding & (!right | early);
            // The countdown, and what the end of it means. The first
            // timeout warns if warning is on and has not happened;
            // otherwise it resets.
            let expired = enabled & (count == 0);
            let warning = expired & warn & !warned;
            let failing = misfed | (expired & !warning);
            // The three registers a lock freezes.
            let writable = wgo & !locked;
            let holding = hold != 0;
            let word = select!(rsel.raw() => {
                0 => ctrl.zext::<32>(),
                1 => load.zext::<32>(),
                2 => count.zext::<32>(),
                4 => status.zext::<32>(),
                5 => sill.zext::<32>(),
                _ => U::<32>::from(0u8),
            });
            with!(self <= {
                // The lock bit sets and does not clear, so that a
                // program cannot take back what it locked.
                writable & (wsel == 0) ? ctrl: written.slice::<0, 4>()
                    | (ctrl & U::<4>::from(8u8)),
                writable & (wsel == 1) ? load: written.slice::<0, 16>(),
                writable & (wsel == 5) ? sill: written.slice::<0, 16>(),
                // A refresh, a warning and a failure all reload. The
                // failure reloads because the count is at zero when it
                // happens: a watchdog that did not reload would ask
                // for a reset on every cycle from then on, and hold
                // the machine down rather than restart it.
                refresh | warning | failing ? count: load,
                enabled & !refresh & !warning & !failing & (count != 0)
                    ? count: count - 1,
                status: (status | warning.zext::<2>()
                    | (failing.zext::<2>() << 1))
                    & mux(
                        wgo & (wsel == 4),
                        !written.slice::<0, 2>(),
                        U::<2>::from(3u8),
                    ),
                failing ? hold: U::<4>::from(8u8),
                holding & !failing ? hold: hold - 1,
            });
            if rgo.to_bool() {
                r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                b.send(LiteB { resp: Resp::Okay });
            }
            // The reset is asked for over several cycles, as the
            // system control block's own is, so that every part of a
            // design sees it. The warning is a level: it stands until
            // a program clears the bit.
            rst_req.set(failing | Bit::from(holding));
            irq.set(warned & warn);
        }
    }
}
// end{run}
