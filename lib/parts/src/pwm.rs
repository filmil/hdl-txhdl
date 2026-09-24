// SPDX-License-Identifier: Apache-2.0
//! Pulse width modulation: four outputs that are high for a chosen
//! part of every period.
//!
//! A digital pin is on or off, and much of what a board drives wants
//! something in between: an LED at a third of its brightness, a fan at
//! half its speed, a servo told where to stand. Switching fast enough
//! that the load cannot follow gives the in-between, and what the load
//! sees is the fraction of each period the pin was high.
//!
//! | Offset | Name | What it is |
//! |---|---|---|
//! | `0x00` | `ctrl` | enable, centre, and a polarity bit per channel |
//! | `0x04` | `period` | the period, in cycles |
//! | `0x08` | `duty0` | channel 0: cycles high per period |
//! | `0x0c` | `duty1` | channel 1 |
//! | `0x10` | `duty2` | channel 2 |
//! | `0x14` | `duty3` | channel 3 |
//!
//! A duty written takes effect at the end of the period rather than at
//! once. A period that changed under a counter halfway through would
//! make one pulse of neither the old width nor the new, which is a
//! runt: a load that averages sees a step it was never told about, and
//! a servo reads it as a command. So a write goes to a shadow, and
//! every shadow is loaded together when the counter wraps.
//!
//! Two shapes of pulse, chosen by `ctrl`. Edge aligned counts up and
//! wraps, so every pulse starts at the same moment and the ends move
//! as the duty does. Centre aligned counts up and then back down, so a
//! pulse is centred in its period and both edges move; a bank of
//! outputs driven that way does not switch all at once, which is what
//! a power supply prefers.
//!
//! The ends are exact. A duty of zero is a pin that never goes high,
//! and a duty of the period or more is a pin that never comes down;
//! neither is a pulse one cycle wide.
use txhdl::comp::{mux, Clock, DefaultClock, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

/// `ctrl` bit 0: the counter runs.
pub const CTRL_ENABLE: u32 = 1;
/// `ctrl` bit 1: count up and down rather than up and wrap.
pub const CTRL_CENTRE: u32 = 2;
/// `ctrl` bits 4 to 7: channel `i` is high where it would be low.
pub const fn polarity(i: u32) -> u32 {
    1 << (4 + i)
}

/// The offset of channel `i`'s duty.
pub const fn duty(i: u32) -> u32 {
    8 + 4 * i
}

// begin{state}
/// Four pulse width modulated outputs on AXI-Lite.
#[derive(Trace, Default)]
pub struct Pwm {
    /// Enable, centre, and the four polarity bits.
    pub ctrl: Reg<U<8>>,
    /// The period in cycles, as the counter uses it.
    pub period: Reg<U<16>>,
    /// The period a program wrote, loaded at the next wrap.
    pub period_next: Reg<U<16>>,
    /// The four duties, as the counter uses them.
    pub duty: Reg<U<64>>,
    /// The four duties a program wrote, loaded at the next wrap.
    pub duty_next: Reg<U<64>>,
    /// Where in the period the counter is.
    pub count: Reg<U<16>>,
    /// Counting down, which only happens when centred.
    pub down: Reg<Bit>,
}
// end{state}

// begin{run}
#[lower]
impl Unit for Pwm {
    async fn run(&mut self, bus: LitePort<32, 32, 4>, pins: Out<U<4>>) {
        loop {
            DefaultClock::rising().await;
            let ctrl = self.ctrl.get();
            let period = self.period.get();
            let count = self.count.get();
            let duties = self.duty.get();
            let shadow = self.duty_next.get();
            let down = self.down.get();
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
            let written = wh.data.slice::<0, 16>();
            let enabled = ctrl.bit(0);
            let centred = ctrl.bit(1);
            // The counter. Up to the last cycle of the period and then
            // back to zero, or, centred, up to it and down again; a
            // period of zero holds the counter at zero, since a period
            // shorter than a cycle is no period at all.
            let last = (count + 1) >= period;
            let at_top = enabled & last & !down;
            let at_foot = enabled & centred & down & (count == 0);
            // The period ends where the counter returns to zero, which
            // is the top of an edge aligned period and the foot of a
            // centred one. Every shadow is loaded there, together.
            let wrap = mux(centred, at_foot, at_top);
            let up = enabled & !down & !at_top;
            let stepping = enabled & (period != 0);
            // The four duties, each against the counter. A duty of
            // zero never wins the comparison and a duty of the period
            // or more always does, so the ends need no special case.
            let d0 = duties.slice::<0, 16>();
            let d1 = duties.slice::<16, 16>();
            let d2 = duties.slice::<32, 16>();
            let d3 = duties.slice::<48, 16>();
            let h0 = Bit::from(count < d0) ^ ctrl.bit(4);
            let h1 = Bit::from(count < d1) ^ ctrl.bit(5);
            let h2 = Bit::from(count < d2) ^ ctrl.bit(6);
            let h3 = Bit::from(count < d3) ^ ctrl.bit(7);
            let lines = h3
                .zext::<1>()
                .concat::<1, 2>(h2.zext::<1>())
                .concat::<1, 3>(h1.zext::<1>())
                .concat::<1, 4>(h0.zext::<1>());
            let word = select!(rsel.raw() => {
                0 => ctrl.zext::<32>(),
                1 => self.period_next.get().zext::<32>(),
                2 => shadow.slice::<0, 16>().zext::<32>(),
                3 => shadow.slice::<16, 16>().zext::<32>(),
                4 => shadow.slice::<32, 16>().zext::<32>(),
                5 => shadow.slice::<48, 16>().zext::<32>(),
                6 => count.zext::<32>(),
                _ => U::<32>::from(0u8),
            });
            with!(self <= {
                wgo & (wsel == 0) ? ctrl: wh.data.slice::<0, 8>(),
                wgo & (wsel == 1) ? period_next: written,
                wgo & (wsel == 2) ? duty_next: shadow.slice::<16, 48>()
                    .concat::<16, 64>(written),
                wgo & (wsel == 3) ? duty_next: shadow.slice::<32, 32>()
                    .concat::<16, 48>(written)
                    .concat::<16, 64>(shadow.slice::<0, 16>()),
                wgo & (wsel == 4) ? duty_next: shadow.slice::<48, 16>()
                    .concat::<16, 32>(written)
                    .concat::<32, 64>(shadow.slice::<0, 32>()),
                wgo & (wsel == 5) ? duty_next: written
                    .concat::<48, 64>(shadow.slice::<0, 48>()),
                // What the counter does with the cycle.
                stepping & up ? count: count + 1,
                stepping & at_top & centred ? down: Bit::One,
                stepping & at_top & !centred ? count: U::<16>::from(0u8),
                stepping & down & (count != 0) ? count: count - 1,
                stepping & at_foot ? down: Bit::Zero,
                // The shadows, loaded together at the end of a period.
                stepping & wrap ? period: self.period_next.get(),
                stepping & wrap ? duty: shadow,
                // A period written while the counter is stopped takes
                // effect at once, since no pulse is under way to spoil.
                !enabled ? period: self.period_next.get(),
                !enabled ? duty: shadow,
                !enabled ? count: U::<16>::from(0u8),
                !enabled ? down: Bit::Zero,
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
            pins.set(mux(enabled, lines, ctrl.slice::<4, 4>()));
        }
    }
}
// end{run}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LiteW};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, In, Running};

    type Host = LiteHost<32, 32, 4>;

    /// A write of one word, and the wait for its response.
    async fn write(h: &Host, addr: u32, data: u32) {
        let (aw, _, w, b, _) = h;
        aw.send(LiteAw {
            addr: U::from(addr),
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

    /// What a test holds: the link's host end and the four pins.
    struct Rig {
        host: Host,
        pins: In<U<4>>,
    }

    impl Rig {
        /// Channel `i` now.
        fn high(&self, i: u32) -> bool {
            self.pins.get().bit(i as usize).to_bool()
        }
    }

    /// Run `client` against one peripheral until it is done.
    fn run<F>(client: impl FnOnce(Rig) -> F)
    where
        F: std::future::Future<Output = ()>,
    {
        let link = axi_lite::<32, 32, 4>();
        let (pins_o, pins) = signal::<U<4>, DefaultClock>();
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let body = client(Rig {
            host: link.host,
            pins,
        });
        let mut pwm = Pwm::default();
        let mut sim = Running::new(join2(
            async move {
                body.await;
                *d.borrow_mut() = true;
            },
            pwm.run(link.per.into(), pins_o),
        ));
        for _ in 0..20000 {
            sim.cycle();
            if *done.borrow() {
                return;
            }
        }
        panic!("the client did not finish");
    }

    /// The cycles channel `i` is high over `n` cycles from now.
    async fn count_high(rig: &Rig, i: u32, n: usize) -> usize {
        let mut high = 0;
        for _ in 0..n {
            DefaultClock::rising().await;
            if rig.high(i) {
                high += 1;
            }
        }
        high
    }

    #[test]
    fn a_duty_is_the_cycles_high_in_a_period() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, 4, 10).await;
            write(h, duty(0), 3).await;
            write(h, duty(1), 7).await;
            write(h, 0, CTRL_ENABLE).await;
            // Four periods of ten cycles: three cycles high of every
            // ten on the first channel, seven on the second.
            assert_eq!(count_high(&rig, 0, 40).await, 12);
            assert_eq!(count_high(&rig, 1, 40).await, 28);
        });
    }

    #[test]
    fn the_ends_are_flat() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, 4, 8).await;
            write(h, duty(0), 0).await;
            write(h, duty(1), 8).await;
            write(h, duty(2), 100).await;
            write(h, 0, CTRL_ENABLE).await;
            // Zero never goes high; the period, or more, never comes
            // down. Neither is a pulse a cycle wide.
            assert_eq!(count_high(&rig, 0, 32).await, 0);
            assert_eq!(count_high(&rig, 1, 32).await, 32);
            assert_eq!(count_high(&rig, 2, 32).await, 32);
        });
    }

    #[test]
    fn polarity_turns_a_channel_over() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, 4, 10).await;
            write(h, duty(0), 3).await;
            write(h, duty(1), 3).await;
            write(h, 0, CTRL_ENABLE | polarity(1)).await;
            assert_eq!(count_high(&rig, 0, 30).await, 9);
            assert_eq!(count_high(&rig, 1, 30).await, 21);
        });
    }

    /// Every whole run of high cycles in `n` cycles from now: the
    /// first is dropped, since the window cuts it.
    async fn runs(rig: &Rig, i: u32, n: usize) -> Vec<usize> {
        let mut seen = Vec::new();
        let mut run = 0;
        for _ in 0..n {
            DefaultClock::rising().await;
            if rig.high(i) {
                run += 1;
            } else if run > 0 {
                seen.push(run);
                run = 0;
            }
        }
        if !seen.is_empty() {
            seen.remove(0);
        }
        seen
    }

    #[test]
    fn a_duty_written_mid_period_makes_no_runt() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, 4, 10).await;
            write(h, duty(0), 2).await;
            write(h, 0, CTRL_ENABLE).await;
            // The write lands wherever it lands in the period under
            // way, and that is the point: every whole pulse from then
            // on is two cycles wide or eight, and none is in between.
            write(h, duty(0), 8).await;
            let widths = runs(&rig, 0, 40).await;
            assert!(!widths.is_empty(), "no whole pulse in forty cycles");
            for w in &widths {
                assert!(*w == 2 || *w == 8, "a runt of {w} cycles: {widths:?}");
            }
            // And the last of them is the new width, since the change
            // has had a period to take effect.
            assert_eq!(*widths.last().unwrap(), 8);
        });
    }

    #[test]
    fn a_centred_pulse_is_one_run_a_period() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, 4, 8).await;
            write(h, duty(0), 4).await;
            write(h, 0, CTRL_ENABLE | CTRL_CENTRE).await;
            // Counting up and then down makes the period twice the
            // count, so half of every sixteen cycles is high, and the
            // high cycles are one run in the middle rather than two at
            // the ends.
            let widths = runs(&rig, 0, 48).await;
            assert!(widths.len() >= 2, "too few whole pulses: {widths:?}");
            for w in &widths {
                assert_eq!(*w, 8, "a centred pulse is as wide: {widths:?}");
            }
        });
    }

    #[test]
    fn a_stopped_counter_sits_at_the_polarity() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, 4, 10).await;
            write(h, duty(0), 5).await;
            write(h, duty(1), 5).await;
            write(h, 0, polarity(1)).await;
            assert_eq!(count_high(&rig, 0, 20).await, 0);
            assert_eq!(count_high(&rig, 1, 20).await, 20);
        });
    }
}
