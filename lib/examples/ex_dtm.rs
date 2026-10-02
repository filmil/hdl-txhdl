// SPDX-License-Identifier: Apache-2.0
//! The RISC-V debug transport behind `BSCANE2`, driven the way a stock
//! OpenOCD drives it through its BSCAN tunnel (issue 154).
//!
//! A driver on the cable's clock plays the scans OpenOCD makes, field
//! for field: a reset, a data scan that reads the transport's IDCODE,
//! `dtmcs`, then a write and a read of a debug module register through
//! `dmi`. Behind the transport a model module answers each access two
//! edges after it arrives. The transport is lowered, and the build
//! simulates its netlist against this run under nvc and Verilator.
use std::cell::Cell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, signal, Clock, Out, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::dtm::{
    Dtm, Tck, DMI_WIDTH, IR_DMI, IR_DTMCS, OP_NOP, OP_READ, OP_WRITE,
};

/// The pins of `BSCANE2` the driver sets, and the transport's TDO,
/// which the device's TAP presents an edge late.
struct Pins {
    capture: Out<Bit, Tck>,
    shift: Out<Bit, Tck>,
    update: Out<Bit, Tck>,
    tdi: Out<Bit, Tck>,
    reset: Out<Bit, Tck>,
}

impl Pins {
    /// One edge with these levels, set on the falling edge before it,
    /// as a JTAG host changes its lines, so that no edge sees them
    /// change under it.
    async fn edge(&self, cap: bool, sh: bool, upd: bool, d: bool) {
        Tck::falling().await;
        self.capture.set(Bit::from(cap));
        self.shift.set(Bit::from(sh));
        self.update.set(Bit::from(upd));
        self.tdi.set(Bit::from(d));
        Tck::rising().await;
    }
}

fn bits(v: u64, n: u32) -> Vec<bool> {
    (0..n).map(|i| (v >> i) & 1 == 1).collect()
}

/// One data scan of `USER4`, as OpenOCD's tunnel frames it: `ir` is
/// a tunneled instruction scan, otherwise a data scan of `w` bits. It
/// returns the register as OpenOCD reads it, from the second bit of
/// the payload on.
async fn tunnel(p: &Pins, tdo: Reg<Bit, Tck>, ir: bool, w: u32, v: u64) -> u64 {
    let mut out = vec![!ir];
    out.extend(bits(w as u64, 7));
    out.extend(bits(v, w));
    if !ir {
        out.push(false);
    }
    out.extend([false; 3]);
    p.edge(true, false, false, false).await;
    let mut got = Vec::new();
    for b in out {
        p.edge(false, true, false, b).await;
        got.push(tdo.get() == Bit::One);
    }
    p.edge(false, false, true, false).await;
    let skip = if ir { 0 } else { 9 };
    got[skip..skip + w as usize]
        .iter()
        .rev()
        .fold(0, |a, &b| (a << 1) | b as u64)
}

fn main() {
    let (sel_o, sel) = signal::<Bit, Tck>();
    let (shift_o, shift) = signal::<Bit, Tck>();
    let (capture_o, capture) = signal::<Bit, Tck>();
    let (update_o, update) = signal::<Bit, Tck>();
    let (tdi_o, tdi) = signal::<Bit, Tck>();
    let (reset_o, reset) = signal::<Bit, Tck>();
    let (tdo_o, tdo) = signal::<Bit, Tck>();
    let (req_tx, req) = chan::<U<41>, Tck>();
    let (ans_tx, ans) = chan::<U<34>, Tck>();
    let mut dtm = Dtm::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<Tck>();
        wave.add("sel", &sel);
        wave.add("shift", &shift);
        wave.add("capture", &capture);
        wave.add("update", &update);
        wave.add("tdi", &tdi);
        wave.add("reset", &reset);
        wave.add("ans", &ans);
        wave.add("tdo", &tdo);
        wave.add("req", &req);
        wave.add("dtm", &dtm);
        wave.start();
    }

    // The TDO the host sees: the transport's register, read before
    // each edge, which is its value after the edge before.
    let out = dtm.out;
    let done = Rc::new(Cell::new(false));
    let fin = done.clone();
    let p = Pins {
        capture: capture_o,
        shift: shift_o,
        update: update_o,
        tdi: tdi_o,
        reset: reset_o,
    };
    let driver = async move {
        Tck::falling().await;
        sel_o.set(Bit::One);
        p.reset.set(Bit::One);
        Tck::rising().await;
        Tck::rising().await;
        Tck::falling().await;
        p.reset.set(Bit::Zero);
        let t = out;
        let id = tunnel(&p, t, false, 32, 0).await;
        println!("idcode {id:08x}");
        tunnel(&p, t, true, 5, IR_DTMCS as u64).await;
        let cs = tunnel(&p, t, false, 32, 0).await;
        println!("dtmcs  {cs:08x}");
        tunnel(&p, t, true, 5, IR_DMI as u64).await;
        let dmi = |a: u64, d: u64, op: u32| (a << 34) | (d << 2) | op as u64;
        let w = DMI_WIDTH;
        tunnel(&p, t, false, w, dmi(0x04, 0x1234_5678, OP_WRITE)).await;
        for _ in 0..4 {
            p.edge(false, false, false, false).await;
        }
        tunnel(&p, t, false, w, dmi(0x04, 0, OP_READ)).await;
        for _ in 0..4 {
            p.edge(false, false, false, false).await;
        }
        let r = tunnel(&p, t, false, w, dmi(0, 0, OP_NOP)).await;
        println!("dmi    op {} data {:08x}", r & 3, (r >> 2) & 0xffff_ffff);
        fin.set(true);
        loop {
            p.edge(false, false, false, false).await;
        }
    };
    // The model module: a register file, each access answered two
    // edges after it arrives.
    let model = async move {
        let mut regs = [0u32; 128];
        loop {
            Tck::rising().await;
            if let Some(r) = req.recv() {
                let r = r.raw() as u64;
                let a = (r >> 34) as usize;
                if r & 3 == OP_WRITE as u64 {
                    regs[a] = (r >> 2) as u32;
                }
                Tck::rising().await;
                Tck::rising().await;
                ans_tx.send(U::<34>::from((regs[a] as u64) << 2));
            }
        }
    };
    let mut sim = Running::new(join2(
        dtm.run(
            (sel, shift, capture, update, tdi, reset, ans),
            (tdo_o, req_tx),
        ),
        join2(driver, model),
    ));
    while !done.get() {
        sim.cycle();
    }
    for _ in 0..4 * Tck::PERIOD {
        sim.cycle();
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Dtm::lowered("dtm"));
    print!("\n{}", Dtm::verilog("dtm"));
}
