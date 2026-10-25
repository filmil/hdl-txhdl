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
//! warn, lock, scale), `load` (the timeout, in ticks), `count` (what is
//! left of it, read only), `feed` (write `KEY` to say software is still
//! there), `status` (warned, failed; write ones to clear) and `sill`
//! (the window opens when `count` falls to this), at `0x00` to `0x14`.
//!
//! A tick is 2^`scale` cycles (#1884). `scale` resets to 0, a tick a
//! cycle, so a program that never writes it sees the watchdog it always
//! did; at 15 a tick is 32,768 cycles, and the longest timeout at
//! 100 MHz goes from 655 µs to about 21.5 s. `count`, `sill` and the
//! warning all count in ticks, and a refresh starts a whole tick.
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
use txhdl::{lower, select, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

// begin{map}
// The map: six words, three address bits above the byte bits selecting
// one, and the other two read zero (issues 499 and 670).
regmap! { regs (regs_read, regs_we), 3: [
    (0, ctrl, rw, "enable, window, warn, lock and scale", [
        (enable, 0, 1, rw, 0, "the watchdog counts"),
        (window, 1, 1, rw, 0, "a refresh above sill is a failure"),
        (warn, 2, 1, rw, 0, "the first timeout warns instead of resetting"),
        (lock, 3, 1, rw, 0, "set, ctrl, load and sill are read only"),
        (scale, 4, 4, rw, 0, "a tick is two to this many cycles"),
    ]),
    (1, load, rw, "the timeout a refresh loads", [
        (cycles, 0, 16, rw, 0, "the timeout, in ticks"),
    ]),
    (2, count, ro, "what is left of the timeout", [
        (cycles, 0, 16, ro, 0, "what is left, in ticks"),
    ]),
    (3, feed, wo, "written with KEY: software is still there"),
    (4, status, w1c, "warned and failed; write ones to clear", [
        (warned, 0, 1, w1c, 0, "the warning timeout has happened"),
        (failed, 1, 1, w1c, 0, "the watchdog asked for a reset"),
    ]),
    (5, sill, rw, "the window opens when count falls to this", [
        (cycles, 0, 16, rw, 0, "the sill, in ticks"),
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
/// `ctrl` bits 4 to 7: a tick is 2^`scale` cycles.
pub const CTRL_SCALE: u32 = regs::ctrl_scale.mask();
/// Where `scale` sits in `ctrl`.
pub const CTRL_SCALE_SHIFT: u32 = regs::ctrl_scale.shift;

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
    /// Enable, window, warn, lock and scale.
    pub ctrl: Reg<U<8>>,
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
    /// Cycles since the last tick (#1884).
    pub pre: Reg<U<16>>,
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
            let pre = self.pre.get();
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
            // A tick ends when `pre` reaches 2^`scale` - 1 (#1884).
            let scale = ctrl.slice::<4, 4>();
            let last = select!(scale.raw() => {
                0 => U::<16>::from(0u16),
                1 => U::<16>::from(1u16),
                2 => U::<16>::from(3u16),
                3 => U::<16>::from(7u16),
                4 => U::<16>::from(15u16),
                5 => U::<16>::from(31u16),
                6 => U::<16>::from(63u16),
                7 => U::<16>::from(127u16),
                8 => U::<16>::from(255u16),
                9 => U::<16>::from(511u16),
                10 => U::<16>::from(1023u16),
                11 => U::<16>::from(2047u16),
                12 => U::<16>::from(4095u16),
                13 => U::<16>::from(8191u16),
                14 => U::<16>::from(16383u16),
                15 => U::<16>::from(32767u16),
                _ => U::<16>::from(0u16),
            });
            let tick = pre == last;
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
                    scale,
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
                writable & we.bit(0) ? ctrl: written.slice::<0, 8>()
                    | (ctrl & U::<8>::from(8u8)),
                writable & we.bit(1) ? load: regs_load_cycles(written),
                writable & we.bit(5) ? sill: regs_sill_cycles(written),
                // A refresh, a warning and a failure all reload. The
                // failure reloads because the count is at zero when it
                // happens: a watchdog that did not reload would ask
                // for a reset on every cycle from then on, and hold
                // the machine down rather than restart it.
                refresh | warning | failing ? count: load,
                enabled & tick & !refresh & !warning & !failing
                    & (count != 0) ? count: count - 1,
                // A refresh, a warning and a failure start a whole tick;
                // otherwise `pre` counts while enabled, back to 0 at
                // each tick. A write to `ctrl` starts one too, so that
                // a smaller `scale` never finds `pre` past the end of
                // its tick, and enabling again never starts on a part
                // of one.
                enabled ? pre: mux(tick, U::<16>::from(0u16), pre + 1),
                refresh | warning | failing | (writable & we.bit(0))
                    ? pre: U::<16>::from(0u16),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi_lite::{axi_lite, LiteAr, LiteAw, LiteHost, LiteW};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, Running};

    type Host = LiteHost<32, 32, 4>;
    const KEY: usize = 0x5744_4f47;

    async fn write(h: &Host, at: u32, data: u32) {
        let (aw, _, w, b, _) = h;
        aw.send(LiteAw {
            addr: U::from(at),
            prot: U::from(0u8),
        });
        w.send(LiteW {
            data: U::from(data),
            strb: U::from(0xfu8),
        });
        loop {
            DefaultClock::rising().await;
            if b.recv().is_some() {
                return;
            }
        }
    }

    async fn read(h: &Host, at: u32) -> u32 {
        let (_, ar, _, _, r) = h;
        ar.send(LiteAr {
            addr: U::from(at),
            prot: U::from(0u8),
        });
        loop {
            DefaultClock::rising().await;
            if let Some(v) = r.recv() {
                return v.data.raw() as u32;
            }
        }
    }

    /// What a run saw: the cycle after `ctrl` is written in which the
    /// warning line first rose, and the reset request, if they did.
    #[derive(Default, Debug, Clone, Copy)]
    struct Seen {
        irq: Option<u32>,
        rst: Option<u32>,
    }

    /// The watchdog with `load` set and a first refresh, which loads the
    /// count, then `ctrl` written; then fed with the key at each cycle in
    /// `feeds`, counted from the `ctrl` write, for `cycles` cycles.
    fn run(load: u32, ctrl: u32, feeds: &[u32], cycles: u32) -> Seen {
        run_with(load, 0, ctrl, feeds, cycles)
    }

    /// `run`, with the window's `sill` set as well.
    fn run_with(
        load: u32,
        sill: u32,
        ctrl: u32,
        feeds: &[u32],
        cycles: u32,
    ) -> Seen {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let host = link.host;
        let (rst_req, rst_in) = signal::<Bit, DefaultClock>();
        let (irq, irq_in) = signal::<Bit, DefaultClock>();
        let seen: Rc<RefCell<Seen>> = Rc::default();
        let log = seen.clone();
        let feeds = feeds.to_vec();
        let client = async move {
            write(&host, regs::load, load).await;
            write(&host, regs::sill, sill).await;
            write(&host, regs::feed, KEY as u32).await;
            write(&host, regs::ctrl, ctrl).await;
            for t in 0..cycles {
                {
                    let mut s = log.borrow_mut();
                    if s.irq.is_none() && irq_in.get().to_bool() {
                        s.irq = Some(t);
                    }
                    if s.rst.is_none() && rst_in.get().to_bool() {
                        s.rst = Some(t);
                    }
                }
                if feeds.contains(&t) {
                    write(&host, regs::feed, KEY as u32).await;
                } else {
                    DefaultClock::rising().await;
                }
            }
        };
        let mut dog = Wdog::<KEY>::default();
        let mut sim = Running::new(join2(dog.run(bus, (rst_req, irq)), client));
        for _ in 0..(cycles * 4 + 200) {
            sim.cycle();
        }
        let s = *seen.borrow();
        s
    }

    fn scale(k: u32) -> u32 {
        k << CTRL_SCALE_SHIFT
    }

    /// At `scale` 0 a tick is a cycle, so the timeout is `load` cycles,
    /// as it was before the prescaler (#1884); at `scale` 3 a tick is
    /// eight, and the same `load` lasts eight times as long, give or
    /// take a tick.
    #[test]
    fn a_tick_is_two_to_the_scale_cycles() {
        let at0 = run(40, CTRL_ENABLE, &[], 200).rst.expect("bites");
        let at3 = run(40, CTRL_ENABLE | scale(3), &[], 600)
            .rst
            .expect("bites");
        assert!((39..=42).contains(&at0), "about 40 cycles: {at0}");
        assert!(at3.abs_diff(8 * at0) <= 8, "eight times: {at3} vs {at0}");
    }

    /// The warning comes after `load` ticks and the reset after `load`
    /// more, both in ticks of 2^`scale` cycles.
    #[test]
    fn the_warning_and_the_reset_count_in_ticks() {
        let s = run(20, CTRL_ENABLE | CTRL_WARN | scale(2), &[], 400);
        let (irq, rst) = (s.irq.expect("warns"), s.rst.expect("bites"));
        assert!(irq.abs_diff(80) <= 4, "the warning at 20 ticks: {irq}");
        assert!(rst.abs_diff(160) <= 8, "the reset at 40 ticks: {rst}");
    }

    /// Refreshes inside each timeout keep the reset away at any scale:
    /// at `scale` 3 and `load` 10, eighty cycles, fed every sixty.
    #[test]
    fn a_refresh_inside_the_timeout_keeps_the_reset_away() {
        let feeds: Vec<u32> = (1..10).map(|k| 60 * k).collect();
        let s = run(10, CTRL_ENABLE | scale(3), &feeds, 600);
        assert_eq!(s.rst, None, "fed in time, no reset");
    }

    /// In window mode the sill counts in ticks too: at `scale` 2, `load`
    /// 20 and `sill` 10, a refresh ten cycles in has 18 ticks left and
    /// is early, and one sixty cycles in has 5 left and is in time.
    #[test]
    fn the_window_counts_in_ticks() {
        let ctrl = CTRL_ENABLE | CTRL_WINDOW | scale(2);
        let early = run_with(20, 10, ctrl, &[10], 200);
        let late = run_with(20, 10, ctrl, &[60], 200);
        let at = early.rst.expect("early is a failure");
        assert!(at < 40, "failed at the refresh: {at}");
        let at = late.rst.expect("bites a timeout later");
        assert!(at > 120, "in time, so a whole timeout more: {at}");
    }

    /// A smaller `scale` written in the middle of a long tick starts a
    /// new tick at once (agy on PR 1917): from `scale` 15 down to 0, a
    /// hundred cycles into a tick of 32,768, the reset comes `load`
    /// cycles after the write and not when `pre` wraps.
    #[test]
    fn a_smaller_scale_starts_a_new_tick() {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let host = link.host;
        let (rst_req, rst_in) = signal::<Bit, DefaultClock>();
        let (irq, _) = signal::<Bit, DefaultClock>();
        let seen: Rc<RefCell<Option<u32>>> = Rc::default();
        let log = seen.clone();
        let client = async move {
            write(&host, regs::load, 20).await;
            write(&host, regs::feed, KEY as u32).await;
            write(&host, regs::ctrl, CTRL_ENABLE | scale(15)).await;
            for _ in 0..100 {
                DefaultClock::rising().await;
            }
            write(&host, regs::ctrl, CTRL_ENABLE).await;
            for t in 0..200 {
                if rst_in.get().to_bool() {
                    *log.borrow_mut() = Some(t);
                    return;
                }
                DefaultClock::rising().await;
            }
        };
        let mut dog = Wdog::<KEY>::default();
        let mut sim = Running::new(join2(dog.run(bus, (rst_req, irq)), client));
        for _ in 0..1000 {
            sim.cycle();
        }
        let at = seen.borrow().expect("bites at the new scale");
        assert!((18..=22).contains(&at), "about 20 cycles: {at}");
    }

    /// `lock` freezes `scale` with the rest of `ctrl`: a locked
    /// watchdog cannot be slowed.
    #[test]
    fn lock_freezes_the_scale() {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let host = link.host;
        let (rst_req, _) = signal::<Bit, DefaultClock>();
        let (irq, _) = signal::<Bit, DefaultClock>();
        let got: Rc<RefCell<Vec<u32>>> = Rc::default();
        let log = got.clone();
        let client = async move {
            write(&host, regs::ctrl, scale(2)).await;
            let v = read(&host, regs::ctrl).await;
            log.borrow_mut().push(v);
            write(&host, regs::ctrl, scale(2) | CTRL_LOCK).await;
            write(&host, regs::ctrl, scale(9) | CTRL_LOCK).await;
            let v = read(&host, regs::ctrl).await;
            log.borrow_mut().push(v);
        };
        let mut dog = Wdog::<KEY>::default();
        let mut sim = Running::new(join2(dog.run(bus, (rst_req, irq)), client));
        for _ in 0..200 {
            sim.cycle();
        }
        let got = got.borrow();
        assert_eq!(got[0], scale(2), "scale reads back");
        assert_eq!(got[1], scale(2) | CTRL_LOCK, "locked, it stays 2");
    }
}
