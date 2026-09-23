// SPDX-License-Identifier: Apache-2.0
//! The debug module: the RISC-V debug specification's module, as far
//! as a debugger reaching it over the bus needs it (issue 154, step
//! two of three).
//!
//! The specification puts the module behind a debug module interface,
//! a small address space of 32-bit registers a transport reaches from
//! the cable. Here that space is on the bus: an AXI-Lite peripheral
//! at `DM_BASE`, each of the interface's registers a word at four
//! times its number, so the JTAG-to-AXI host of the board reaches it
//! from the hardware manager today and a transport of its own reaches
//! it the same way when there is one. The registers are the
//! specification's, at their numbers, with what this design has:
//!
//! | number | register     | what                                          |
//! |--------|--------------|-----------------------------------------------|
//! | `0x04` | `data0`      | the abstract command's one data word           |
//! | `0x10` | `dmcontrol`  | `haltreq` 31, `resumereq` 30, `dmactive` 0     |
//! | `0x11` | `dmstatus`   | halted, running, resume acknowledged, version  |
//! | `0x12` | `hartinfo`   | zero: no scratch registers, no data window     |
//! | `0x16` | `abstractcs` | `busy` 12, `cmderr` 10 to 8, `datacount` 1    |
//! | `0x17` | `command`    | access register, 32 bits, `regno` 15 to 0      |
//! | `0x40` | `haltsum0`   | bit 0, the one hart, halted                    |
//!
//! One hart, so `hartsel` is not decoded. The abstract command is the
//! access-register command for a general register, `0x1000` to
//! `0x101f`, or a CSR, which the core reads for it and, for `dpc` and
//! `dcsr`, writes for it; any other command is refused with
//! `cmderr` 2, a command while the core runs with 4, a command while
//! one is in flight with 1, and a command while `cmderr` stands is
//! ignored, all as the specification says. A command takes two
//! cycles: one for the number to reach the core, one for the word to
//! come back or go in. No program buffer and no system bus access:
//! memory is reached through the bus the module sits on.
//!
//! The core's side is a halt request and a resume request in, and
//! debug mode out, which is what `dmstatus` reports; the resume
//! request drops and is acknowledged when the core has left debug
//! mode. A halt request stays as written, as the specification has
//! it: a debugger clears it before it resumes. A resume request is an
//! action: a one asks, if the core is halted as the write lands, and
//! clears the acknowledgement; a zero does nothing.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};
use txhdl_parts::bus::axi::Resp;
use txhdl_parts::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};

/// Where the module sits on the board's bus, and the bits an address
/// must match: 64 KiB at `0x1000_0000`, a router port of its own. Not
/// nearer the interrupt controller: its window at `0x0c00_0000` is 64
/// MiB, up to `0x0fff_ffff`, and the router gives the lowest match
/// the transaction.
pub const DM_BASE: u32 = 0x1000_0000;
pub const DM_MASK: u32 = 0xffff_0000;

/// The interface's register numbers, as the specification has them.
pub const DATA0: u32 = 0x04;
pub const DMCONTROL: u32 = 0x10;
pub const DMSTATUS: u32 = 0x11;
pub const HARTINFO: u32 = 0x12;
pub const ABSTRACTCS: u32 = 0x16;
pub const COMMAND: u32 = 0x17;
pub const HALTSUM0: u32 = 0x40;

/// A register's address on the bus: four bytes per number.
pub const fn at(number: u32) -> u32 {
    DM_BASE + number * 4
}

/// `dmcontrol` bits.
pub const HALTREQ: u32 = 1 << 31;
pub const RESUMEREQ: u32 = 1 << 30;
pub const DMACTIVE: u32 = 1;

/// `dmstatus` bits: `allhalted` and `anyhalted`, `allrunning` and
/// `anyrunning`, `allresumeack` and `anyresumeack`, `authenticated`,
/// and the version, 2 for the 0.13 specification.
pub const ALLHALTED: u32 = 3 << 8;
pub const ALLRUNNING: u32 = 3 << 10;
pub const ALLRESUMEACK: u32 = 3 << 16;
pub const AUTHENTICATED: u32 = 1 << 7;
pub const VERSION: u32 = 2;

/// The register numbers of the access-register command: the general
/// registers from `0x1000`, and the CSRs at their own numbers.
pub const REGNO_GPR: u32 = 0x1000;

/// The access-register command for one 32-bit register: `cmdtype`
/// zero, `aarsize` 2, `transfer`, and `write` when it is one.
pub const fn access(regno: u32, write: bool) -> u32 {
    (2 << 20) | (1 << 17) | ((write as u32) << 16) | (regno & 0xffff)
}

// begin{dm}
/// The module's state.
#[derive(Trace, Default)]
pub struct Dm {
    /// `dmactive`: the module is in use; clear, it holds nothing.
    pub active: Reg<Bit>,
    /// `haltreq`, as the debugger wrote it.
    pub haltreq: Reg<Bit>,
    /// `resumereq`, until the core has resumed.
    pub resumereq: Reg<Bit>,
    /// `allresumeack`: the core resumed since the last request.
    pub resumeack: Reg<Bit>,
    /// `data0`: what a read brought back, or what a write sends.
    pub data0: Reg<U<32>>,
    /// `cmderr`, cleared by writing ones to it.
    pub cmderr: Reg<U<3>>,
    /// A command in flight: 1 the cycle the core is asked, 2 the cycle
    /// it answers, 0 between commands.
    pub stage: Reg<U<2>>,
    /// The command in flight: its register and whether it writes.
    pub regno: Reg<U<16>>,
    pub write: Reg<Bit>,
}
// end{dm}

// begin{run}
#[lower]
impl Unit for Dm {
    async fn run(
        &mut self,
        (aw, ar, w, halted, rdata): (
            Rx<LiteAw<32>>,
            Rx<LiteAr<32>>,
            Rx<LiteW<32, 4>>,
            In<Bit>,
            In<U<32>>,
        ),
        (b, r, haltreq_o, resumereq_o, regno_o, wdata_o, we_o): (
            Tx<LiteB>,
            Tx<LiteR<32>>,
            Out<Bit>,
            Out<Bit>,
            Out<U<16>>,
            Out<U<32>>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let halted = halted.get();
            let active = self.active.get();
            let (hreq, rreq) = (self.haltreq.get(), self.resumereq.get());
            let rack = self.resumeack.get();
            let cmderr = self.cmderr.get();
            let stage = self.stage.get();
            let busy = Bit::from(stage != 0);
            // The bus: one read and one write a cycle, as any of the
            // small peripherals; the number is the word address.
            let arh = ar.head();
            let awh = aw.head();
            let wh = w.head();
            let rsel = arh.addr.slice::<2, 7>();
            let wsel = awh.addr.slice::<2, 7>();
            let rgo = r.ready() & ar.peek().is_some();
            let _ = ar.recv_if(r.ready());
            let wgo = b.ready() & aw.peek().is_some() & w.peek().is_some();
            let _ = aw.recv_if(wgo);
            let _ = w.recv_if(wgo);
            let wd = wh.data;
            // What the registers read as.
            let dmcontrol = hreq
                .zext::<1>()
                .concat::<1, 2>(rreq.zext::<1>())
                .concat::<29, 31>(U::<29>::from(0u8))
                .concat::<1, 32>(active.zext::<1>());
            let dmstatus = U::<32>::from(VERSION | AUTHENTICATED)
                | mux(halted, U::<32>::from(ALLHALTED), U::<32>::from(0u8))
                | mux(halted, U::<32>::from(0u8), U::<32>::from(ALLRUNNING))
                | mux(rack, U::<32>::from(ALLRESUMEACK), U::<32>::from(0u8));
            let abstractcs = U::<32>::from(1u8)
                | (cmderr.zext::<32>() << 8)
                | (busy.zext::<32>() << 12);
            let word = select!(rsel.raw() => {
                0x04 => self.data0.get(),
                0x10 => dmcontrol,
                0x11 => dmstatus,
                0x16 => abstractcs,
                0x40 => halted.zext::<32>(),
                _ => U::<32>::from(0u8),
            });
            // A write of `dmcontrol`, honoured when the module is or
            // becomes active; and of `command`, honoured when active,
            // when nothing is in flight and no error stands.
            let ctl = wgo & (wsel == DMCONTROL);
            let on = ctl & wd.bit(0);
            let cmd = wgo & (wsel == COMMAND) & active & (cmderr == 0);
            let cmdtype = wd.slice::<24, 8>();
            let aarsize = wd.slice::<20, 3>();
            let transfer = wd.bit(17);
            let wr = wd.bit(16);
            let rn = wd.slice::<0, 16>();
            let is_gpr = Bit::from(rn.slice::<5, 11>() == 0x80);
            let is_csr = Bit::from(rn.slice::<12, 4>() == 0);
            let csr_writable = Bit::from(rn == 0x7b0) | Bit::from(rn == 0x7b1);
            let supported = Bit::from(cmdtype == 0)
                & (aarsize == 2)
                & (is_gpr | (is_csr & (!wr | csr_writable)));
            let err_busy = cmd & busy;
            let err_unsupported = cmd & !busy & !supported;
            let err_running = cmd & !busy & supported & !halted;
            let go = cmd & !busy & supported & halted & transfer;
            let asking = Bit::from(stage == 1);
            let answered = Bit::from(stage == 2);
            // The arms apply in order and the last drive of a field wins,
            // so what fires together is ordered by who should win: the
            // core's completion of a resume before the debugger's write,
            // which may be a new request in the same cycle (issue 439).
            with!(self <= {
                ctl ? active: wd.bit(0),
                // The core left debug mode: the request is done.
                rreq & !halted ? { resumereq: Bit::Zero, resumeack: Bit::One },
                // `haltreq` is a level, written as such; `resumereq` is an
                // action: a one asks for a resume if the core is halted
                // as the write lands and does nothing if it is running, a
                // zero does nothing, and either way the one clears the
                // acknowledgement.
                on ? haltreq: wd.bit(31),
                on & wd.bit(30) ? resumeack: Bit::Zero,
                on & wd.bit(30) & halted ? resumereq: Bit::One,
                // `data0` is the debugger's between commands; touched while
                // one is in flight, the access is the busy error and
                // writes nothing, as the specification says.
                wgo & (wsel == DATA0) & !busy ? data0: wd,
                wgo & (wsel == DATA0) & busy & (cmderr == 0) ?
                    cmderr: U::<3>::from(1u8),
                wgo & (wsel == ABSTRACTCS) ? cmderr: cmderr & !wd.slice::<8, 3>(),
                err_busy ? cmderr: U::<3>::from(1u8),
                err_unsupported ? cmderr: U::<3>::from(2u8),
                err_running ? cmderr: U::<3>::from(4u8),
                go ? { stage: U::<2>::from(1u8), regno: rn, write: wr },
                asking ? stage: U::<2>::from(2u8),
                answered ? stage: U::<2>::from(0u8),
                answered & !self.write ? data0: rdata.get(),
                // Inactive, the module holds nothing.
                !active & !on ? {
                    haltreq: Bit::Zero,
                    resumereq: Bit::Zero,
                    resumeack: Bit::Zero,
                    cmderr: U::<3>::from(0u8),
                    stage: U::<2>::from(0u8),
                },
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
            haltreq_o.set(hreq);
            resumereq_o.set(rreq);
            regno_o.set(self.regno.get());
            wdata_o.set(self.data0.get());
            we_o.set(asking & self.write.get());
        }
    }
}
// end{run}

#[cfg(test)]
mod tests {
    use super::*;
    use txhdl::comp::{join2, signal, Running};
    use txhdl_parts::bus::axi_lite::{axi_lite, LiteHost};

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

    /// A read of one word.
    async fn read(h: &Host, addr: u32) -> u32 {
        let (_, ar, _, _, r) = h;
        ar.send(LiteAw {
            addr: U::from(addr),
            prot: U::from(0u8),
        });
        loop {
            DefaultClock::rising().await;
            if let Some(got) = r.recv() {
                return got.data.raw() as u32;
            }
        }
    }

    async fn cycles(n: usize) {
        for _ in 0..n {
            DefaultClock::rising().await;
        }
    }

    /// What a test holds: the link's host end and the core's side,
    /// played by the test: whether the core is halted and what it
    /// answers a register read with; and the module's lines to the
    /// core, read back.
    struct Rig {
        host: Host,
        halted: Out<Bit>,
        rdata: Out<U<32>>,
        haltreq: In<Bit>,
        resumereq: In<Bit>,
        regno: In<U<16>>,
        wdata: In<U<32>>,
        we: In<Bit>,
    }

    /// Run `client` against a module until it is done.
    fn run<F>(client: impl FnOnce(Rig) -> F)
    where
        F: std::future::Future<Output = ()>,
    {
        let link = axi_lite::<32, 32, 4>();
        let (aw, ar, w, b, r) = link.per;
        let (halted_o, halted) = signal::<Bit, DefaultClock>();
        let (rdata_o, rdata) = signal::<U<32>, DefaultClock>();
        let (haltreq_o, haltreq) = signal::<Bit, DefaultClock>();
        let (resumereq_o, resumereq) = signal::<Bit, DefaultClock>();
        let (regno_o, regno) = signal::<U<16>, DefaultClock>();
        let (wdata_o, wdata) = signal::<U<32>, DefaultClock>();
        let (we_o, we) = signal::<Bit, DefaultClock>();
        let done = std::rc::Rc::new(std::cell::RefCell::new(false));
        let d = done.clone();
        let rig = Rig {
            host: link.host,
            halted: halted_o,
            rdata: rdata_o,
            haltreq,
            resumereq,
            regno,
            wdata,
            we,
        };
        let body = client(rig);
        let mut dm = Dm::default();
        let mut sim = Running::new(join2(
            dm.run(
                (aw, ar, w, halted, rdata),
                (b, r, haltreq_o, resumereq_o, regno_o, wdata_o, we_o),
            ),
            async move {
                body.await;
                *d.borrow_mut() = true;
            },
        ));
        for _ in 0..4000 {
            sim.cycle();
            if *done.borrow() {
                return;
            }
        }
        panic!("the client did not finish");
    }

    fn bit(i: &In<Bit>) -> bool {
        i.get().to_bool()
    }

    #[test]
    fn inactive_it_reports_its_version_and_holds_nothing() {
        run(|rig| async move {
            let h = &rig.host;
            assert_eq!(
                read(h, at(DMSTATUS)).await & 0xff,
                AUTHENTICATED | VERSION
            );
            assert_eq!(read(h, at(DMCONTROL)).await, 0);
            assert_eq!(read(h, at(HARTINFO)).await, 0);
            assert_eq!(read(h, at(ABSTRACTCS)).await, 1, "one data word");
            // A halt request while inactive is not kept.
            write(h, at(DMCONTROL), HALTREQ).await;
            cycles(2).await;
            assert!(!bit(&rig.haltreq));
            assert_eq!(read(h, at(DMCONTROL)).await, 0);
        });
    }

    #[test]
    fn a_halt_request_reaches_the_core_and_its_halt_is_reported() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, at(DMCONTROL), DMACTIVE).await;
            let s = read(h, at(DMSTATUS)).await;
            assert_eq!(s & ALLRUNNING, ALLRUNNING, "running: {s:#x}");
            assert_eq!(s & ALLHALTED, 0);
            write(h, at(DMCONTROL), HALTREQ | DMACTIVE).await;
            cycles(2).await;
            assert!(bit(&rig.haltreq), "the request is on the line");
            assert_eq!(read(h, at(DMCONTROL)).await, HALTREQ | DMACTIVE);
            // The core halts, as the test says it does.
            rig.halted.set(Bit::One);
            cycles(2).await;
            let s = read(h, at(DMSTATUS)).await;
            assert_eq!(s & ALLHALTED, ALLHALTED, "halted: {s:#x}");
            assert_eq!(s & ALLRUNNING, 0);
            assert_eq!(read(h, at(HALTSUM0)).await, 1);
        });
    }

    #[test]
    fn a_register_is_read_and_written_by_the_abstract_command() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, at(DMCONTROL), HALTREQ | DMACTIVE).await;
            rig.halted.set(Bit::One);
            rig.rdata.set(U::from(0xc0ffee05u32));
            cycles(2).await;
            // A read of x5: the number goes to the core, the word
            // comes back into data0, and nothing is written.
            write(h, at(COMMAND), access(REGNO_GPR + 5, false)).await;
            let cs = read(h, at(ABSTRACTCS)).await;
            assert_eq!(cs >> 8 & 7, 0, "no error: {cs:#x}");
            assert_eq!(rig.regno.get().raw(), 0x1005);
            assert!(!bit(&rig.we));
            assert_eq!(read(h, at(DATA0)).await, 0xc0ffee05);
            assert_eq!(
                read(h, at(ABSTRACTCS)).await & (1 << 12),
                0,
                "not busy"
            );
            // A write of dpc: data0 first, then the command, and the
            // write pulse carries the word and the number.
            write(h, at(DATA0), 0x4000_0010).await;
            let (aw, _, w, b, _) = h;
            aw.send(LiteAw {
                addr: U::from(at(COMMAND)),
                prot: U::from(0u8),
            });
            w.send(LiteW {
                data: U::from(access(0x7b1, true)),
                strb: U::from(0xfu8),
            });
            let mut pulses = 0;
            let mut answered = false;
            for _ in 0..12 {
                DefaultClock::rising().await;
                if b.recv().is_some() {
                    answered = true;
                }
                if bit(&rig.we) {
                    pulses += 1;
                    assert_eq!(rig.regno.get().raw(), 0x7b1);
                    assert_eq!(rig.wdata.get().raw(), 0x4000_0010);
                }
            }
            assert!(answered);
            assert_eq!(pulses, 1, "one write pulse");
            assert_eq!(read(h, at(ABSTRACTCS)).await >> 8 & 7, 0);
        });
    }

    #[test]
    fn an_unsupported_command_or_a_running_core_is_an_error_that_sticks() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, at(DMCONTROL), DMACTIVE).await;
            // Running: cmderr 4.
            write(h, at(COMMAND), access(REGNO_GPR + 1, false)).await;
            assert_eq!(read(h, at(ABSTRACTCS)).await >> 8 & 7, 4);
            // The error stands, and a good command is ignored meanwhile.
            rig.halted.set(Bit::One);
            rig.rdata.set(U::from(77u32));
            cycles(2).await;
            write(h, at(COMMAND), access(REGNO_GPR + 1, false)).await;
            cycles(4).await;
            assert_eq!(read(h, at(DATA0)).await, 0, "ignored");
            // Cleared by writing the bits, and then the command runs.
            write(h, at(ABSTRACTCS), 7 << 8).await;
            assert_eq!(read(h, at(ABSTRACTCS)).await >> 8 & 7, 0);
            write(h, at(COMMAND), access(REGNO_GPR + 1, false)).await;
            cycles(2).await;
            assert_eq!(read(h, at(DATA0)).await, 77);
            // Unsupported: a 64-bit access, a write of a CSR that is
            // not dpc or dcsr, a command of another type.
            write(h, at(COMMAND), access(REGNO_GPR + 1, false) | 1 << 20).await;
            assert_eq!(read(h, at(ABSTRACTCS)).await >> 8 & 7, 2);
            write(h, at(ABSTRACTCS), 7 << 8).await;
            write(h, at(COMMAND), access(0x300, true)).await;
            assert_eq!(read(h, at(ABSTRACTCS)).await >> 8 & 7, 2);
            write(h, at(ABSTRACTCS), 7 << 8).await;
            write(h, at(COMMAND), 1 << 24).await;
            assert_eq!(read(h, at(ABSTRACTCS)).await >> 8 & 7, 2);
        });
    }

    /// The debugger writes `resumereq` again in the very cycle the core
    /// leaves debug mode from the last request. The core is running as
    /// the write lands, so the write asks nothing; but it clears the
    /// acknowledgement, and the acknowledgement of the resume that
    /// completed in that same cycle must not stand over it. The two
    /// arms fire in one cycle, and the one written last in the module
    /// wins, so this is the case the order is for.
    #[test]
    fn a_resume_request_written_as_the_core_resumes_clears_the_ack() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, at(DMCONTROL), HALTREQ | DMACTIVE).await;
            rig.halted.set(Bit::One);
            cycles(2).await;
            write(h, at(DMCONTROL), DMACTIVE).await;
            write(h, at(DMCONTROL), RESUMEREQ | DMACTIVE).await;
            cycles(2).await;
            assert!(bit(&rig.resumereq));
            // The write goes out, and the core resumes in the same
            // cycle the module takes it.
            let (aw, _, w, b, _) = h;
            aw.send(LiteAw {
                addr: U::from(at(DMCONTROL)),
                prot: U::from(0u8),
            });
            w.send(LiteW {
                data: U::from(RESUMEREQ | DMACTIVE),
                strb: U::from(0xfu8),
            });
            rig.halted.set(Bit::Zero);
            loop {
                DefaultClock::rising().await;
                if b.recv().is_some() {
                    break;
                }
            }
            cycles(2).await;
            assert!(
                !bit(&rig.resumereq),
                "a request to a running core asks nothing"
            );
            let s = read(h, at(DMSTATUS)).await;
            assert_eq!(
                s & ALLRESUMEACK,
                0,
                "the write cleared the ack: {s:#x}"
            );
        });
    }

    /// `data0` written while a command is in flight: the specification
    /// makes it a busy error, and the answer, not the write, is what
    /// `data0` holds afterwards.
    #[test]
    fn data0_written_while_a_command_is_in_flight_is_a_busy_error() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, at(DMCONTROL), HALTREQ | DMACTIVE).await;
            rig.halted.set(Bit::One);
            rig.rdata.set(U::from(0x1234_5678u32));
            cycles(2).await;
            // The command, and the write of data0 right behind it, so
            // the write lands while the command is in flight.
            let (aw, _, w, b, _) = h;
            aw.send(LiteAw {
                addr: U::from(at(COMMAND)),
                prot: U::from(0u8),
            });
            w.send(LiteW {
                data: U::from(access(REGNO_GPR + 1, false)),
                strb: U::from(0xfu8),
            });
            DefaultClock::rising().await;
            let _ = b.recv();
            aw.send(LiteAw {
                addr: U::from(at(DATA0)),
                prot: U::from(0u8),
            });
            w.send(LiteW {
                data: U::from(0xdead_beefu32),
                strb: U::from(0xfu8),
            });
            let mut answered = 0;
            for _ in 0..8 {
                DefaultClock::rising().await;
                if b.recv().is_some() {
                    answered += 1;
                }
            }
            assert!(answered >= 1, "the writes were answered");
            assert_eq!(read(h, at(ABSTRACTCS)).await >> 8 & 7, 1, "busy");
            write(h, at(ABSTRACTCS), 7 << 8).await;
            assert_eq!(read(h, at(DATA0)).await, 0x1234_5678, "the answer");
        });
    }

    #[test]
    fn a_resume_request_drops_when_the_core_resumes_and_is_acknowledged() {
        run(|rig| async move {
            let h = &rig.host;
            write(h, at(DMCONTROL), HALTREQ | DMACTIVE).await;
            rig.halted.set(Bit::One);
            cycles(2).await;
            write(h, at(DMCONTROL), DMACTIVE).await;
            cycles(2).await;
            assert!(!bit(&rig.haltreq), "the request is withdrawn");
            write(h, at(DMCONTROL), RESUMEREQ | DMACTIVE).await;
            cycles(2).await;
            assert!(bit(&rig.resumereq));
            let s = read(h, at(DMSTATUS)).await;
            assert_eq!(s & ALLRESUMEACK, 0, "not yet: {s:#x}");
            rig.halted.set(Bit::Zero);
            cycles(3).await;
            assert!(!bit(&rig.resumereq), "dropped once the core ran");
            let s = read(h, at(DMSTATUS)).await;
            assert_eq!(s & ALLRESUMEACK, ALLRESUMEACK, "acknowledged: {s:#x}");
            assert_eq!(s & ALLRUNNING, ALLRUNNING);
        });
    }
}
