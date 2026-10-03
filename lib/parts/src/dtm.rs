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
use crate::bus::axi::{BurstKind, Done, Grant, Issue, Resp, R, W};
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
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

/// The system bus registers of the debug module's interface, which the
/// bridge serves itself: `sbcs`, `sbaddress0` and `sbdata0`.
pub const SBCS: u32 = 0x38;
/// `sbaddress0`: the address the next system bus access goes to.
pub const SBADDRESS0: u32 = 0x39;
/// `sbdata0`: the word a system bus access reads or writes.
pub const SBDATA0: u32 = 0x3c;

/// `sbcs` as it always reads: version 1, a 32-bit address, and accesses
/// of 8, 16 and 32 bits (issue 873).
pub const SBCS_FIXED: u32 = (1 << 29) | (32 << 5) | (1 << 2) | (1 << 1) | 1;

/// The error `sbcs` reports for an access the bus refused: "other",
/// since the bus does not say which kind of refusal it was.
pub const SBERROR_BUS: u32 = 7;
/// The error for an access of a width the bridge does not serve, wider
/// than 32 bits.
pub const SBERROR_SIZE: u32 = 4;
/// The error for an access not aligned to its width: a halfword at an
/// odd address, or a word at one that is not a multiple of four.
pub const SBERROR_ALIGN: u32 = 3;

// begin{bridge}
/// The transport's other half, on the system's clock: each access the
/// transport sends becomes a read or a write on the board's link.
///
/// An access to `sbcs`, `sbaddress0` or `sbdata0` is the debug
/// module's system bus access (0.13, section 3.12), served here,
/// where the bus master is. It is answered at once, and the access it
/// starts runs on the link behind it, as the specification has it. Any
/// other address is one of the debug module's registers, which sits on
/// the link at `DM`, each register a word at four times its number; that
/// access is answered when its word comes back. One access is on the
/// link at a time, which is all a transport asks for, so the bridge's
/// host needs identifiers of one bit, `I`.
#[derive(Trace, Default)]
pub struct DtmBridge<const DM: usize, const I: usize> {
    /// An access is on the link, from its address phase to its answer.
    pub busy: Reg<Bit>,
    /// Its address phase has gone out.
    pub issued: Reg<Bit>,
    /// Its identifier, once granted.
    pub held: Reg<Bit>,
    /// The identifier.
    pub id: Reg<U<I>>,
    /// It reads.
    pub isrd: Reg<Bit>,
    /// It is a debug module register, which the transport waits on,
    /// rather than a system bus access, which it does not.
    pub fordmi: Reg<Bit>,
    /// Its address and the word it writes.
    pub addr: Reg<U<32>>,
    /// The word a write of the access sends.
    pub wdata: Reg<U<32>>,
    /// `sbaddress0`.
    pub sbaddr: Reg<U<32>>,
    /// `sbdata0`.
    pub sbdata: Reg<U<32>>,
    /// `sbreadonaddr`: a write of `sbaddress0` starts a read.
    pub rdonaddr: Reg<Bit>,
    /// `sbreadondata`: a read of `sbdata0` starts the next read.
    pub rdondata: Reg<Bit>,
    /// `sbautoincrement`: the address moves on after each access.
    pub autoinc: Reg<Bit>,
    /// `sbaccess`: the width, 0 for 8 bits, 1 for 16 and 2 for 32.
    pub sbaccess: Reg<U<3>>,
    /// The width of the access on the link, as `sbaccess` was when it
    /// started.
    pub width: Reg<U<3>>,
    /// `sberror`, cleared by writing ones to it.
    pub sberror: Reg<U<3>>,
    /// `sbbusyerror`: an access was asked for while one ran.
    pub busyerr: Reg<Bit>,
}
// end{bridge}

// begin{bridge_run}
#[lower]
impl<const DM: usize, const I: usize> Unit for DtmBridge<DM, I> {
    async fn run(
        &mut self,
        (dmi, grant, done, rdata): (
            Rx<U<41>>,
            Rx<Grant<I>>,
            Rx<Done<I>>,
            Rx<R<32, I>>,
        ),
        (ans, issue, wbeat, release): (
            Tx<U<34>>,
            Tx<Issue<32>>,
            Tx<W<32, 4>>,
            Tx<Grant<I>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let busy = self.busy.get();
            let issued = self.issued.get();
            let held = self.held.get();
            let isrd = self.isrd.get();
            let fordmi = self.fordmi.get();
            let sbaddr = self.sbaddr.get();
            let sberror = self.sberror.get();
            let busyerr = self.busyerr.get();
            // An access from the transport, taken when nothing is on
            // the link and an answer could go at once.
            let q = dmi.head();
            let take = Bit::from(dmi.peek().is_some()) & !busy & ans.ready();
            let _ = dmi.recv_if(!busy & ans.ready());
            let a = q.slice::<34, 7>();
            let d = q.slice::<2, 32>();
            let wr = Bit::from(q.slice::<0, 2>() == 2);
            let is_cs = Bit::from(a == SBCS);
            let is_ad = Bit::from(a == SBADDRESS0);
            let is_da = Bit::from(a == SBDATA0);
            let is_sb = is_cs | is_ad | is_da;
            // What the system bus registers read as.
            let sbcs = U::<32>::from(SBCS_FIXED)
                | (busyerr.zext::<32>() << 22u32)
                | (self.rdonaddr.get().zext::<32>() << 20u32)
                | (self.sbaccess.get().zext::<32>() << 17u32)
                | (self.autoinc.get().zext::<32>() << 16u32)
                | (self.rdondata.get().zext::<32>() << 15u32)
                | (sberror.zext::<32>() << 12u32);
            let sbword = select!(a.raw() => {
                0x38 => sbcs,
                0x39 => sbaddr,
                0x3c => self.sbdata.get(),
                _ => U::<32>::from(0u8),
            });
            // A system bus access asked for: a write of `sbaddress0`
            // under `sbreadonaddr`, a write of `sbdata0`, or a read of
            // it under `sbreadondata`. It goes only with no error
            // standing, at a width served, 8, 16 or 32 bits, and at an
            // address aligned to it (issue 873).
            let clear = (sberror == 0) & !busyerr;
            let acc = self.sbaccess.get();
            let sb_rd_addr = take & is_ad & wr & self.rdonaddr.get();
            let sb_wr = take & is_da & wr;
            let sb_rd_data = take & is_da & !wr & self.rdondata.get();
            let sb_ask = sb_rd_addr | sb_wr | sb_rd_data;
            let sb_at = mux(sb_rd_addr, d, sbaddr);
            let lane_at = sb_at.slice::<0, 2>();
            let served =
                Bit::from(acc == 0) | Bit::from(acc == 1) | Bit::from(acc == 2);
            let misaligned = (Bit::from(acc == 1) & lane_at.bit(0))
                | (Bit::from(acc == 2) & Bit::from(lane_at != 0));
            let sb_go = sb_ask & clear & served & !misaligned;
            let sb_bad = sb_ask & clear & !served;
            let sb_mis = sb_ask & clear & served & misaligned;
            // A debug module register: the access goes on the link and
            // is answered when it comes back.
            let dm_go = take & !is_sb;
            let dm_at = U::<32>::from(DM as u32) + (a.zext::<32>() << 2u32);
            // The link: the address phase, with a write's beat beside
            // it, then the identifier, then the answer.
            let rd_now = isrd;
            let go_issue =
                busy & !issued & issue.ready() & (rd_now | wbeat.ready());
            let take_id = Bit::from(grant.peek().is_some());
            let gid = grant.head().id;
            let _ = grant.recv_if(true);
            let wdone = Bit::from(done.peek().is_some());
            let rdone = Bit::from(rdata.peek().is_some());
            let back_ok = mux(
                isrd,
                Bit::from(rdata.head().resp == Resp::Okay),
                Bit::from(done.head().resp == Resp::Okay),
            );
            let word = rdata.head().data;
            // A narrow access is a word on the link at the aligned address:
            // a write's data moved into its lanes with only their strobes
            // set, and a read's lane moved down and the rest cleared.
            let width = self.width.get();
            let lane = self.addr.get().slice::<0, 2>();
            let shifted = select!(lane.raw() => {
                0 => word,
                1 => word >> 8u32,
                2 => word >> 16u32,
                _ => word >> 24u32,
            });
            let narrow = mux(
                Bit::from(width == 0),
                shifted & U::<32>::from(0xffu32),
                mux(
                    Bit::from(width == 1),
                    shifted & U::<32>::from(0xffffu32),
                    shifted,
                ),
            );
            let wd = self.wdata.get();
            let placed = select!(lane.raw() => {
                0 => wd,
                1 => wd << 8u32,
                2 => wd << 16u32,
                _ => wd << 24u32,
            });
            let strb8 = select!(lane.raw() => {
                0 => U::<4>::from(1u8),
                1 => U::<4>::from(2u8),
                2 => U::<4>::from(4u8),
                _ => U::<4>::from(8u8),
            });
            let strb16 =
                mux(lane.bit(1), U::<4>::from(0xcu8), U::<4>::from(3u8));
            let strb = mux(
                Bit::from(width == 0),
                strb8,
                mux(Bit::from(width == 1), strb16, U::<4>::from(0xfu8)),
            );
            let step = select!(width.raw() => {
                0 => U::<32>::from(1u8),
                1 => U::<32>::from(2u8),
                _ => U::<32>::from(4u8),
            });
            let fin = busy
                & held
                & mux(isrd, rdone, wdone)
                & release.ready()
                & (!fordmi | ans.ready());
            let _ = done.recv_if(fin & !isrd);
            let _ = rdata.recv_if(fin & isrd);
            let sb_fin = fin & !fordmi;
            let status = mux(back_ok, U::<2>::from(0u8), U::<2>::from(2u8));
            let reply = mux(
                take,
                sbword.zext::<34>() << 2u32,
                (mux(isrd, word, U::<32>::from(0u8)).zext::<34>() << 2u32)
                    | status.zext::<34>(),
            );
            let answering = (take & is_sb) | (fin & fordmi);
            with!(self <= {
                // `sbcs`: the settings written, the errors cleared by
                // writing ones.
                take & is_cs & wr ? {
                    rdonaddr: d.bit(20),
                    sbaccess: d.slice::<17, 3>(),
                    autoinc: d.bit(16),
                    rdondata: d.bit(15),
                    sberror: sberror & !d.slice::<12, 3>(),
                },
                take & is_cs & wr & d.bit(22) ? busyerr: Bit::Zero,
                take & is_ad & wr ? sbaddr: d,
                take & is_da & wr ? sbdata: d,
                sb_bad ? sberror: U::<3>::from(SBERROR_SIZE),
                sb_mis ? sberror: U::<3>::from(SBERROR_ALIGN),
                sb_go ? {
                    busy: Bit::One,
                    issued: Bit::Zero,
                    held: Bit::Zero,
                    isrd: !sb_wr,
                    fordmi: Bit::Zero,
                    addr: sb_at,
                    wdata: d,
                    width: acc,
                },
                dm_go ? {
                    busy: Bit::One,
                    issued: Bit::Zero,
                    held: Bit::Zero,
                    isrd: !wr,
                    fordmi: Bit::One,
                    addr: dm_at,
                    wdata: d,
                    width: U::<3>::from(2u8),
                },
                go_issue ? issued: Bit::One,
                take_id ? { id: gid, held: Bit::One },
                fin ? { busy: Bit::Zero, issued: Bit::Zero, held: Bit::Zero },
                sb_fin & isrd ? sbdata: narrow,
                sb_fin & self.autoinc.get() ? sbaddr: self.addr.get() + step,
                sb_fin & !back_ok & (sberror == 0) ?
                    sberror: U::<3>::from(SBERROR_BUS),
            });
            if go_issue.to_bool() {
                issue.send(Issue {
                    read: isrd,
                    addr: self.addr.get() & U::<32>::from(0xffff_fffcu32),
                    len: U::<8>::from(0u8),
                    size: U::<3>::from(2u8),
                    burst: BurstKind::Incr,
                    lock: Bit::Zero,
                    cache: U::<4>::from(0u8),
                    prot: U::<3>::from(0u8),
                    qos: U::<4>::from(0u8),
                    region: U::<4>::from(0u8),
                });
            }
            if (go_issue & !isrd).to_bool() {
                wbeat.send(W {
                    data: placed,
                    strb,
                    last: Bit::One,
                });
            }
            if fin.to_bool() {
                release.send(Grant { id: self.id.get() });
            }
            if answering.to_bool() {
                ans.send(reply);
            }
        }
    }
}
// end{bridge_run}

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

#[cfg(test)]
mod bridge_tests {
    use super::*;
    use crate::bus::axi::{axi_units, Answer, AxiHost, AxiPer};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use txhdl::comp::{chan, join2, Running};

    /// Where the module's registers sit in these tests.
    const DM: usize = 0x8000;
    /// An address the model peripheral refuses.
    const REFUSED: u32 = 0xf000_0000;

    /// One access from the transport: the address, the word, the op.
    fn access(a: u32, d: u32, op: u32) -> U<41> {
        U::<41>::from(((a as u64) << 34) | ((d as u64) << 2) | op as u64)
    }

    /// The bridge on a link, with a model peripheral behind the link
    /// holding `mem`, and `script` played as accesses from the
    /// transport. What came back, word and status, is returned.
    fn with_bridge(
        mem: Rc<RefCell<HashMap<u32, u32>>>,
        script: Vec<(u32, u32, u32)>,
    ) -> Vec<(u32, u32)> {
        let link = axi_units::<32, 32, 4, 1>();
        let (issue, wbeat, release, grant, done, rdata) = link.host_client;
        let (req, wd, answer, rb) = link.per_client;
        let (dmi_tx, dmi_rx) = chan::<U<41>, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<U<34>, DefaultClock>();
        let mut bridge = DtmBridge::<DM, 1>::default();
        let mut host = AxiHost::<32, 32, 4, 1, 2>::default();
        let mut per = AxiPer::<32, 32, 4, 1>::default();
        let got = Rc::new(RefCell::new(Vec::new()));
        let out = got.clone();
        let n = script.len();
        let driver = async move {
            for (a, d, op) in script {
                DefaultClock::rising().await;
                while !dmi_tx.ready().to_bool() {
                    DefaultClock::rising().await;
                }
                dmi_tx.send(access(a, d, op));
                loop {
                    DefaultClock::rising().await;
                    if let Some(v) = ans_rx.recv() {
                        let v = v.raw() as u64;
                        out.borrow_mut()
                            .push(((v >> 2) as u32, (v & 3) as u32));
                        break;
                    }
                }
            }
            loop {
                DefaultClock::rising().await;
            }
        };
        let model = async move {
            loop {
                DefaultClock::rising().await;
                if let Some(q) = req.recv() {
                    let at = q.addr.raw() as u32;
                    assert_eq!(at & 3, 0, "the link carries word addresses");
                    let ok = at < REFUSED;
                    let resp = if ok { Resp::Okay } else { Resp::SlvErr };
                    if q.read == Bit::One {
                        let v = *mem.borrow().get(&at).unwrap_or(&0);
                        DefaultClock::rising().await;
                        rb.send(R {
                            id: q.id,
                            data: U::<32>::from(v),
                            resp,
                            last: Bit::One,
                        });
                    } else {
                        let beat = loop {
                            if let Some(b) = wd.recv() {
                                break b;
                            }
                            DefaultClock::rising().await;
                        };
                        if ok {
                            // Only the bytes the strobes name change.
                            let strb = beat.strb.raw() as u32;
                            let mask = (0..4)
                                .filter(|i| strb >> i & 1 == 1)
                                .fold(0u32, |m, i| m | 0xff << (8 * i));
                            let old = *mem.borrow().get(&at).unwrap_or(&0);
                            let new =
                                old & !mask | beat.data.raw() as u32 & mask;
                            mem.borrow_mut().insert(at, new);
                        }
                        DefaultClock::rising().await;
                        answer.send(Answer { id: q.id, resp });
                    }
                }
            }
        };
        let mut sim = Running::new(join2(
            join2(
                bridge.run(
                    (dmi_rx, grant, done, rdata),
                    (ans_tx, issue, wbeat, release),
                ),
                join2(
                    host.run(link.host_in, link.host_out),
                    per.run(link.per_in, link.per_out),
                ),
            ),
            join2(driver, model),
        ));
        for _ in 0..4000 {
            sim.cycle();
            if got.borrow().len() == n {
                break;
            }
        }
        let v = got.borrow().clone();
        assert_eq!(v.len(), n, "every access was answered: {v:x?}");
        v
    }

    fn mem(words: &[(u32, u32)]) -> Rc<RefCell<HashMap<u32, u32>>> {
        Rc::new(RefCell::new(words.iter().copied().collect()))
    }

    #[test]
    fn a_module_register_is_a_word_at_four_times_its_number() {
        let m = mem(&[]);
        let got = with_bridge(
            m.clone(),
            vec![(0x10, 0x8000_0001, OP_WRITE), (0x10, 0, OP_READ)],
        );
        assert_eq!(got, vec![(0, 0), (0x8000_0001, 0)]);
        assert_eq!(m.borrow()[&(DM as u32 + 0x40)], 0x8000_0001);
    }

    #[test]
    fn sbcs_reads_version_one_and_its_three_widths() {
        let got = with_bridge(mem(&[]), vec![(SBCS, 0, OP_READ)]);
        let cs = got[0].0;
        assert_eq!(cs >> 29, 1, "sbversion 1");
        assert_eq!((cs >> 5) & 0x7f, 32, "sbasize 32");
        assert_eq!(cs & 0x1f, 0b111, "8, 16 and 32-bit accesses");
    }

    /// `sbcs` for reads on a write of the address, at width `access`:
    /// 0 for 8 bits, 1 for 16, 2 for 32.
    fn rd_cs(access: u32) -> u32 {
        (1 << 20) | (access << 17)
    }

    /// Each byte and each halfword of a word, read alone: the lane
    /// comes back at the bottom of `sbdata0`, and nothing above it.
    #[test]
    fn narrow_reads_take_every_lane() {
        let m = mem(&[(0x400, 0x4433_2211)]);
        let mut acc = vec![(SBCS, rd_cs(0), OP_WRITE)];
        for a in 0x400..0x404 {
            acc.extend([(SBADDRESS0, a, OP_WRITE), (SBDATA0, 0, OP_READ)]);
        }
        acc.push((SBCS, rd_cs(1), OP_WRITE));
        for a in [0x400, 0x402] {
            acc.extend([(SBADDRESS0, a, OP_WRITE), (SBDATA0, 0, OP_READ)]);
        }
        acc.push((SBCS, 0, OP_READ));
        let got = with_bridge(m, acc);
        let bytes: Vec<u32> = (0..4).map(|i| got[2 + 2 * i].0).collect();
        assert_eq!(bytes, vec![0x11, 0x22, 0x33, 0x44], "the four bytes");
        let halves: Vec<u32> = (0..2).map(|i| got[11 + 2 * i].0).collect();
        assert_eq!(halves, vec![0x2211, 0x4433], "the two halfwords");
        assert_eq!((got[14].0 >> 12) & 7, 0, "no error");
    }

    /// Each byte and each halfword written alone: only its lanes of the
    /// word change.
    #[test]
    fn narrow_writes_change_only_their_lanes() {
        let m = mem(&[(0x500, 0xaaaa_aaaa), (0x504, 0xaaaa_aaaa)]);
        let mut acc = vec![(SBCS, 0, OP_WRITE)];
        for (i, a) in (0x500..0x504).enumerate() {
            acc.extend([
                (SBADDRESS0, a, OP_WRITE),
                (SBDATA0, 0x11 * (i as u32 + 1), OP_WRITE),
            ]);
        }
        acc.push((SBCS, 1 << 17, OP_WRITE));
        acc.extend([
            (SBADDRESS0, 0x506, OP_WRITE),
            (SBDATA0, 0xbeef, OP_WRITE),
            (SBCS, 0, OP_READ),
        ]);
        let got = with_bridge(m.clone(), acc);
        assert_eq!(m.borrow()[&0x500], 0x4433_2211, "byte by byte");
        assert_eq!(m.borrow()[&0x504], 0xbeef_aaaa, "the upper halfword");
        assert_eq!((got.last().unwrap().0 >> 12) & 7, 0, "no error");
    }

    /// `sbautoincrement` steps by the width: four byte reads in a row,
    /// each started by the read of the one before, under
    /// `sbreadondata`, walk the word a byte at a time.
    #[test]
    fn autoincrement_steps_by_the_width() {
        let m = mem(&[(0x400, 0x4433_2211)]);
        let cs = rd_cs(0) | (1 << 16) | (1 << 15);
        let got = with_bridge(
            m,
            vec![
                (SBCS, cs, OP_WRITE),
                (SBADDRESS0, 0x400, OP_WRITE),
                (SBDATA0, 0, OP_READ),
                (SBDATA0, 0, OP_READ),
                (SBDATA0, 0, OP_READ),
                (SBDATA0, 0, OP_READ),
                (SBADDRESS0, 0, OP_READ),
            ],
        );
        let bytes: Vec<u32> = got[2..6].iter().map(|g| g.0).collect();
        assert_eq!(bytes, vec![0x11, 0x22, 0x33, 0x44], "byte after byte");
        // Five reads: the one the address started and one per read of
        // the data, the last too, each a byte on.
        assert_eq!(got[6].0, 0x405, "the address moved on a byte each");
    }

    /// An access not aligned to its width is refused with `sberror` 3,
    /// as the specification has it, and reaches no memory: a halfword
    /// at an odd address, a word at an address not a multiple of four.
    /// A width over 32 bits is refused with 4.
    #[test]
    fn misaligned_and_oversized_accesses_are_refused() {
        let m = mem(&[(0x400, 0x4433_2211)]);
        let clear = 7 << 12;
        let got = with_bridge(
            m.clone(),
            vec![
                (SBCS, 1 << 17, OP_WRITE),
                (SBADDRESS0, 0x401, OP_WRITE),
                (SBDATA0, 0xffff, OP_WRITE),
                (SBCS, 0, OP_READ),
                (SBCS, (2 << 17) | clear, OP_WRITE),
                (SBADDRESS0, 0x402, OP_WRITE),
                (SBDATA0, 0xffff_ffff, OP_WRITE),
                (SBCS, 0, OP_READ),
                (SBCS, (3 << 17) | clear, OP_WRITE),
                (SBADDRESS0, 0x400, OP_WRITE),
                (SBDATA0, 0xffff_ffff, OP_WRITE),
                (SBCS, 0, OP_READ),
            ],
        );
        assert_eq!((got[3].0 >> 12) & 7, 3, "a halfword at an odd address");
        assert_eq!((got[7].0 >> 12) & 7, 3, "a word at a halfword address");
        assert_eq!((got[11].0 >> 12) & 7, 4, "64 bits");
        assert_eq!(m.borrow()[&0x400], 0x4433_2211, "nothing written");
    }

    /// What OpenOCD does to put a breakpoint over a compressed
    /// instruction and take it out again: read the halfword, write
    /// `c.ebreak` over it, read it back, write the instruction back and
    /// read that back. The halfword beside it never changes.
    #[test]
    fn a_compressed_breakpoint_goes_in_and_comes_out() {
        let m = mem(&[(0x700, 0x1234_5678)]);
        let got = with_bridge(
            m.clone(),
            vec![
                (SBCS, rd_cs(1), OP_WRITE),
                (SBADDRESS0, 0x702, OP_WRITE),
                (SBDATA0, 0, OP_READ),
                (SBDATA0, 0x9002, OP_WRITE),
                (SBADDRESS0, 0x702, OP_WRITE),
                (SBDATA0, 0, OP_READ),
                (SBDATA0, 0x1234, OP_WRITE),
                (SBADDRESS0, 0x702, OP_WRITE),
                (SBDATA0, 0, OP_READ),
                (SBCS, 0, OP_READ),
            ],
        );
        assert_eq!(got[2].0, 0x1234, "the instruction");
        assert_eq!(got[5].0, 0x9002, "c.ebreak in its place");
        assert_eq!(got[8].0, 0x1234, "the instruction again");
        assert_eq!(m.borrow()[&0x700], 0x1234_5678, "the word as it was");
        assert_eq!((got[9].0 >> 12) & 7, 0, "no error");
    }

    #[test]
    fn system_bus_reads_follow_the_address_and_the_data() {
        let m = mem(&[(0x100, 0x1111_1111), (0x104, 0x2222_2222)]);
        // `sbreadonaddr`, `sbaccess` 2, `sbautoincrement`, `sbreadondata`.
        let cs = (1 << 20) | (2 << 17) | (1 << 16) | (1 << 15);
        let got = with_bridge(
            m,
            vec![
                (SBCS, cs, OP_WRITE),
                (SBADDRESS0, 0x100, OP_WRITE),
                (SBDATA0, 0, OP_READ),
                (SBDATA0, 0, OP_READ),
                (SBADDRESS0, 0, OP_READ),
            ],
        );
        assert_eq!(got[2], (0x1111_1111, 0), "the read the address started");
        assert_eq!(got[3], (0x2222_2222, 0), "the read the data started");
        // Three reads, each moving the address on: the one the address
        // started, and one for each read of the data, the second too.
        assert_eq!(got[4], (0x10c, 0), "the address moved on three times");
    }

    #[test]
    fn a_system_bus_write_lands() {
        let m = mem(&[]);
        let got = with_bridge(
            m.clone(),
            vec![
                (SBCS, 2 << 17, OP_WRITE),
                (SBADDRESS0, 0x200, OP_WRITE),
                (SBDATA0, 0xcafe_f00d, OP_WRITE),
                (SBCS, 0, OP_READ),
            ],
        );
        assert_eq!(m.borrow()[&0x200], 0xcafe_f00d);
        assert_eq!((got[3].0 >> 12) & 7, 0, "no error");
    }

    #[test]
    fn a_refused_access_sticks_in_sberror_until_cleared() {
        let m = mem(&[(0x300, 7)]);
        let rd = (1 << 20) | (2 << 17);
        let got = with_bridge(
            m,
            vec![
                (SBCS, rd, OP_WRITE),
                (SBADDRESS0, REFUSED, OP_WRITE),
                (SBCS, 0, OP_READ),
                (SBADDRESS0, 0x300, OP_WRITE),
                (SBDATA0, 0, OP_READ),
                (SBCS, rd | (7 << 12), OP_WRITE),
                (SBADDRESS0, 0x300, OP_WRITE),
                (SBDATA0, 0, OP_READ),
            ],
        );
        assert_eq!((got[2].0 >> 12) & 7, SBERROR_BUS, "the refusal");
        assert_eq!(got[4].0, 0, "nothing goes while the error stands");
        assert_eq!(got[7].0, 7, "and goes again once it is cleared");
    }

    /// A read of 64 bits, started by the address under `sbreadonaddr`,
    /// is refused with the size error.
    #[test]
    fn a_width_over_32_bits_is_refused() {
        let got = with_bridge(
            mem(&[]),
            vec![
                (SBCS, rd_cs(3), OP_WRITE),
                (SBADDRESS0, 0x100, OP_WRITE),
                (SBCS, 0, OP_READ),
            ],
        );
        assert_eq!((got[2].0 >> 12) & 7, SBERROR_SIZE);
    }
}
