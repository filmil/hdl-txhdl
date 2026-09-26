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
//! Its six words are `regmap!`'s `regs`, below: `ctrl` (enable, window,
//! warn, lock), `load` (the timeout, in cycles), `count` (what is left
//! of it, read only), `feed` (write `KEY` to say software is still
//! there), `status` (warned, failed; write ones to clear) and `sill`
//! (the window opens when `count` falls to this), at `0x00` to `0x14`.
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
use txhdl::comp::{mux, Clock, DefaultClock, Out, Reg, Unit};
use txhdl::regmap;
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

// begin{map}
// The map: six words, three address bits above the byte bits selecting
// one, and the other two read zero (issues 499 and 670).
regmap! { regs (regs_read, regs_we), 3: [
    (0, ctrl, rw, "enable, window, warn and lock", [
        (enable, 0, 1, rw, 0, "the watchdog counts"),
        (window, 1, 1, rw, 0, "a refresh above sill is a failure"),
        (warn, 2, 1, rw, 0, "the first timeout warns instead of resetting"),
        (lock, 3, 1, rw, 0, "set, ctrl, load and sill are read only"),
    ]),
    (1, load, rw, "the timeout a refresh loads", [
        (cycles, 0, 16, rw, 0, "the timeout, in cycles"),
    ]),
    (2, count, ro, "what is left of the timeout", [
        (cycles, 0, 16, ro, 0, "what is left, in cycles"),
    ]),
    (3, feed, wo, "written with KEY: software is still there"),
    (4, status, w1c, "warned and failed; write ones to clear", [
        (warned, 0, 1, w1c, 0, "the warning timeout has happened"),
        (failed, 1, 1, w1c, 0, "the watchdog asked for a reset"),
    ]),
    (5, sill, rw, "the window opens when count falls to this", [
        (cycles, 0, 16, rw, 0, "the sill, in cycles"),
    ]),
] }
// end{map}

/// `ctrl` bit 0: the watchdog counts.
pub const CTRL_ENABLE: u32 = regs::ctrl_enable.mask();
/// `ctrl` bit 1: a refresh above `sill` is a failure.
pub const CTRL_WINDOW: u32 = regs::ctrl_window.mask();
/// `ctrl` bit 2: the first timeout warns instead of resetting.
pub const CTRL_WARN: u32 = regs::ctrl_warn.mask();
/// `ctrl` bit 3: `ctrl`, `load` and `sill` are read only from now on.
pub const CTRL_LOCK: u32 = regs::ctrl_lock.mask();

/// `status` bit 0: the warning timeout has happened.
pub const STATUS_WARNED: u32 = regs::status_warned.mask();
/// `status` bit 1: the watchdog asked for a reset.
pub const STATUS_FAILED: u32 = regs::status_failed.mask();

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
        bus: LitePort<32, 32, 4>,
        (rst_req, irq): (Out<Bit>, Out<Bit>),
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
            // Which word a write goes to, one bit a word in the map's
            // order: ctrl, load, count, feed, status, sill.
            let we = regs_we(wgo, wsel);
            let enabled = ctrl.bit(0);
            let window = ctrl.bit(1);
            let warn = ctrl.bit(2);
            let locked = ctrl.bit(3);
            let warned = status.bit(0);
            // A write to `feed`, and whether it was a refresh or a
            // failure. Too early counts as a failure only in window
            // mode; a wrong key counts in either.
            let feeding = we.bit(3);
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
            let word = regs_read(
                rsel,
                regs_ctrl_pack(
                    ctrl.bit(0),
                    ctrl.bit(1),
                    ctrl.bit(2),
                    ctrl.bit(3),
                ),
                regs_load_pack(load),
                regs_count_pack(count),
                U::<32>::from(0u8),
                regs_status_pack(status.bit(0), status.bit(1)),
                regs_sill_pack(sill),
            );
            with!(self <= {
                // The lock bit sets and does not clear, so that a
                // program cannot take back what it locked.
                writable & we.bit(0) ? ctrl: written.slice::<0, 4>()
                    | (ctrl & U::<4>::from(8u8)),
                writable & we.bit(1) ? load: regs_load_cycles(written),
                writable & we.bit(5) ? sill: regs_sill_cycles(written),
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
                        we.bit(4),
                        !written.slice::<0, 2>(),
                        U::<2>::from(3u8),
                    ),
                failing ? hold: U::<4>::from(8u8),
                holding & !failing ? hold: hold - 1,
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
