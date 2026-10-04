// SPDX-License-Identifier: Apache-2.0
//! Sv32, the RISC-V page-based virtual memory of thirty-two bits:
//! `Mmu<E>`, two translation lookaside buffers of `E` entries each,
//! one for fetches and one for data, and the walker that fills them
//! (issue 1014, item M7 of #279). It is built as issue 1009's note,
//! `docs/sv32-timing.md`, decided.
//!
//! The core asks with a virtual address it already holds in a
//! register, and the answer is a register too: a cycle after the
//! request, the physical address, or a page fault, or an access fault.
//! A miss answers none of the three; the walker reads the page table
//! and fills an entry, and the answer comes a cycle after the fill.
//! The core holds its request, the address and the request bit both
//! steady, until one of the three comes. An answer always belongs to
//! the request of the cycle before it, so if the core changes the
//! address or drops the request in the meantime, the answer it sees
//! next is for the old one, and it ignores it.
//!
//! * Translation is on when `satp.MODE` is set and the hart is not in
//!   machine mode. Otherwise the address passes through, with no check
//!   but with the same latency of one cycle, so the core has one rule.
//! * A request in the cycle of a flush is not answered: the core holds
//!   it, and it is answered from the emptied TLBs, by a walk.
//! * Pages are four kilobytes, and a leaf at the first level is a four
//!   megabyte megapage. Each TLB is fully associative and replaced in
//!   turn.
//! * The walker never writes. An entry whose accessed bit is clear, or
//!   whose dirty bit is clear on a store, is a page fault, and the
//!   kernel sets the bit (Svade).
//! * The checks are on a hit: read, write and execute against the
//!   access, the user bit against the mode with `SUM`, and `MXR`.
//! * A flush, on a write of `satp` or an `sfence.vma`, empties both.
//!   Address space identifiers are not kept, which the specification
//!   allows, so `satp.ASID` is ignored.
//! * Physical addresses are thirty-two bits, the board's. A page table
//!   entry or a `satp` whose page number reaches past them is an access
//!   fault, as an address where nothing is.
//!
//! [`translate`] is the reference: the specification's walk, with no
//! TLB, which the unit must agree with whenever the tables have not
//! changed since the last flush.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Regs, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

/// A fetch's request: whether there is one, and its virtual address.
/// The core holds both steady until the answer comes (issue 1122).
#[derive(ValueDerive, Clone, Copy, Default, PartialEq, Debug)]
pub struct IReq {
    /// A fetch is asked for.
    pub req: Bit,
    /// Its virtual address.
    pub va: U<32>,
}

/// A data access's request: whether there is one, its virtual address,
/// and whether it is a store.
#[derive(ValueDerive, Clone, Copy, Default, PartialEq, Debug)]
pub struct DReq {
    /// An access is asked for.
    pub req: Bit,
    /// Its virtual address.
    pub va: U<32>,
    /// It is a store, not a load.
    pub store: Bit,
}

/// A port's answer, a cycle after its request: at most one of ok,
/// fault and err is set, and none on a miss.
#[derive(ValueDerive, Clone, Copy, Default, PartialEq, Debug)]
pub struct Res {
    /// The access is allowed, at pa.
    pub ok: Bit,
    /// The physical address.
    pub pa: U<32>,
    /// A page fault.
    pub fault: Bit,
    /// An access fault.
    pub err: Bit,
}

/// The walker's answer: a page table entry, or that its read failed.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Pte {
    /// The entry read.
    pub data: U<32>,
    /// The read failed: nothing answers at its address.
    pub err: Bit,
}

// begin{state}
/// Two TLBs of `E` entries each, `E` from 1 to 16, and the walker that
/// fills them.
#[derive(Trace, Default)]
pub struct Mmu<const E: usize> {
    /// Whether each fetch entry holds a translation.
    pub ivalid: Regs<Bit, E>,
    /// Whether each fetch entry is a megapage.
    pub imega: Regs<Bit, E>,
    /// Each fetch entry's virtual page number.
    pub ivpn: Regs<U<20>, E>,
    /// Each fetch entry's physical page number.
    pub ippn: Regs<U<20>, E>,
    /// Each fetch entry's low byte of its page table entry: the
    /// permissions, with the accessed and dirty bits.
    pub iperm: Regs<U<8>, E>,
    /// The fetch entry to fill next.
    pub inext: Reg<U<4>>,
    /// Whether each data entry holds a translation.
    pub dvalid: Regs<Bit, E>,
    /// Whether each data entry is a megapage.
    pub dmega: Regs<Bit, E>,
    /// Each data entry's virtual page number.
    pub dvpn: Regs<U<20>, E>,
    /// Each data entry's physical page number.
    pub dppn: Regs<U<20>, E>,
    /// Each data entry's low byte of its page table entry.
    pub dperm: Regs<U<8>, E>,
    /// The data entry to fill next.
    pub dnext: Reg<U<4>>,
    /// A walk is under way.
    pub busy: Reg<Bit>,
    /// The walk is for the data port, not the fetch port.
    pub data: Reg<Bit>,
    /// The walk is at the first level, `satp`'s page.
    pub first: Reg<Bit>,
    /// The read of the entry at `addr` has been sent.
    pub asked: Reg<Bit>,
    /// A flush came during the walk, so what it finds is dropped.
    pub drop: Reg<Bit>,
    /// The page being walked.
    pub vpn: Reg<U<20>>,
    /// The address of the entry the walk reads next.
    pub addr: Reg<U<32>>,
    /// A walk for the fetch port ended in a fault. It answers while
    /// that port asks for that page, which the core does until it
    /// takes the trap.
    pub ifault: Reg<Bit>,
    /// That fault was an access fault, not a page fault.
    pub ierr: Reg<Bit>,
    /// The page that fault was for.
    pub ifvpn: Reg<U<20>>,
    /// A walk for the data port ended in a fault.
    pub dfault: Reg<Bit>,
    /// That fault was an access fault.
    pub derr: Reg<Bit>,
    /// The page that fault was for.
    pub dfvpn: Reg<U<20>>,
    /// The fetch port's answer, a cycle after its request: allowed.
    pub iok: Reg<Bit>,
    /// The fetch's physical address.
    pub ipa: Reg<U<32>>,
    /// The fetch is a page fault.
    pub ipf: Reg<Bit>,
    /// The fetch is an access fault.
    pub iaf: Reg<Bit>,
    /// The data port's answer, a cycle after its request: allowed.
    pub dok: Reg<Bit>,
    /// The access's physical address.
    pub dpa: Reg<U<32>>,
    /// The access is a page fault.
    pub dpf: Reg<Bit>,
    /// The access is an access fault.
    pub daf: Reg<Bit>,
}
// end{state}

/// Eight entries in each TLB, as the note sizes them.
pub type Mmu8 = Mmu<8>;

// begin{check}
/// Whether an access may use a page whose entry's low byte is `perm`:
/// execute for a fetch, write and dirty for a store, read (or execute
/// under `MXR`) for a load; a user page only from user mode, and from
/// supervisor mode only for data under `SUM`; and the accessed bit
/// always.
#[lower]
fn allowed(
    perm: U<8>,
    fetch: Bit,
    store: Bit,
    user: Bit,
    sum: Bit,
    mxr: Bit,
) -> Bit {
    let (r, w, x) = (perm.bit(1), perm.bit(2), perm.bit(3));
    let (u, a, d) = (perm.bit(4), perm.bit(6), perm.bit(7));
    let mode = mux(user, u, !u | (sum & !fetch));
    let kind = mux(fetch, x, mux(store, w & d, r | (x & mxr)));
    mode & kind & a
}
// end{check}

// begin{ports}
// The lowering reads a loop over an array as `ivalid[i]`, so the index
// is what it is written with, and Clippy would rather it were an
// iterator.
#[allow(clippy::needless_range_loop)]
#[lower]
impl<const E: usize> Unit for Mmu<E> {
    async fn run(
        &mut self,
        (rst, satp, prv, sum, mxr, flush, ireq, dreq, pte): (
            In<Bit>,
            In<U<32>>,
            In<U<2>>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<IReq>,
            In<DReq>,
            Rx<Pte>,
        ),
        (ires, dres, ptw): (Out<Res>, Out<Res>, Tx<U<32>>),
    ) {
        loop {
            DefaultClock::rising().await;
            // end{ports}
            // begin{lookup}
            let (rst, flush) = (rst.get(), flush.get());
            let (satp, prv) = (satp.get(), prv.get());
            let (sum, mxr) = (sum.get(), mxr.get());
            let (iq, dq) = (ireq.get(), dreq.get());
            let (ireq, iva) = (iq.req, iq.va);
            let (dreq, dva, store) = (dq.req, dq.va, dq.store);
            // Translation is on outside machine mode when `satp` says.
            let vm = satp.bit(31) & Bit::from(prv != 3);
            let user = Bit::from(prv == 0);
            let ivq = iva.slice::<12, 20>();
            let dvq = dva.slice::<12, 20>();
            // Every entry is compared at once. A megapage compares the
            // upper ten bits of the page number only; at most one entry
            // matches, since an entry is filled only on a miss.
            let z20 = U::<20>::from(0u32);
            let z8 = U::<8>::from(0u8);
            let mut ihit = Bit::Zero;
            let mut ippn = z20;
            let mut iperm = z8;
            let mut imega = Bit::Zero;
            let mut dhit = Bit::Zero;
            let mut dppn = z20;
            let mut dperm = z8;
            let mut dmega = Bit::Zero;
            for i in 0..E {
                let t = self.ivpn[i].get();
                let mg = self.imega[i].get();
                let m = self.ivalid[i].get()
                    & Bit::from(t.slice::<10, 10>() == ivq.slice::<10, 10>())
                    & (mg
                        | Bit::from(
                            t.slice::<0, 10>() == ivq.slice::<0, 10>(),
                        ));
                ihit = ihit | m;
                ippn = mux(m, self.ippn[i].get(), ippn);
                iperm = mux(m, self.iperm[i].get(), iperm);
                imega = mux(m, mg, imega);
                let t = self.dvpn[i].get();
                let mg = self.dmega[i].get();
                let m = self.dvalid[i].get()
                    & Bit::from(t.slice::<10, 10>() == dvq.slice::<10, 10>())
                    & (mg
                        | Bit::from(
                            t.slice::<0, 10>() == dvq.slice::<0, 10>(),
                        ));
                dhit = dhit | m;
                dppn = mux(m, self.dppn[i].get(), dppn);
                dperm = mux(m, self.dperm[i].get(), dperm);
                dmega = mux(m, mg, dmega);
            }
            // A megapage's physical page takes the low ten bits of the
            // virtual one, which its own entry has as zero.
            let ippn =
                ippn | mux(imega, ivq.slice::<0, 10>().zext::<20>(), z20);
            let dppn =
                dppn | mux(dmega, dvq.slice::<0, 10>().zext::<20>(), z20);
            let ipa = ippn.concat::<_, 32>(iva.slice::<0, 12>());
            let dpa = dppn.concat::<_, 32>(dva.slice::<0, 12>());
            let iallow = allowed(iperm, Bit::One, Bit::Zero, user, sum, mxr);
            let dallow = allowed(dperm, Bit::Zero, store, user, sum, mxr);
            // A walk's fault answers for the page it was for.
            let ifault = self.ifault.get() & Bit::from(self.ifvpn.get() == ivq);
            let dfault = self.dfault.get() & Bit::from(self.dfvpn.get() == dvq);
            let (ierr, derr) = (self.ierr.get(), self.derr.get());
            // The answers, registered: each port sees this cycle's a
            // cycle from now. A flush this cycle answers nothing.
            let iask = ireq & !flush;
            let dask = dreq & !flush;
            with!(self <= {
                iok: iask & (!vm | (ihit & iallow)),
                ipa: mux(vm, ipa, iva),
                ipf: iask & vm & ((ihit & !iallow) | (!ihit & ifault & !ierr)),
                iaf: iask & vm & !ihit & ifault & ierr,
                dok: dask & (!vm | (dhit & dallow)),
                dpa: mux(vm, dpa, dva),
                dpf: dask & vm & ((dhit & !dallow) | (!dhit & dfault & !derr)),
                daf: dask & vm & !dhit & dfault & derr,
                rst ? {
                    iok: Bit::Zero,
                    ipf: Bit::Zero,
                    iaf: Bit::Zero,
                    dok: Bit::Zero,
                    dpf: Bit::Zero,
                    daf: Bit::Zero,
                },
            });
            ires.set(Res {
                ok: self.iok.get(),
                pa: self.ipa.get(),
                fault: self.ipf.get(),
                err: self.iaf.get(),
            });
            dres.set(Res {
                ok: self.dok.get(),
                pa: self.dpa.get(),
                fault: self.dpf.get(),
                err: self.daf.get(),
            });
            // end{lookup}
            // begin{walk}
            // A miss with no fault on record starts a walk when the
            // walker is free; the data port first, since its
            // instruction is the older.
            let imiss = ireq & vm & !ihit & !ifault;
            let dmiss = dreq & vm & !dhit & !dfault;
            let busy = self.busy.get();
            let start = !busy & !flush & (imiss | dmiss);
            let sdata = dmiss;
            let svpn = mux(dmiss, dvq, ivq);
            // The first level's entry: `satp`'s page, indexed by the
            // upper ten bits. A page past thirty-two bits is an access
            // fault at once.
            let root = satp.slice::<0, 22>();
            let root_far = Bit::from(root.slice::<20, 2>() != 0u32);
            let l1 = root.slice::<0, 20>().concat::<_, 32>(
                svpn.slice::<10, 10>().concat::<_, 12>(U::<2>::from(0u8)),
            );
            let go = start & !root_far;
            // The read goes out once, when the link has room.
            let asked = self.asked.get();
            let send = busy & !asked & ptw.ready();
            if send.to_bool() {
                ptw.send(self.addr.get());
            }
            // The answer.
            // Whether an answer is here is asked with `peek`, not with
            // what `recv_if` returns, which lowers to the data (#1079).
            let ans = Bit::from(pte.peek().is_some()) & busy & asked;
            let _ = pte.recv_if(ans);
            let e = pte.head();
            let p = e.data;
            let first = self.first.get();
            let wdata = self.data.get();
            let wvpn = self.vpn.get();
            let (v, r, w, x) = (p.bit(0), p.bit(1), p.bit(2), p.bit(3));
            let leaf = r | x;
            // Invalid, writable but not readable, a pointer at the
            // second level, or a megapage whose low page number is not
            // zero: a page fault. A page past thirty-two bits: an
            // access fault.
            let bad = !v
                | (!r & w)
                | (!leaf & !first)
                | (leaf & first & Bit::from(p.slice::<10, 10>() != 0u32));
            let far = Bit::from(p.slice::<30, 2>() != 0u32);
            let ok = ans & !e.err & !bad & !far;
            let descend = ok & !leaf;
            let keep = !self.drop.get() & !flush;
            let fill = ok & leaf & keep;
            let fault = ans & (e.err | bad | far) & keep;
            // A leaf past thirty-two bits is a page fault if the access
            // would not be allowed anyway, since translation fails
            // before the access is made, and an access fault if it
            // would.
            let wok = allowed(
                p.slice::<0, 8>(),
                !wdata,
                wdata & store,
                user,
                sum,
                mxr,
            );
            let fault_err = e.err | (!bad & far & (!leaf | wok));
            let next = p.slice::<10, 20>().concat::<_, 32>(
                wvpn.slice::<0, 10>().concat::<_, 12>(U::<2>::from(0u8)),
            );
            // The record of a fault: from the walk's answer, or from a
            // `satp` past thirty-two bits at the start.
            let ifset = (fault & !wdata) | (start & root_far & !sdata);
            let dfset = (fault & wdata) | (start & root_far & sdata);
            let rec_err = fault_err | start;
            let rec_vpn = mux(start, svpn, wvpn);
            // end{walk}
            // begin{fill}
            // A leaf fills the next entry of its port's TLB.
            let ifill = fill & !wdata;
            let dfill = fill & wdata;
            let (inext, dnext) = (self.inext.get(), self.dnext.get());
            let mega = first;
            let perm = p.slice::<0, 8>();
            let ppn = p.slice::<10, 20>();
            for i in 0..E {
                let iat = ifill & Bit::from(inext == i);
                let dat = dfill & Bit::from(dnext == i);
                with!(self <= {
                    iat ? {
                        ivalid[i]: Bit::One,
                        imega[i]: mega,
                        ivpn[i]: wvpn,
                        ippn[i]: ppn,
                        iperm[i]: perm,
                    },
                    dat ? {
                        dvalid[i]: Bit::One,
                        dmega[i]: mega,
                        dvpn[i]: wvpn,
                        dppn[i]: ppn,
                        dperm[i]: perm,
                    },
                    flush | rst ? {
                        ivalid[i]: Bit::Zero,
                        dvalid[i]: Bit::Zero,
                    },
                });
            }
            let last = U::<4>::from(E - 1);
            let zero4 = U::<4>::from(0u8);
            with!(self <= {
                ifill ? inext: mux(inext == last, zero4, inext + 1),
                dfill ? dnext: mux(dnext == last, zero4, dnext + 1),
                rst ? { inext: zero4, dnext: zero4 },
            });
            // end{fill}
            // begin{drives}
            with!(self <= {
                send ? asked: Bit::One,
                descend ? {
                    first: Bit::Zero,
                    asked: Bit::Zero,
                    addr: next,
                },
                ans & !descend ? { busy: Bit::Zero, drop: Bit::Zero },
                flush & busy ? drop: Bit::One,
                go ? {
                    busy: Bit::One,
                    data: sdata,
                    first: Bit::One,
                    asked: Bit::Zero,
                    drop: Bit::Zero,
                    vpn: svpn,
                    addr: l1,
                },
                // A record lasts while its port asks for its page, so a
                // later request walks again.
                !(ireq & ifault) ? ifault: Bit::Zero,
                !(dreq & dfault) ? dfault: Bit::Zero,
                ifset ? { ifault: Bit::One, ierr: rec_err, ifvpn: rec_vpn },
                dfset ? { dfault: Bit::One, derr: rec_err, dfvpn: rec_vpn },
                flush ? { ifault: Bit::Zero, dfault: Bit::Zero },
                rst ? {
                    busy: Bit::Zero,
                    asked: Bit::Zero,
                    drop: Bit::Zero,
                    ifault: Bit::Zero,
                    dfault: Bit::Zero,
                },
            });
            // end{drives}
        }
    }
}

/// What an access is, for [`translate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// An instruction fetch.
    Fetch,
    /// A load.
    Load,
    /// A store, or an atomic memory operation.
    Store,
}

/// Why a translation failed: a page fault (cause 12, 13 or 15) or an
/// access fault (1, 5 or 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// A page fault.
    Page,
    /// An access fault.
    Access,
}

/// The hart's state a translation depends on.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mode {
    /// The `satp` register.
    pub satp: u32,
    /// The privilege: 0 user, 1 supervisor, 3 machine.
    pub prv: u8,
    /// `sstatus.SUM`: supervisor mode may read and write user pages.
    pub sum: bool,
    /// `sstatus.MXR`: an executable page may be read.
    pub mxr: bool,
}

/// The specification's translation of `va`, with no TLB: the walk of
/// section 4.3.2 of the privileged specification for Sv32, with A and
/// D by page fault and physical addresses of thirty-two bits. `read`
/// answers a word of memory, or `None` where nothing is.
pub fn translate(
    read: impl Fn(u32) -> Option<u32>,
    m: Mode,
    va: u32,
    access: Access,
) -> Result<u32, Fault> {
    if m.satp >> 31 == 0 || m.prv == 3 {
        return Ok(va);
    }
    let vpn = [(va >> 12) & 0x3ff, va >> 22];
    let mut table = u64::from(m.satp & 0x3f_ffff) << 12;
    let mut level = 1;
    loop {
        let at = table + u64::from(vpn[level]) * 4;
        let at = u32::try_from(at).map_err(|_| Fault::Access)?;
        let p = read(at).ok_or(Fault::Access)?;
        let (v, r, w, x) = (p & 1, p >> 1 & 1, p >> 2 & 1, p >> 3 & 1);
        if v == 0 || (r == 0 && w == 1) {
            return Err(Fault::Page);
        }
        let ppn = u64::from(p >> 10);
        if r == 0 && x == 0 {
            if level == 0 {
                return Err(Fault::Page);
            }
            table = ppn << 12;
            level = 0;
            continue;
        }
        let (u, a, d) = (p >> 4 & 1, p >> 6 & 1, p >> 7 & 1);
        let user = m.prv == 0;
        let mode = if user {
            u == 1
        } else {
            u == 0 || (m.sum && access != Access::Fetch)
        };
        let kind = match access {
            Access::Fetch => x == 1,
            Access::Store => w == 1 && d == 1,
            Access::Load => r == 1 || (x == 1 && m.mxr),
        };
        if level == 1 && ppn & 0x3ff != 0 {
            return Err(Fault::Page);
        }
        if !mode || !kind || a == 0 {
            return Err(Fault::Page);
        }
        let page = if level == 1 {
            ppn | u64::from(vpn[0])
        } else {
            ppn
        };
        let pa = page << 12 | u64::from(va & 0xfff);
        return u32::try_from(pa).map_err(|_| Fault::Access);
    }
}

/// The bits of a page table entry.
pub mod pte {
    /// Valid.
    pub const V: u32 = 1;
    /// Readable.
    pub const R: u32 = 2;
    /// Writable.
    pub const W: u32 = 4;
    /// Executable.
    pub const X: u32 = 8;
    /// A user page.
    pub const U: u32 = 16;
    /// Global, which this unit ignores.
    pub const G: u32 = 32;
    /// Accessed.
    pub const A: u32 = 64;
    /// Dirty.
    pub const D: u32 = 128;

    /// An entry for the page or table at physical address `pa`.
    pub const fn to(pa: u32, bits: u32) -> u32 {
        (pa >> 12) << 10 | bits
    }
}

/// `satp` with translation on, its tables at physical address `root`.
pub const fn satp(root: u32) -> u32 {
    1 << 31 | root >> 12
}

#[cfg(test)]
mod tests;
