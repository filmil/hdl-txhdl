// SPDX-License-Identifier: Apache-2.0
//! The board's configuration flash, reached from user logic after the
//! bitstream has loaded (issue 312).
//!
//! On a 7-series part the flash's clock is the dedicated CCLK pin, and
//! user logic reaches it only through the `STARTUPE2` primitive's
//! `USRCCLKO`. Two things follow, both from UG470, the configuration
//! user guide.
//!
//! * The primitive says when configuration is over, on `EOS`, the end
//!   of startup. Before that the flash belongs to the configuration
//!   logic, which may still be reading it.
//! * The first three clocks on `USRCCLKO` after `EOS` never reach the
//!   pin: they are spent moving CCLK over from the configuration logic
//!   to the user. A command whose clocks start there reaches the chip
//!   three bits short.
//!
//! So three pieces. [`Startup`] is the primitive, as a foreign module
//! with a model that does both of those things. [`FlashPins`] stands
//! between the chip and the two parts that talk to it, the master in
//! `spi` and the window in `flashwin`: it waits for `EOS`, spends four
//! clocks with the chip not selected, holds the window in reset until
//! then, and afterwards gives the wires to whichever of the two
//! selects the chip first. [`CfgFlash`] is the two joined, as a board
//! instantiates them.
//!
//! `FlashPins` can refuse the command that lets a flash be changed.
//! Every command that programs or erases a flash, or writes its
//! registers, is ignored by the chip unless write enable, `0x06`, came
//! first, and the chip forgets write enable at power up and after every
//! such command. So one command is the door to all of them, and with
//! `GUARD` set, the default on a board, `FlashPins` raises the select
//! before the last bit of a `0x06` from the master reaches the chip: the
//! chip discards a command shorter than eight bits, and the master's
//! later program or erase is then ignored by the chip itself. The
//! window only reads. Nothing the core runs can then write the flash
//! that holds the bitstream; a cable and Vivado still can.
//!
//! Every output here is a register, so the path from a master to the
//! chip is a cycle longer than a wire, and the clock a cycle longer
//! again through `Startup`. The master's clock must turn slowly enough
//! for that: a half of a bit of at least four cycles, `DIV` of three,
//! for the chip's answer to be on `miso` when the master takes it.
//! The chip's `miso` goes to both masters directly, not through here.
//!
//! Only mode 0 is carried: the clock idles low. That is the mode every
//! flash reads in, and the window's only mode; a master in mode 3
//! would raise the clock as it took the wires.
use txhdl::comp::trace::Kind;
use txhdl::comp::{join2, signal, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::netlist::{foreign, Lower, Lowered};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{startup}
/// The cycles the model takes to reach the end of startup. A real part
/// says it when configuration is done; the model needs a number, and a
/// few cycles is enough to show what waits for it.
pub const EOS_AFTER: u32 = 8;

/// The clocks on `USRCCLKO` after `EOS` that never reach the pin.
pub const SWALLOWED: u32 = 3;

/// `STARTUPE2`, as the foreign module `startup` in
/// `lib/parts/hdl/startup.v`, which instantiates the primitive when
/// synthesised and is the model below when simulated.
///
/// `usrcclko` is the clock user logic drives the flash with. `eos`
/// says configuration is over. `cclk` is what reaches the flash's
/// clock pin: nothing before `eos`, nothing for the first three rises
/// after it, and `usrcclko` from then on. A real part has no such
/// output, since the pin is not a net user logic can see; synthesised,
/// it is held low and nothing reads it, and in a simulation it is what
/// a model of the chip is clocked by.
#[derive(Trace, Default)]
pub struct Startup {
    /// Cycles since the start, up to `EOS_AFTER`.
    pub count: Reg<U<4>>,
    /// Configuration is over.
    pub eos: Reg<Bit>,
    /// `usrcclko` a cycle ago, to see it rise.
    pub prev: Reg<Bit>,
    /// Rises of `usrcclko` since `eos`, up to `SWALLOWED`.
    pub seen: Reg<U<2>>,
    /// The pin.
    pub cclk: Reg<Bit>,
}

impl Lower for Startup {
    fn lowered_as(name: &str) -> Lowered {
        foreign(
            name,
            "startup",
            &[
                ("usrcclko", Kind::In, 1),
                ("eos", Kind::Out, 1),
                ("cclk", Kind::Out, 1),
            ],
            &[("EOS_AFTER", EOS_AFTER as i128)],
            &[("clk", DefaultClock::NAME)],
        )
    }
}

impl Unit<In<Bit>, (Out<Bit>, Out<Bit>)> for Startup {
    async fn run(
        &mut self,
        usrcclko: In<Bit>,
        (eos, cclk): (Out<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let u = usrcclko.get();
            let count = self.count.get();
            let on = self.eos.get();
            // `CfgFlash` runs the pins before this, so `usrcclko` is
            // the pins' register as this step drives it, and the pins
            // read `eos` a step after it is driven here. So `eos` is
            // driven with what its register becomes at this edge, which
            // the pins then read when the module's `assign` would show
            // it: a wire promises nothing about which process ran first.
            let at_end = count == U::<4>::from(EOS_AFTER);
            eos.set(on | Bit::from(at_end));
            cclk.set(self.cclk.get());
            let seen = self.seen.get();
            let passed = seen == U::<2>::from(SWALLOWED);
            if !at_end {
                self.count.set(count + 1);
            } else {
                self.eos.set(Bit::One);
            }
            self.prev.set(u);
            if (on & u & !self.prev.get() & !Bit::from(passed)).to_bool() {
                self.seen.set(seen + 1);
            }
            self.cclk.set(u & on & Bit::from(passed));
        }
    }
}
// end{startup}

// begin{pins}
/// The four clocks spent after `EOS`, as eight turns of the clock.
const TURNS: u32 = 8;

/// The first seven bits of write enable, `0x06`, top bit first.
const WREN_TOP7: u32 = 0x06 >> 1;

/// Who has the wires: nobody, the master, or the window.
const NOBODY: u32 = 0;
const MASTER: u32 = 1;
const WINDOW: u32 = 2;

/// Between the chip and the two parts that reach it. `GUARD` of one
/// refuses write enable from the master, zero passes it; see the
/// module's comment for why that one command.
#[derive(Trace)]
pub struct FlashPins<const GUARD: usize> {
    /// Where it is: waiting for `EOS`, spending the clocks, or ready.
    pub phase: Reg<U<2>>,
    /// Turns of the clock spent so far.
    pub turns: Reg<U<4>>,
    /// Who has the wires.
    pub owner: Reg<U<2>>,
    /// The master's clock a cycle ago, to see it rise.
    pub mprev: Reg<Bit>,
    /// The master's command so far, a bit a rise of its clock.
    pub cmd: Reg<U<7>>,
    /// How many bits of it, up to eight.
    pub bits: Reg<U<4>>,
    /// The master's command was write enable, and the chip was let go
    /// before its last bit; held until the master lets go too.
    pub cut: Reg<Bit>,
    /// The clock to the pin, through the primitive.
    pub pclk: Reg<Bit>,
    /// The data to the chip.
    pub pmosi: Reg<Bit>,
    /// The chip held.
    pub sel: Reg<Bit>,
    /// The select pin, from a register of its own: written whenever
    /// `sel` is, with `sel`'s new value inverted, so it is `!sel` at
    /// every cycle. The pin's net is then register to pad with no
    /// other load, and `sel`'s own feedback stays off it, which the
    /// router's hold detour for the pin had stretched to 22 ns (issue
    /// 893). It starts and resets at 1, the chip let go.
    pub pcs_n: Reg<Bit>,
}

impl<const GUARD: usize> Default for FlashPins<GUARD> {
    fn default() -> Self {
        FlashPins {
            phase: Reg::default(),
            turns: Reg::default(),
            owner: Reg::default(),
            mprev: Reg::default(),
            cmd: Reg::default(),
            bits: Reg::default(),
            cut: Reg::default(),
            pclk: Reg::default(),
            pmosi: Reg::default(),
            sel: Reg::default(),
            pcs_n: Reg::new(Bit::One),
        }
    }
}

#[lower]
impl<const GUARD: usize> Unit for FlashPins<GUARD> {
    async fn run(
        &mut self,
        (eos, m_sclk, m_mosi, m_cs_n, w_sclk, w_mosi, w_cs_n): (
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
        ),
        (usrcclko, mosi, cs_n, w_rst, refused): (
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            // The pins first, from the registers alone, before any input
            // is read: a wire promises nothing about which process ran
            // first, and `Startup` reads the clock as this drives it.
            usrcclko.set(self.pclk.get());
            mosi.set(self.pmosi.get());
            cs_n.set(self.pcs_n.get());
            w_rst.set(self.phase.get() != U::<2>::from(2u8));
            refused.set(self.cut.get());
            let phase = self.phase.get();
            let turns = self.turns.get();
            let owner = self.owner.get();
            let cmd = self.cmd.get();
            let bits = self.bits.get();
            let cut = self.cut.get();
            let pclk = self.pclk.get();
            let waiting = phase == U::<2>::from(0u8);
            let spending = phase == U::<2>::from(1u8);
            let ready = phase == U::<2>::from(2u8);
            let spent = turns == U::<4>::from(TURNS - 1);
            // Who has the wires next: the window before the master when
            // both ask in one cycle, and whoever has them until it lets
            // go of its select.
            let nobody = owner == U::<2>::from(NOBODY);
            let master = owner == U::<2>::from(MASTER);
            let window = owner == U::<2>::from(WINDOW);
            let m_on = !m_cs_n.get();
            let w_on = !w_cs_n.get();
            let take_w = ready & nobody & w_on;
            let take_m = ready & nobody & !w_on & m_on;
            let free = (master & !m_on) | (window & !w_on);
            // The guard: the master's command a bit a rise of its
            // clock, and the chip let go on the seventh rise if the
            // seven are the start of write enable.
            let rise = master & m_on & m_sclk.get() & !self.mprev.get();
            let counting = rise & (bits != U::<4>::from(8u8));
            let cmd1 = (cmd << 1u32) | m_mosi.get().zext::<7>();
            let trip = Bit::from(GUARD != 0)
                & counting
                & (bits == U::<4>::from(6u8))
                & (cmd1 == U::<7>::from(WREN_TOP7));
            let stop = cut | trip;
            with!(self <= {
                waiting & eos.get() ? phase: U::<2>::from(1u8),
                spending ? {
                    turns: turns + 1,
                    pclk: !pclk,
                },
                spending & spent ? phase: U::<2>::from(2u8),
                take_w ? owner: U::<2>::from(WINDOW),
                take_m ? owner: U::<2>::from(MASTER),
                free ? {
                    owner: U::<2>::from(NOBODY),
                    cmd: U::<7>::from(0u8),
                    bits: U::<4>::from(0u8),
                    cut: Bit::Zero,
                },
                mprev: m_sclk.get(),
                counting ? {
                    cmd: cmd1,
                    bits: bits + 1,
                },
                trip ? cut: Bit::One,
                ready ? {
                    pclk: (master & m_sclk.get()) | (window & w_sclk.get()),
                    pmosi: (master & m_mosi.get()) | (window & w_mosi.get()),
                    sel: (master & m_on & !stop) | (window & w_on),
                    pcs_n: !((master & m_on & !stop) | (window & w_on)),
                },
            });
        }
    }
}
// end{pins}

// begin{cfgflash}
/// The primitive and the pins, as a board instantiates them. The
/// inputs are the master's three lines and the window's; the outputs
/// are the chip's select and data, the clock pin as the model shows
/// it, the window's reset and whether a command was refused.
#[derive(Trace, Default)]
pub struct CfgFlash<const GUARD: usize> {
    /// The primitive. Not `startup`, the module's own name, which
    /// the VHDL netlist cannot label an instance with (issue 816).
    pub prim: Startup,
    /// The pins, between the chip and the two that reach it.
    pub pins: FlashPins<GUARD>,
}

#[lower]
impl<const GUARD: usize> Unit for CfgFlash<GUARD> {
    async fn run(
        &mut self,
        (m_sclk, m_mosi, m_cs_n, w_sclk, w_mosi, w_cs_n): (
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
        ),
        (cclk, mosi, cs_n, w_rst, refused): (
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        let (eos_o, eos_i) = signal::<Bit, DefaultClock>();
        let (uclk_o, uclk_i) = signal::<Bit, DefaultClock>();
        // The pins first: `Startup` reads the clock they drive this
        // step, and drives `eos` a step ahead for them.
        join2(
            self.pins.run(
                (eos_i, m_sclk, m_mosi, m_cs_n, w_sclk, w_mosi, w_cs_n),
                (uclk_o, mosi, cs_n, w_rst, refused),
            ),
            self.prim.run(uclk_i, (eos_o, cclk)),
        )
        .await;
    }
}
// end{cfgflash}
