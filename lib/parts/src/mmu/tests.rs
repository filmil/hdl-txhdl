// SPDX-License-Identifier: Apache-2.0
//! The unit against the reference: page tables in a map, a memory that
//! answers the walker's reads after a few cycles, and requests on both
//! ports, each answer checked against [`translate`].
use super::pte::{A, D, R, U as UB, V, W, X};
use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use txhdl::comp::{chan, join2, signal, Running};

type Mem = Rc<RefCell<HashMap<u32, u32>>>;

/// What a test holds: the unit's inputs, its answers, the memory
/// behind the walker and what it was asked.
struct Rig {
    satp: Out<U<32>>,
    prv: Out<U<2>>,
    sum: Out<Bit>,
    mxr: Out<Bit>,
    flush: Out<Bit>,
    ireq: Out<Bit>,
    iva: Out<U<32>>,
    dreq: Out<Bit>,
    dva: Out<U<32>>,
    store: Out<Bit>,
    iok: In<Bit>,
    ipa: In<U<32>>,
    ifault: In<Bit>,
    ierr: In<Bit>,
    dok: In<Bit>,
    dpa: In<U<32>>,
    dfault: In<Bit>,
    derr: In<Bit>,
    mem: Mem,
    reads: Rc<RefCell<Vec<u32>>>,
    latency: Rc<Cell<usize>>,
    mode: Cell<Mode>,
}

/// Which port asks.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Port {
    I,
    D,
}

async fn cycles(n: usize) {
    for _ in 0..n {
        DefaultClock::rising().await;
    }
}

impl Rig {
    fn set_mode(&self, m: Mode) {
        self.satp.set(U::<32>::from(m.satp));
        self.prv.set(U::<2>::from(m.prv));
        self.sum.set(Bit::from_bool(m.sum));
        self.mxr.set(Bit::from_bool(m.mxr));
        self.mode.set(m);
    }

    fn put(&self, at: u32, v: u32) {
        self.mem.borrow_mut().insert(at, v);
    }

    fn reads(&self) -> usize {
        self.reads.borrow().len()
    }

    /// A flush, for one cycle.
    async fn flush(&self) {
        self.flush.set(Bit::One);
        cycles(1).await;
        self.flush.set(Bit::Zero);
    }

    /// One request, held until the unit answers, and the cycles it took
    /// to answer: 2 for a hit, the request's cycle and the answer's.
    async fn ask(
        &self,
        port: Port,
        va: u32,
        store: bool,
    ) -> (Result<u32, Fault>, usize) {
        let (req, addr) = match port {
            Port::I => (&self.ireq, &self.iva),
            Port::D => (&self.dreq, &self.dva),
        };
        req.set(Bit::One);
        addr.set(U::<32>::from(va));
        self.store.set(Bit::from_bool(store));
        for n in 1..400 {
            DefaultClock::rising().await;
            let (ok, pa, fault, err) = match port {
                Port::I => (&self.iok, &self.ipa, &self.ifault, &self.ierr),
                Port::D => (&self.dok, &self.dpa, &self.dfault, &self.derr),
            };
            let got = if ok.get().to_bool() {
                Some(Ok(pa.get().raw() as u32))
            } else if fault.get().to_bool() {
                Some(Err(Fault::Page))
            } else if err.get().to_bool() {
                Some(Err(Fault::Access))
            } else {
                None
            };
            if let Some(got) = got {
                req.set(Bit::Zero);
                cycles(1).await;
                return (got, n);
            }
        }
        panic!("no answer for {va:#x} on {port:?}");
    }

    /// What the reference says of the same request.
    fn model(&self, port: Port, va: u32, store: bool) -> Result<u32, Fault> {
        let access = match (port, store) {
            (Port::I, _) => Access::Fetch,
            (Port::D, false) => Access::Load,
            (Port::D, true) => Access::Store,
        };
        let mem = self.mem.borrow();
        translate(|a| mem.get(&a).copied(), self.mode.get(), va, access)
    }

    /// A request, checked against the reference.
    async fn check(
        &self,
        port: Port,
        va: u32,
        store: bool,
    ) -> Result<u32, Fault> {
        let want = self.model(port, va, store);
        let (got, _) = self.ask(port, va, store).await;
        assert_eq!(
            got,
            want,
            "{port:?} {va:#x} store={store} {:?}",
            self.mode.get()
        );
        got
    }
}

/// Run `client` against an eight-entry unit and a memory whose reads
/// answer after `latency` cycles, until it is done.
fn run<F>(client: impl FnOnce(Rig) -> F)
where
    F: std::future::Future<Output = ()>,
{
    let (rst_o, rst) = signal::<Bit, DefaultClock>();
    let (satp_o, satp) = signal::<U<32>, DefaultClock>();
    let (prv_o, prv) = signal::<U<2>, DefaultClock>();
    let (sum_o, sum) = signal::<Bit, DefaultClock>();
    let (mxr_o, mxr) = signal::<Bit, DefaultClock>();
    let (flush_o, flush) = signal::<Bit, DefaultClock>();
    let (ireq_o, ireq) = signal::<Bit, DefaultClock>();
    let (iva_o, iva) = signal::<U<32>, DefaultClock>();
    let (dreq_o, dreq) = signal::<Bit, DefaultClock>();
    let (dva_o, dva) = signal::<U<32>, DefaultClock>();
    let (store_o, store) = signal::<Bit, DefaultClock>();
    let (iok_o, iok) = signal::<Bit, DefaultClock>();
    let (ipa_o, ipa) = signal::<U<32>, DefaultClock>();
    let (ifault_o, ifault) = signal::<Bit, DefaultClock>();
    let (ierr_o, ierr) = signal::<Bit, DefaultClock>();
    let (dok_o, dok) = signal::<Bit, DefaultClock>();
    let (dpa_o, dpa) = signal::<U<32>, DefaultClock>();
    let (dfault_o, dfault) = signal::<Bit, DefaultClock>();
    let (derr_o, derr) = signal::<Bit, DefaultClock>();
    let (ptw_tx, ptw_rx) = chan::<U<32>, DefaultClock>();
    let (pte_tx, pte_rx) = chan::<Pte, DefaultClock>();
    let mem: Mem = Rc::default();
    let reads = Rc::new(RefCell::new(Vec::new()));
    let latency = Rc::new(Cell::new(2));
    let rig = Rig {
        satp: satp_o,
        prv: prv_o,
        sum: sum_o,
        mxr: mxr_o,
        flush: flush_o,
        ireq: ireq_o,
        iva: iva_o,
        dreq: dreq_o,
        dva: dva_o,
        store: store_o,
        iok,
        ipa,
        ifault,
        ierr,
        dok,
        dpa,
        dfault,
        derr,
        mem: mem.clone(),
        reads: reads.clone(),
        latency: latency.clone(),
        mode: Cell::new(Mode::default()),
    };
    let body = client(rig);
    // The memory: one read at a time, answered after the latency, an
    // address that holds nothing answering as an error.
    let memory = async move {
        loop {
            let at = ptw_rx.wait().await.raw() as u32;
            reads.borrow_mut().push(at);
            cycles(latency.get()).await;
            let word = mem.borrow().get(&at).copied();
            pte_tx
                .put(|| Pte {
                    data: U::<32>::from(word.unwrap_or(0)),
                    err: Bit::from_bool(word.is_none()),
                })
                .await;
        }
    };
    let done = Rc::new(Cell::new(false));
    let d = done.clone();
    let mut mmu = Mmu8::default();
    let hardware = mmu.run(
        (
            rst, satp, prv, sum, mxr, flush, ireq, iva, dreq, dva, store,
            pte_rx,
        ),
        (
            iok_o, ipa_o, ifault_o, ierr_o, dok_o, dpa_o, dfault_o, derr_o,
            ptw_tx,
        ),
    );
    // The client first: it drives the request wires, which the unit
    // reads in the same step.
    let mut sim = Running::new(join2(
        join2(
            async move {
                // Past the reset first, and the answer registers' reset.
                cycles(2).await;
                body.await;
                d.set(true);
            },
            hardware,
        ),
        memory,
    ));
    rst_o.set(Bit::One);
    sim.cycle();
    rst_o.set(Bit::Zero);
    for _ in 0..200_000 {
        sim.cycle();
        if done.get() {
            return;
        }
    }
    panic!("the client did not finish");
}

/// The tables the tests share: the root at `ROOT`, its entry for the
/// first four megabytes pointing at `L0`.
const ROOT: u32 = 0x8000_0000;
const L0: u32 = 0x8000_1000;

/// The first level's entry for `va`.
fn l1(va: u32) -> u32 {
    ROOT + (va >> 22) * 4
}

/// The second level's entry for `va`, in the table at `table`.
fn l0(table: u32, va: u32) -> u32 {
    table + ((va >> 12) & 0x3ff) * 4
}

/// Supervisor mode with translation on.
fn smode() -> Mode {
    Mode {
        satp: satp(ROOT),
        prv: 1,
        sum: false,
        mxr: false,
    }
}

/// A page at `va` mapped to `pa` with `bits`, through `L0`.
fn map(rig: &Rig, va: u32, pa: u32, bits: u32) {
    rig.put(l1(va), pte::to(L0, V));
    rig.put(l0(L0, va), pte::to(pa, bits));
}

const RWXAD: u32 = V | R | W | X | A | D;

#[test]
fn bare_and_machine_mode_pass_the_address_through_a_cycle_later() {
    run(|rig| async move {
        let va = 0x1234_5678;
        rig.set_mode(Mode::default());
        assert_eq!(rig.ask(Port::I, va, false).await, (Ok(va), 2));
        assert_eq!(rig.ask(Port::D, va, true).await, (Ok(va), 2));
        // Machine mode is untranslated whatever `satp` says.
        rig.set_mode(Mode { prv: 3, ..smode() });
        assert_eq!(rig.ask(Port::D, va, false).await, (Ok(va), 2));
        assert_eq!(rig.reads(), 0, "no walk");
    });
}

#[test]
fn a_page_is_walked_once_and_then_hits_a_cycle_later() {
    run(|rig| async move {
        rig.set_mode(smode());
        map(&rig, 0x0040_1000, 0x4123_4000, RWXAD);
        let (got, n) = rig.ask(Port::D, 0x0040_1abc, false).await;
        assert_eq!(got, Ok(0x4123_4abc));
        assert_eq!(
            rig.reads.borrow().as_slice(),
            &[l1(0x0040_1000), l0(L0, 0x0040_1000)]
        );
        assert!(n > 2, "the walk took {n}");
        // The same page again, at another offset: a hit, a cycle later.
        assert_eq!(
            rig.ask(Port::D, 0x0040_1004, true).await,
            (Ok(0x4123_4004), 2)
        );
        assert_eq!(rig.reads(), 2, "no second walk");
        // The fetch TLB is its own, so a fetch walks again.
        assert_eq!(
            rig.check(Port::I, 0x0040_1000, false).await,
            Ok(0x4123_4000)
        );
        assert_eq!(rig.reads(), 4);
    });
}

#[test]
fn a_megapage_maps_four_megabytes_from_one_entry() {
    run(|rig| async move {
        rig.set_mode(smode());
        let va = 0x8040_0000;
        rig.put(l1(va), pte::to(0x4080_0000, RWXAD));
        assert_eq!(
            rig.check(Port::D, va + 0x12_3456, false).await,
            Ok(0x4092_3456)
        );
        assert_eq!(rig.reads(), 1, "one level");
        // Anywhere in the four megabytes hits.
        assert_eq!(
            rig.ask(Port::D, va + 0x3f_fffc, false).await,
            (Ok(0x40bf_fffc), 2)
        );
        assert_eq!(rig.ask(Port::D, va, false).await, (Ok(0x4080_0000), 2));
        assert_eq!(rig.reads(), 1);
    });
}

/// Each way a walk or a check fails, and two ways the bits allow an
/// access they might be thought to refuse, each against the reference.
#[test]
fn every_fault_is_the_specifications() {
    run(|rig| async move {
        let va = 0x0040_2000;
        // (the leaf's bits, the mode, the port, a store, what it gives)
        let cases: &[(u32, Mode, Port, bool, Result<(), Fault>)] = &[
            (0, smode(), Port::D, false, Err(Fault::Page)),
            (V | W | A | D, smode(), Port::D, true, Err(Fault::Page)),
            (V | R | A, smode(), Port::D, true, Err(Fault::Page)),
            (V | R | W | A, smode(), Port::D, true, Err(Fault::Page)),
            (V | R | W | D, smode(), Port::D, false, Err(Fault::Page)),
            (V | R | A, smode(), Port::I, false, Err(Fault::Page)),
            (V | X | A, smode(), Port::D, false, Err(Fault::Page)),
            (
                V | X | A,
                Mode {
                    mxr: true,
                    ..smode()
                },
                Port::D,
                false,
                Ok(()),
            ),
            (V | R | UB | A, smode(), Port::D, false, Err(Fault::Page)),
            (
                V | R | UB | A,
                Mode {
                    sum: true,
                    ..smode()
                },
                Port::D,
                false,
                Ok(()),
            ),
            (
                V | X | UB | A,
                Mode {
                    sum: true,
                    ..smode()
                },
                Port::I,
                false,
                Err(Fault::Page),
            ),
            (
                V | R | X | A,
                Mode { prv: 0, ..smode() },
                Port::I,
                false,
                Err(Fault::Page),
            ),
            (
                V | R | X | UB | A,
                Mode { prv: 0, ..smode() },
                Port::I,
                false,
                Ok(()),
            ),
        ];
        for (k, &(bits, mode, port, store, want)) in cases.iter().enumerate() {
            rig.set_mode(mode);
            map(&rig, va, 0x4000_0000, bits);
            rig.flush().await;
            let got = rig.check(port, va, store).await;
            assert_eq!(got.map(|_| ()), want, "case {k}");
        }
        // A pointer at the second level.
        rig.set_mode(smode());
        map(&rig, va, 0x4000_0000, V);
        rig.flush().await;
        assert_eq!(rig.check(Port::D, va, false).await, Err(Fault::Page));
        // A megapage whose low page number is not zero.
        rig.put(l1(0x0080_0000), pte::to(0x4000_1000, RWXAD));
        rig.flush().await;
        assert_eq!(
            rig.check(Port::D, 0x0080_0000, false).await,
            Err(Fault::Page)
        );
    });
}

#[test]
fn an_address_past_thirty_two_bits_or_nowhere_is_an_access_fault() {
    run(|rig| async move {
        rig.set_mode(smode());
        // Nothing at the first level's entry: the read fails.
        assert_eq!(
            rig.check(Port::D, 0x0c00_0000, false).await,
            Err(Fault::Access)
        );
        // A leaf past thirty-two bits: an access fault when the access
        // is allowed, a page fault when it is not.
        let va = 0x0040_3000;
        rig.put(l1(va), pte::to(L0, V));
        rig.put(l0(L0, va), 1 << 30 | V | R | A);
        assert_eq!(rig.check(Port::D, va, false).await, Err(Fault::Access));
        rig.flush().await;
        assert_eq!(rig.check(Port::D, va, true).await, Err(Fault::Page));
        // A pointer past thirty-two bits.
        rig.put(l1(0x0100_0000), 1 << 31 | V);
        assert_eq!(
            rig.check(Port::I, 0x0100_0000, false).await,
            Err(Fault::Access)
        );
        // A `satp` past thirty-two bits: no walk at all.
        let n = rig.reads();
        rig.set_mode(Mode {
            satp: 1 << 31 | 1 << 20,
            ..smode()
        });
        assert_eq!(rig.check(Port::D, 0, false).await, Err(Fault::Access));
        assert_eq!(rig.reads(), n);
    });
}

#[test]
fn a_fault_is_not_kept_after_its_request() {
    run(|rig| async move {
        rig.set_mode(smode());
        let va = 0x0040_4000;
        map(&rig, va, 0x4000_0000, 0);
        assert_eq!(rig.check(Port::D, va, false).await, Err(Fault::Page));
        // The kernel makes it valid; the next request walks again.
        map(&rig, va, 0x4000_0000, RWXAD);
        assert_eq!(rig.check(Port::D, va, false).await, Ok(0x4000_0000));
    });
}

#[test]
fn a_flush_empties_both_tlbs() {
    run(|rig| async move {
        rig.set_mode(smode());
        let va = 0x0040_5000;
        map(&rig, va, 0x4000_0000, RWXAD);
        assert_eq!(rig.check(Port::I, va, false).await, Ok(0x4000_0000));
        assert_eq!(rig.check(Port::D, va, false).await, Ok(0x4000_0000));
        // A new mapping is not seen until the flush.
        map(&rig, va, 0x4100_0000, RWXAD);
        assert_eq!(rig.ask(Port::I, va, false).await, (Ok(0x4000_0000), 2));
        assert_eq!(rig.ask(Port::D, va, false).await, (Ok(0x4000_0000), 2));
        rig.flush().await;
        assert_eq!(rig.check(Port::I, va, false).await, Ok(0x4100_0000));
        assert_eq!(rig.check(Port::D, va, false).await, Ok(0x4100_0000));
    });
}

#[test]
fn a_flush_during_a_walk_drops_what_it_finds() {
    run(|rig| async move {
        rig.set_mode(smode());
        rig.latency.set(6);
        let va = 0x0040_6000;
        map(&rig, va, 0x4000_0000, RWXAD);
        rig.dreq.set(Bit::One);
        rig.dva.set(U::<32>::from(va));
        rig.store.set(Bit::Zero);
        cycles(3).await;
        // Mid-walk: the tables change, and the flush says so.
        map(&rig, va, 0x4100_0000, RWXAD);
        rig.flush().await;
        rig.dreq.set(Bit::Zero);
        cycles(1).await;
        assert_eq!(rig.check(Port::D, va, false).await, Ok(0x4100_0000));
    });
}

/// A request in the cycle of a flush is not answered from the TLB as it
/// was: it is answered later, from the emptied TLB, by a walk.
#[test]
fn a_request_in_the_cycle_of_a_flush_is_answered_after_it() {
    run(|rig| async move {
        rig.set_mode(smode());
        let va = 0x0040_8000;
        map(&rig, va, 0x4000_0000, RWXAD);
        assert_eq!(rig.check(Port::D, va, false).await, Ok(0x4000_0000));
        let n = rig.reads();
        map(&rig, va, 0x4100_0000, RWXAD);
        rig.flush.set(Bit::One);
        rig.dreq.set(Bit::One);
        rig.dva.set(U::<32>::from(va));
        rig.store.set(Bit::Zero);
        cycles(1).await;
        rig.flush.set(Bit::Zero);
        cycles(1).await;
        assert!(!rig.dok.get().to_bool(), "not answered from the old entry");
        let mut waited = 0;
        while !rig.dok.get().to_bool() {
            cycles(1).await;
            waited += 1;
            assert!(waited < 100, "never answered");
        }
        assert_eq!(rig.dpa.get().raw() as u32, 0x4100_0000);
        assert_eq!(rig.reads(), n + 2, "it walked");
        rig.dreq.set(Bit::Zero);
        cycles(1).await;
    });
}

#[test]
fn entries_are_replaced_in_turn() {
    run(|rig| async move {
        rig.set_mode(smode());
        let page = |k: u32| 0x0040_0000 + k * 0x1000;
        for k in 0..9 {
            map(&rig, page(k), 0x4000_0000 + k * 0x1000, RWXAD);
            rig.check(Port::D, page(k), false).await.unwrap();
        }
        // The ninth took the first's entry; the second is still there.
        assert_eq!(rig.ask(Port::D, page(1), false).await.1, 2);
        let n = rig.reads();
        rig.check(Port::D, page(0), false).await.unwrap();
        assert_eq!(rig.reads(), n + 2, "the first walks again");
    });
}

#[test]
fn the_data_port_walks_first() {
    run(|rig| async move {
        rig.set_mode(smode());
        let (iv, dv) = (0x0040_7000, 0x00c0_7000);
        map(&rig, iv, 0x4000_0000, RWXAD);
        rig.put(l1(dv), pte::to(0x8000_2000, V));
        rig.put(l0(0x8000_2000, dv), pte::to(0x4100_0000, RWXAD));
        rig.ireq.set(Bit::One);
        rig.iva.set(U::<32>::from(iv));
        rig.dreq.set(Bit::One);
        rig.dva.set(U::<32>::from(dv));
        let (mut i, mut d) = (false, false);
        while !(i && d) {
            cycles(1).await;
            if rig.iok.get().to_bool() {
                i = true;
                rig.ireq.set(Bit::Zero);
            }
            if rig.dok.get().to_bool() {
                d = true;
                rig.dreq.set(Bit::Zero);
            }
        }
        assert_eq!(
            rig.reads.borrow().as_slice(),
            &[l1(dv), l0(0x8000_2000, dv), l1(iv), l0(L0, iv)]
        );
    });
}

/// Random tables and random requests, on both ports and in both modes,
/// every answer the reference's. The tables do not change, so no flush
/// is needed and the TLBs fill and turn over as they go.
#[test]
fn random_requests_agree_with_the_reference() {
    run(|rig| async move {
        let mut seed = 0x2545_f491_u32;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        // Four first-level entries: two pointers, a megapage, and one
        // with random bits; each pointer's table with sixteen pages of
        // random bits, some of them invalid.
        let tops = [0x000u32, 0x001, 0x002, 0x003];
        for (k, &top) in tops.iter().enumerate() {
            let at = ROOT + top * 4;
            let table = 0x8001_0000 + (k as u32) * 0x1000;
            let entry = match k {
                0 | 1 => pte::to(table, V),
                2 => pte::to(0x4040_0000, RWXAD | UB),
                _ => rnd() & 0xff,
            };
            rig.put(at, entry);
            for p in 0..16 {
                let bits = (rnd() & 0xff) | (V * (rnd() % 4 != 0) as u32);
                rig.put(
                    table + p * 4,
                    pte::to(0x4000_0000 + (rnd() % 64) * 0x1000, bits),
                );
            }
        }
        for _ in 0..600 {
            let mode = Mode {
                satp: satp(ROOT),
                prv: [0, 1, 1, 3][(rnd() % 4) as usize],
                sum: rnd() % 2 == 0,
                mxr: rnd() % 2 == 0,
            };
            rig.set_mode(mode);
            let va = (rnd() % 4) << 22 | (rnd() % 20) << 12 | (rnd() & 0xffc);
            let port = if rnd() % 2 == 0 { Port::I } else { Port::D };
            let store = port == Port::D && rnd() % 2 == 0;
            rig.check(port, va, store).await.ok();
        }
    });
}

/// The netlist: one module with both TLBs and the walker.
#[test]
fn the_eight_entry_unit_lowers() {
    let v = Mmu8::verilog("mmu");
    assert!(v.contains("module mmu("), "the module");
    assert!(
        v.contains("ivpn_7") && v.contains("dvpn_7"),
        "eight entries each"
    );
    assert!(v.contains("ptw"), "the walker's port");
}
