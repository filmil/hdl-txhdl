// SPDX-License-Identifier: Apache-2.0
//! The debug transport module: the RISC-V debug specification's
//! transport over JTAG, so that a stock OpenOCD, and gdb through it,
//! reaches a debug module from the board's own cable (issue 154,
//! step three).
//!
//! The FPGA's JTAG port belongs to the device, so a design reaches it
//! through `BSCANE2`, one of the user scan chains of the device's TAP.
//! OpenOCD speaks to a transport there with its BSCAN tunnel, which it
//! drives as `riscv use_bscan_tunnel 5`: the device's instruction
//! register is set to `USER4`, and every scan the transport's own TAP
//! would have had is carried inside one data scan of that chain, as
//! OpenOCD v0.12.0's `riscv.c` frames it (the nested TAP, its default):
//!
//! | scan        | first bit shifted to last                         |
//! |-------------|---------------------------------------------------|
//! | instruction | `0`, the width 5 (7 bits), the register (5), `000` |
//! | data        | `1`, the width W (7 bits), the data (W + 1), `000` |
//!
//! The payload is one bit longer than the register, because what comes
//! back is late by a clock: the bit that leaves on edge `j` of the
//! payload is the register's bit `j`, and the device's TAP presents it
//! on the edge after, so OpenOCD reads the register from the second
//! bit on. This unit shifts the first W bits of the payload into the
//! register and lets the last one go by.
//!
//! The registers are the specification's, at its numbers:
//!
//! | IR     | register | width | what                                   |
//! |--------|----------|-------|----------------------------------------|
//! | `0x01` | `idcode` | 32    | `IDCODE`, the transport's own          |
//! | `0x10` | `dtmcs`  | 32    | version 1, `abits` 7, `dmistat`, reset |
//! | `0x11` | `dmi`    | 41    | address 7, data 32, op 2               |
//! | other  | bypass   | 1     | zero                                   |
//!
//! A `dmi` scan that asks for a read (op 1) or a write (op 2) sends a
//! request on `req`, the scan's own 41 bits; the answer comes back on
//! `ans`, the data above a two-bit status, 0 for done and 2 for
//! failed. Until it has, the access is in flight, and a `dmi` scan
//! captured meanwhile says busy, op 3, which sticks, as the
//! specification has it, until OpenOCD writes `dmireset` in `dtmcs`.
//! OpenOCD answers busy by idling longer before its next scan.
//!
//! Everything here runs on [`Tck`], the cable's clock, which `BSCANE2`
//! hands the fabric; the request and the answer cross to the system's
//! clock beside the unit, so the unit itself has one clock.
use txhdl::comp::{mux, Clock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

/// The cable's clock, as `BSCANE2` gives it. Its period in the
/// simulation is five of the default clock's, which is slower than
/// the system, as a cable always is.
pub struct Tck;

impl Clock for Tck {
    const NAME: &'static str = "tck";
    const PERIOD: u64 = 10;
}

/// The instruction register's codes, as the specification has them.
/// `idcode`: the transport's identity, selected after reset.
pub const IR_IDCODE: u32 = 0x01;
/// `dtmcs`: the transport's control and status.
pub const IR_DTMCS: u32 = 0x10;
/// `dmi`: an access to the debug module's registers.
pub const IR_DMI: u32 = 0x11;

/// The transport's identity: version 0, part `0x0154`, and bit 0 set as
/// JTAG requires, with no manufacturer of record.
pub const IDCODE: u32 = 0x0015_4001;

/// Address bits of `dmi`: enough for the debug module's `haltsum0` at
/// `0x40` and the system bus registers at `0x38` to `0x3c`.
pub const ABITS: u32 = 7;
/// The width of `dmi`: the address, a word, and the op.
pub const DMI_WIDTH: u32 = ABITS + 34;

/// `dtmcs` as it reads with no error standing: version 1 (the 0.13
/// specification), `abits`, and an idle hint of one cycle.
pub const DTMCS: u32 = 1 | (ABITS << 4) | (1 << 12);

/// The ops of a `dmi` scan: what it asks in, how the last access went
/// out.
/// Asked: nothing.
pub const OP_NOP: u32 = 0;
/// Asked: a read of the address.
pub const OP_READ: u32 = 1;
/// Asked: a write of the data to the address.
pub const OP_WRITE: u32 = 2;
/// Told: the last access failed.
pub const OP_FAILED: u32 = 2;
/// Told: an access was still in flight when a scan came.
pub const OP_BUSY: u32 = 3;

// begin{dtm}
/// The transport's state.
#[derive(Trace, Default)]
pub struct Dtm {
    /// The instruction register: which register a data scan reaches.
    pub ir: Reg<U<5>, Tck>,
    /// The shift register, wide enough for `dmi`: a scan's bits enter
    /// at the top and leave at the bottom.
    pub sr: Reg<U<41>, Tck>,
    /// What the device's TAP presents on its TDO: the bit that left.
    pub out: Reg<Bit, Tck>,
    /// Where the scan is: 0 the first bit, 1 the width, 2 the payload,
    /// 3 the three bits after it.
    pub phase: Reg<U<2>, Tck>,
    /// Whether this scan is a data scan, from its first bit.
    pub isdr: Reg<Bit, Tck>,
    /// The payload's width, from the seven bits after the first.
    pub len: Reg<U<7>, Tck>,
    /// Bits of the width, then of the payload, so far.
    pub cnt: Reg<U<7>, Tck>,
    /// An access is in flight.
    pub inflight: Reg<Bit, Tck>,
    /// `dmistat`: how the accesses went since the last reset, sticky.
    pub sticky: Reg<U<2>, Tck>,
    /// The address of the last access; it and the word are what a `dmi`
    /// capture shows.
    pub addr: Reg<U<7>, Tck>,
    /// The word of the last access.
    pub data: Reg<U<32>, Tck>,
}
// end{dtm}

// begin{run}
#[lower]
impl Unit for Dtm {
    async fn run(
        &mut self,
        (sel, shift, capture, update, tdi, reset, ans): (
            In<Bit, Tck>,
            In<Bit, Tck>,
            In<Bit, Tck>,
            In<Bit, Tck>,
            In<Bit, Tck>,
            In<Bit, Tck>,
            Rx<U<34>, Tck>,
        ),
        (tdo, req): (Out<Bit, Tck>, Tx<U<41>, Tck>),
    ) {
        loop {
            Tck::rising().await;
            let sel = sel.get();
            let ir = self.ir.get();
            let sr = self.sr.get();
            let phase = self.phase.get();
            let isdr = self.isdr.get();
            let len = self.len.get();
            let cnt = self.cnt.get();
            let inflight = self.inflight.get();
            let sticky = self.sticky.get();
            let bit = tdi.get();
            let shifting = sel & shift.get();
            // The scan's frame: the first bit, then seven of width.
            let first = shifting & (phase == 0);
            let width = shifting & (phase == 1);
            let len1 = (len >> 1u32) | (bit.zext::<7>() << 6u32);
            let width_done = width & (cnt == 6);
            // What the register holds as the payload starts: the
            // instruction register's capture, or the selected data
            // register, in the low bits so that bit 0 leaves first.
            let dtmcs = U::<41>::from(DTMCS) | (sticky.zext::<41>() << 10u32);
            let dmi = (self.addr.get().zext::<41>() << 34u32)
                | (self.data.get().zext::<41>() << 2u32)
                | mux(inflight, U::<41>::from(OP_BUSY), sticky.zext::<41>());
            let dr = select!(ir.raw() => {
                0x01 => U::<41>::from(IDCODE),
                0x10 => dtmcs,
                0x11 => dmi,
                _ => U::<41>::from(0u8),
            });
            let loaded = mux(isdr, dr, U::<41>::from(1u8));
            // The payload: the first `len` bits in at the top, the
            // last one let go by; the bit at the bottom leaves each
            // edge.
            let payload = shifting & (phase == 2);
            let into = payload & (cnt < len);
            let shifted = (sr >> 1u32) | (bit.zext::<41>() << 40u32);
            let payload_done = payload & (cnt == len);
            // The update, after a whole scan: the instruction, or the
            // data register the instruction selects.
            let done = sel & update.get() & (phase == 3);
            let new_ir = sr.slice::<36, 5>();
            let to_dtmcs = done & isdr & (ir == IR_DTMCS);
            let dmireset = to_dtmcs & sr.bit(25);
            let hardreset = to_dtmcs & sr.bit(26);
            let to_dmi = done & isdr & (ir == IR_DMI);
            let op = sr.slice::<0, 2>();
            let asks =
                to_dmi & (sticky == 0) & ((op == OP_READ) | (op == OP_WRITE));
            let go = asks & !inflight & req.ready();
            let refused = asks & !go;
            // A `dmi` capture while an access is in flight says busy,
            // and the specification makes that stick.
            let seen_busy = width_done & isdr & (ir == IR_DMI) & inflight;
            // The answer to the access in flight.
            let answered = Bit::from(ans.peek().is_some());
            let _ = ans.recv_if(Bit::One);
            let back = ans.head();
            with!(self <= {
                sel & capture.get() ? { phase: U::<2>::from(0u8), cnt: U::<7>::from(0u8) },
                first ? { isdr: bit, phase: U::<2>::from(1u8) },
                width ? { len: len1, cnt: cnt + 1 },
                width_done ? {
                    phase: U::<2>::from(2u8),
                    cnt: U::<7>::from(0u8),
                    sr: loaded,
                },
                payload ? { cnt: cnt + 1, out: sr.bit(0) },
                into ? sr: shifted,
                payload_done ? phase: U::<2>::from(3u8),
                done & !isdr ? ir: new_ir,
                go ? { inflight: Bit::One, addr: sr.slice::<34, 7>() },
                go & (op == OP_WRITE) ? data: sr.slice::<2, 32>(),
                answered ? {
                    inflight: Bit::Zero,
                    data: back.slice::<2, 32>(),
                },
                answered & (back.slice::<0, 2>() == OP_FAILED) & (sticky == 0) ?
                    sticky: U::<2>::from(2u8),
                refused | seen_busy ? sticky: U::<2>::from(3u8),
                dmireset | hardreset ? sticky: U::<2>::from(0u8),
                hardreset ? inflight: Bit::Zero,
                reset.get() ? {
                    ir: U::<5>::from(IR_IDCODE),
                    phase: U::<2>::from(0u8),
                },
            });
            if go.to_bool() {
                req.send(sr);
            }
            tdo.set(self.out);
        }
    }
}
// end{run}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use txhdl::comp::{chan, join2, signal, Running};

    /// The simulation, whatever its future: one tick at a time.
    trait Sim {
        fn tick(&mut self);
    }

    impl<F: std::future::Future<Output = ()>> Sim for Running<F> {
        fn tick(&mut self) {
            self.step();
        }
    }

    /// The device's side of `BSCANE2`, as the tests drive it, and the
    /// transport's register that its TDO carries.
    struct Bscan {
        sel: Out<Bit, Tck>,
        shift: Out<Bit, Tck>,
        capture: Out<Bit, Tck>,
        update: Out<Bit, Tck>,
        tdi: Out<Bit, Tck>,
        reset: Out<Bit, Tck>,
        out: Reg<Bit, Tck>,
    }

    /// One edge of the cable's clock, with these levels on the pins.
    fn edge(sim: &mut dyn Sim, b: &Bscan, levels: [bool; 5]) {
        let [capture, shift, update, tdi, reset] = levels;
        b.sel.set(Bit::One);
        b.capture.set(Bit::from(capture));
        b.shift.set(Bit::from(shift));
        b.update.set(Bit::from(update));
        b.tdi.set(Bit::from(tdi));
        b.reset.set(Bit::from(reset));
        for _ in 0..Tck::PERIOD {
            sim.tick();
        }
    }

    /// One data scan of `USER4`: capture, a shift edge per bit, update.
    /// What comes back is what the host reads: on each edge the bit
    /// the transport let go of on the edge before, since the device's
    /// TAP presents it a clock late.
    fn scan(sim: &mut dyn Sim, b: &Bscan, bits: &[bool]) -> Vec<bool> {
        edge(sim, b, [true, false, false, false, false]);
        let mut got = vec![b.out.get() == Bit::One];
        for &bit in bits {
            edge(sim, b, [false, true, false, bit, false]);
            got.push(b.out.get() == Bit::One);
        }
        got.truncate(bits.len());
        edge(sim, b, [false, false, true, false, false]);
        got
    }

    fn bits(v: u64, n: u32) -> Vec<bool> {
        (0..n).map(|i| (v >> i) & 1 == 1).collect()
    }

    fn value(bits: &[bool]) -> u64 {
        bits.iter().rev().fold(0, |a, &b| (a << 1) | b as u64)
    }

    /// OpenOCD's tunneled instruction scan, field for field from
    /// `riscv.c`: `0`, the width 5 in seven bits, the register, `000`.
    fn select(sim: &mut dyn Sim, b: &Bscan, ir: u32) {
        let mut v = vec![false];
        v.extend(bits(5, 7));
        v.extend(bits(ir as u64, 5));
        v.extend([false; 3]);
        scan(sim, b, &v);
    }

    /// OpenOCD's tunneled data scan of a register `w` bits wide: `1`,
    /// the width, the value and one bit more, `000`; it reads the
    /// register from the second bit of the payload on.
    fn dr(sim: &mut dyn Sim, b: &Bscan, w: u32, out: u64) -> u64 {
        let mut v = vec![true];
        v.extend(bits(w as u64, 7));
        v.extend(bits(out, w));
        v.push(false);
        v.extend([false; 3]);
        let got = scan(sim, b, &v);
        value(&got[9..9 + w as usize])
    }

    /// A `dmi` scan: what it sends and the op, data and address that
    /// come back.
    fn dmi(
        sim: &mut dyn Sim,
        b: &Bscan,
        addr: u32,
        data: u32,
        op: u32,
    ) -> (u32, u32, u32) {
        let out = ((addr as u64) << 34) | ((data as u64) << 2) | op as u64;
        let v = dr(sim, b, DMI_WIDTH, out);
        (
            (v & 3) as u32,
            ((v >> 2) & 0xffff_ffff) as u32,
            (v >> 34) as u32,
        )
    }

    /// Idle edges, the run-test cycles OpenOCD waits between scans:
    /// the clock runs and nothing is shifted.
    fn idle(sim: &mut dyn Sim, b: &Bscan, n: u32) {
        for _ in 0..n {
            edge(sim, b, [false, false, false, false, false]);
        }
    }

    /// A transport with a model module behind it: 128 registers, an
    /// access answered `latency` edges after it arrives, and an
    /// address at or above `0x7f` refused.
    fn with_dtm(latency: u32, body: impl FnOnce(&mut dyn Sim, &Bscan)) {
        let (sel_o, sel) = signal::<Bit, Tck>();
        let (shift_o, shift) = signal::<Bit, Tck>();
        let (capture_o, capture) = signal::<Bit, Tck>();
        let (update_o, update) = signal::<Bit, Tck>();
        let (tdi_o, tdi) = signal::<Bit, Tck>();
        let (reset_o, reset) = signal::<Bit, Tck>();
        let (tdo_o, _tdo) = signal::<Bit, Tck>();
        let (req_tx, req_rx) = chan::<U<41>, Tck>();
        let (ans_tx, ans_rx) = chan::<U<34>, Tck>();
        let mut dtm = Dtm::default();
        let b = Bscan {
            sel: sel_o,
            shift: shift_o,
            capture: capture_o,
            update: update_o,
            tdi: tdi_o,
            reset: reset_o,
            out: dtm.out,
        };
        let lat = Rc::new(Cell::new(latency));
        let model = async move {
            let mut regs = [0u32; 128];
            loop {
                Tck::rising().await;
                if let Some(r) = req_rx.recv() {
                    let r = r.raw() as u64;
                    let (addr, data, op) =
                        ((r >> 34) as usize, (r >> 2) as u32, r & 3);
                    for _ in 0..lat.get() {
                        Tck::rising().await;
                    }
                    let (word, status) = if addr >= 0x7f {
                        (0, OP_FAILED)
                    } else if op == OP_WRITE as u64 {
                        regs[addr] = data;
                        (data, 0)
                    } else {
                        (regs[addr], 0)
                    };
                    ans_tx.send(U::<34>::from(
                        ((word as u64) << 2) | status as u64,
                    ));
                }
            }
        };
        let mut sim = Running::new(join2(
            dtm.run(
                (sel, shift, capture, update, tdi, reset, ans_rx),
                (tdo_o, req_tx),
            ),
            model,
        ));
        // Test-Logic-Reset first, as the device's TAP passes through it
        // when OpenOCD starts.
        edge(&mut sim, &b, [false, false, false, false, true]);
        body(&mut sim, &b);
    }

    #[test]
    fn after_reset_a_data_scan_reads_the_idcode() {
        with_dtm(1, |sim, b| {
            assert_eq!(dr(sim, b, 32, 0), IDCODE as u64);
        });
    }

    #[test]
    fn dtmcs_reads_version_one_and_seven_address_bits() {
        with_dtm(1, |sim, b| {
            select(sim, b, IR_DTMCS);
            let d = dr(sim, b, 32, 0) as u32;
            assert_eq!(d & 0xf, 1, "version 1, the 0.13 specification");
            assert_eq!((d >> 4) & 0x3f, ABITS, "abits");
            assert_eq!((d >> 10) & 3, 0, "dmistat clear");
        });
    }

    #[test]
    fn an_unknown_instruction_is_bypass() {
        with_dtm(1, |sim, b| {
            select(sim, b, 0x1f);
            assert_eq!(dr(sim, b, 32, 0xdead_beef), 0);
        });
    }

    #[test]
    fn a_dmi_write_then_read_comes_back() {
        with_dtm(2, |sim, b| {
            select(sim, b, IR_DMI);
            dmi(sim, b, 0x04, 0x1234_5678, OP_WRITE);
            idle(sim, b, 4);
            let (op, _, _) = dmi(sim, b, 0, 0, OP_NOP);
            assert_eq!(op, 0, "the write went");
            dmi(sim, b, 0x04, 0, OP_READ);
            idle(sim, b, 4);
            let (op, data, addr) = dmi(sim, b, 0, 0, OP_NOP);
            assert_eq!((op, data, addr), (0, 0x1234_5678, 0x04));
        });
    }

    #[test]
    fn a_scan_while_an_access_is_in_flight_says_busy_until_dmireset() {
        with_dtm(40, |sim, b| {
            select(sim, b, IR_DMI);
            dmi(sim, b, 0x10, 0, OP_READ);
            let (op, _, _) = dmi(sim, b, 0x10, 0, OP_READ);
            assert_eq!(op, OP_BUSY, "captured while in flight");
            idle(sim, b, 60);
            let (op, _, _) = dmi(sim, b, 0, 0, OP_NOP);
            assert_eq!(op, OP_BUSY, "busy sticks after the access ends");
            select(sim, b, IR_DTMCS);
            let d = dr(sim, b, 32, 0) as u32;
            assert_eq!((d >> 10) & 3, OP_BUSY, "dmistat says busy too");
            dr(sim, b, 32, 1 << 16);
            assert_eq!((dr(sim, b, 32, 0) >> 10) & 3, 0, "dmireset clears it");
            select(sim, b, IR_DMI);
            dmi(sim, b, 0x10, 0, OP_READ);
            idle(sim, b, 60);
            assert_eq!(dmi(sim, b, 0, 0, OP_NOP).0, 0, "and accesses go again");
        });
    }

    #[test]
    fn a_failed_access_sticks_as_failed() {
        with_dtm(1, |sim, b| {
            select(sim, b, IR_DMI);
            dmi(sim, b, 0x7f, 0, OP_READ);
            idle(sim, b, 4);
            assert_eq!(dmi(sim, b, 0, 0, OP_NOP).0, OP_FAILED);
            dmi(sim, b, 0x04, 0, OP_READ);
            idle(sim, b, 4);
            assert_eq!(
                dmi(sim, b, 0, 0, OP_NOP).0,
                OP_FAILED,
                "nothing goes while an error stands"
            );
        });
    }
}
