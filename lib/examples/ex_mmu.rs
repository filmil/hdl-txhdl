// SPDX-License-Identifier: Apache-2.0
//! Sv32 translation, the way a kernel meets it. Page tables sit in a
//! memory behind the walker: a megapage of kernel text, a user page,
//! and a page whose dirty bit is clear. A core's two ports ask in turn,
//! each answered a cycle after it asks or after a walk: untranslated
//! before `satp` is set, a fetch that walks one level and then hits, a
//! load from the user page that faults until `SUM` is set, a store that
//! faults until the kernel sets the dirty bit and flushes, both ports
//! missing at once with the data port walked first, and a read of a
//! table that is not there. The unit is lowered, and the build
//! simulates its netlist against this run under nvc and under
//! Verilator.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, signal, Clock, DefaultClock, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl_parts::mmu::pte::{to, A, D, R, U as USER, V, W, X};
use txhdl_parts::mmu::{satp, Mmu8, Pte};

/// The first level's table, and a second level's.
const ROOT: u32 = 0x8000_0000;
const L0: u32 = 0x8000_1000;

/// The first level's entry for `va`.
fn top(va: u32) -> u32 {
    ROOT + (va >> 22) * 4
}

async fn cycles(n: usize) {
    for _ in 0..n {
        DefaultClock::rising().await;
    }
}

fn main() {
    let (rst_o, rst) = signal::<Bit, DefaultClock>();
    let (satp_o, satp_i) = signal::<U<32>, DefaultClock>();
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
    let mut mmu = Mmu8::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("rst", &rst);
        wave.add("satp", &satp_i);
        wave.add("prv", &prv);
        wave.add("sum", &sum);
        wave.add("mxr", &mxr);
        wave.add("flush", &flush);
        wave.add("ireq", &ireq);
        wave.add("ireq_va", &iva);
        wave.add("dreq", &dreq);
        wave.add("dreq_va", &dva);
        wave.add("dreq_store", &store);
        wave.add("pte", &pte_rx);
        wave.add("ires_ok", &iok);
        wave.add("ires_pa", &ipa);
        wave.add("ires_fault", &ifault);
        wave.add("ires_err", &ierr);
        wave.add("dres_ok", &dok);
        wave.add("dres_pa", &dpa);
        wave.add("dres_fault", &dfault);
        wave.add("dres_err", &derr);
        wave.add("ptw", &ptw_tx);
        wave.add("mmu", &mmu);
        wave.start();
    }

    // The tables: the kernel's text as a megapage at 0xc000_0000, and
    // under 0x0040_0000 a user page, a kernel page whose dirty bit is
    // clear, and one more for the fetch.
    let mem = Rc::new(RefCell::new(HashMap::new()));
    let put = |at: u32, v: u32| mem.borrow_mut().insert(at, v);
    put(top(0xc000_0000), to(0x4040_0000, V | R | X | A));
    put(top(0x0040_0000), to(L0, V));
    put(L0, to(0x4100_0000, V | R | W | USER | A | D));
    put(L0 + 4, to(0x4100_1000, V | R | W | A));
    put(L0 + 8, to(0x4100_2000, V | R | X | A));
    let tables = mem.clone();

    // The memory behind the walker: an entry two cycles after it is
    // asked for, and an error where nothing is.
    let memory = async move {
        loop {
            let at = ptw_rx.wait().await.raw() as u32;
            cycles(2).await;
            let word = tables.borrow().get(&at).copied();
            pte_tx
                .put(|| Pte {
                    data: U::<32>::from(word.unwrap_or(0)),
                    err: Bit::from_bool(word.is_none()),
                })
                .await;
        }
    };

    let ports = (
        (ireq_o, iva_o, iok, ipa, ifault, ierr),
        (dreq_o, dva_o, dok, dpa, dfault, derr),
    );
    let client = async move {
        let (
            (ireq, iva, iok, ipa, ifault, ierr),
            (dreq, dva, dok, dpa, dfault, derr),
        ) = &ports;
        // One request on a port, held until it is answered.
        let ask = |data: bool, va: u32, st: bool| {
            let (req, addr) = if data { (dreq, dva) } else { (ireq, iva) };
            let (ok, pa, fault, err) = if data {
                (dok, dpa, dfault, derr)
            } else {
                (iok, ipa, ifault, ierr)
            };
            req.set(Bit::One);
            addr.set(U::<32>::from(va));
            store_o.set(Bit::from_bool(st));
            async move {
                let mut n = 0;
                loop {
                    DefaultClock::rising().await;
                    n += 1;
                    let what = if ok.get().to_bool() {
                        format!("{:#010x}", pa.get().raw())
                    } else if fault.get().to_bool() {
                        "page fault".to_string()
                    } else if err.get().to_bool() {
                        "access fault".to_string()
                    } else {
                        continue;
                    };
                    req.set(Bit::Zero);
                    let port = if data && st {
                        "store"
                    } else if data {
                        "load "
                    } else {
                        "fetch"
                    };
                    println!(
                        "t={:>3} {port} {va:#010x} -> {what}, cycles {n}",
                        now()
                    );
                    cycles(1).await;
                    return what;
                }
            }
        };
        // Past the reset, and the answer registers' reset.
        cycles(2).await;
        // Machine mode, `satp` zero: the address as it is, a cycle later
        // like every answer.
        prv_o.set(U::<2>::from(3u8));
        assert_eq!(ask(false, 0x4000_0000, false).await, "0x40000000");
        // Supervisor mode with the tables on. The kernel's text walks
        // one level, then hits.
        satp_o.set(U::<32>::from(satp(ROOT)));
        prv_o.set(U::<2>::from(1u8));
        assert_eq!(ask(false, 0xc012_3450, false).await, "0x40523450");
        assert_eq!(ask(false, 0xc012_3454, false).await, "0x40523454");
        // A user page from the kernel: a fault until `SUM` is set.
        assert_eq!(ask(true, 0x0040_0010, false).await, "page fault");
        sum_o.set(Bit::One);
        assert_eq!(ask(true, 0x0040_0010, false).await, "0x41000010");
        // A store to a page whose dirty bit is clear: the kernel sets
        // it, flushes, and the store goes again.
        assert_eq!(ask(true, 0x0040_1000, true).await, "page fault");
        mem.borrow_mut()
            .insert(L0 + 4, to(0x4100_1000, V | R | W | A | D));
        flush_o.set(Bit::One);
        cycles(1).await;
        flush_o.set(Bit::Zero);
        assert_eq!(ask(true, 0x0040_1000, true).await, "0x41001000");
        // Both ports miss at once: the data port's walk goes first.
        ireq.set(Bit::One);
        iva.set(U::<32>::from(0x0040_2000u32));
        let d = ask(true, 0xc000_0040, false).await;
        assert_eq!(d, "0x40400040");
        assert_eq!(ask(false, 0x0040_2000, false).await, "0x41002000");
        // A table that is not there: an access fault.
        assert_eq!(ask(true, 0x0800_0000, false).await, "access fault");
        println!("every answer the specification's");
    };

    // The client first: it drives the request wires, which the unit
    // reads in the same step.
    let hardware = mmu.run(
        (
            rst, satp_i, prv, sum, mxr, flush, ireq, iva, dreq, dva, store,
            pte_rx,
        ),
        (
            iok_o, ipa_o, ifault_o, ierr_o, dok_o, dpa_o, dfault_o, derr_o,
            ptw_tx,
        ),
    );
    mxr_o.set(Bit::Zero);
    let mut sim = Running::new(join2(join2(client, hardware), memory));
    rst_o.set(Bit::One);
    sim.cycle();
    rst_o.set(Bit::Zero);
    for _ in 0..140 {
        sim.cycle();
    }
    stop();
    txhdl::netlist::write_netlists_from_env(&[&Mmu8::lowered("mmu8")]);
}
