// SPDX-License-Identifier: Apache-2.0
//! Wishbone, as far as an AXI link needs to reach it: a bridge from the
//! peripheral end of a link to the host lines of a pipelined Wishbone
//! bus, which is what controllers written elsewhere often speak.
//!
//! [`AxiWb`] is a peripheral client written as hardware, as Razboj's
//! framebuffer is. It takes one single-beat burst at a time and puts it
//! on the Wishbone lines as one request: `cyc` and `stb` held until the
//! peripheral takes it, a cycle with `cyc`, `stb` and no `stall`; then
//! `cyc` alone until `ack`; then the answer on the link. Every line it
//! drives is a register, so nothing combinational crosses from the link
//! to the peripheral or back.
//!
//! Words are thirty-two bits with four lanes, and the Wishbone address
//! is a word address: the burst's byte address shifted down by two and
//! cut to `AW` bits. A peripheral that decodes a region of the link's
//! address map gets the region's offset for free, as long as the base
//! lies above those bits.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

use crate::bus::axi::{Answer, PerReq, Resp, R, W};

pub mod sim;

/// A word is four bytes, so a byte address shifted down by this is a
/// word address.
const WORD: usize = 2;

// begin{state}
/// The bridge. `A` and `I` are the link's address and identifier
/// widths; `AW` is the width of the Wishbone word address.
#[derive(Trace, Default)]
pub struct AxiWb<const A: usize, const I: usize, const AW: usize> {
    /// Where the request is: 0 waiting for one, 1 a write waiting for
    /// its beat, 2 offered on the lines, 3 taken and waiting for the
    /// acknowledge, 4 answering on the link.
    pub stage: Reg<U<3>>,
    /// Whether the request is a read.
    pub reading: Reg<U<1>>,
    /// The burst's identifier, which the answer carries back.
    pub rid: Reg<U<I>>,
    /// The word address, as the lines carry it.
    pub wadr: Reg<U<AW>>,
    /// The word: for a write the one the beat brought, for a read the
    /// one the acknowledge brought.
    pub wdat: Reg<U<32>>,
    /// The lanes a write writes, all four for a read.
    pub wsel: Reg<U<4>>,
}
// end{state}

// begin{run}
#[lower]
impl<const A: usize, const I: usize, const AW: usize> Unit for AxiWb<A, I, AW> {
    async fn run(
        &mut self,
        (req, wd, stall, ack, rdat): (
            Rx<PerReq<A, I>>,
            Rx<W<32, 4>>,
            In<Bit>,
            In<Bit>,
            In<U<32>>,
        ),
        (ans, rb, cyc, stb, we, adr, dat, sel): (
            Tx<Answer<I>>,
            Tx<R<32, I>>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<U<AW>>,
            Out<U<32>>,
            Out<U<4>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let st = self.stage.get();
            // A request, taken when there is none in hand.
            let q = req.head();
            let take = (st == 0) & req.peek().is_some();
            let _ = req.recv_if(take);
            // A write's beat, which comes when it comes.
            let wh = wd.head();
            let beat = (st == 1) & wd.peek().is_some();
            let _ = wd.recv_if(beat);
            // The lines: offered, taken, acknowledged.
            let offered = st == 2;
            let taken = offered & !stall.get();
            let acked = (taken | (st == 3)) & ack.get();
            // The answer, sent when the link has room for it.
            let read = self.reading.get() == 1;
            let answering = st == 4;
            let sent_r = answering & read & rb.ready();
            let sent_b = answering & !read & ans.ready();
            let finished = sent_r | sent_b;
            // A read goes on the lines at once; a write waits for its
            // beat first.
            let first = mux(q.read, U::<3>::from(2u8), U::<3>::from(1u8));
            let is_read = q.read.zext::<1>();
            let word = (q.addr >> WORD).resize::<AW>();
            let answer = mux(read, rdat.get(), self.wdat.get());
            with!(self <= {
                take ? {
                    reading: is_read,
                    rid: q.id,
                    wadr: word,
                    wsel: U::<4>::from(15u8),
                    stage: first,
                },
                beat ? {
                    wdat: wh.data,
                    wsel: wh.strb,
                    stage: U::<3>::from(2u8),
                },
                taken ? stage: U::<3>::from(3u8),
                acked ? {
                    stage: U::<3>::from(4u8),
                    wdat: answer,
                },
                finished ? stage: U::<3>::from(0u8),
            });
            if sent_r.to_bool() {
                rb.send(R {
                    id: self.rid.get(),
                    data: self.wdat.get(),
                    resp: Resp::Okay,
                    last: Bit::One,
                });
            }
            if sent_b.to_bool() {
                ans.send(Answer {
                    id: self.rid.get(),
                    resp: Resp::Okay,
                });
            }
            cyc.set(offered | (st == 3));
            stb.set(offered);
            we.set(!read);
            adr.set(self.wadr.get());
            dat.set(self.wdat.get());
            sel.set(self.wsel.get());
        }
    }
}
// end{run}

/// The bridge on a link, against a Wishbone memory, driven from client
/// code on the host's side.
#[cfg(test)]
mod tests {
    use super::sim::WbMem;
    use super::AxiWb;
    use crate::bus::axi::{
        axi_to_unit, AxiHost, AxiPer, Host, HostLink, Rd, Resp, Wr,
    };
    use std::cell::RefCell;
    use std::future::Future;
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, DefaultClock, Running, Unit};
    use txhdl::types::{Bit, U};

    type Client = Host<32, 32, 4, 2, 4>;

    /// Run a client against a memory for `cycles` cycles, with the
    /// link, the bridge and the Wishbone lines between them.
    fn drive(
        mem: WbMem<28>,
        client: impl FnOnce(Client) -> Box<dyn Future<Output = ()> + Unpin>,
        cycles: usize,
    ) {
        let HostLink {
            host,
            per_client,
            host_in,
            host_out,
            per_in,
            per_out,
        } = axi_to_unit::<32, 32, 4, 2, 4>();
        let (req, wd, ans, rb) = per_client;
        let (cyc_o, cyc) = signal::<Bit, DefaultClock>();
        let (stb_o, stb) = signal::<Bit, DefaultClock>();
        let (we_o, we) = signal::<Bit, DefaultClock>();
        let (adr_o, adr) = signal::<U<28>, DefaultClock>();
        let (dat_o, dat) = signal::<U<32>, DefaultClock>();
        let (sel_o, sel) = signal::<U<4>, DefaultClock>();
        let (stall_o, stall) = signal::<Bit, DefaultClock>();
        let (ack_o, ack) = signal::<Bit, DefaultClock>();
        let (rdat_o, rdat) = signal::<U<32>, DefaultClock>();
        let mut h = AxiHost::<32, 32, 4, 2, 4>::default();
        let mut p = AxiPer::<32, 32, 4, 2>::default();
        let mut bridge = AxiWb::<32, 2, 28>::default();
        let mut mem = mem;
        // The memory before the bridge: the bridge reads the memory's
        // lines in the cycle they are driven, as the netlist does.
        let mut sim = Running::new(join2(
            join2(h.run(host_in, host_out), p.run(per_in, per_out)),
            join2(
                join2(
                    mem.run(
                        (cyc, stb, we, adr, dat, sel),
                        (stall_o, ack_o, rdat_o),
                    ),
                    bridge.run(
                        (req, wd, stall, ack, rdat),
                        (ans, rb, cyc_o, stb_o, we_o, adr_o, dat_o, sel_o),
                    ),
                ),
                client(host),
            ),
        ));
        for _ in 0..cycles {
            sim.cycle();
        }
    }

    #[test]
    fn a_read_gets_what_a_write_put() {
        let mem = WbMem::<28>::new(1, 0);
        let got = Rc::new(RefCell::new(Vec::new()));
        let out = got.clone();
        drive(
            mem.clone(),
            move |host| {
                Box::new(Box::pin(async move {
                    let a = host
                        .write(
                            Wr::at(0x4000_0010u32),
                            &[U::from(0xdead_beefu32)],
                        )
                        .await;
                    assert_eq!(a.done().await.resp, Resp::Okay);
                    let r = host.read(Rd::at(0x4000_0010u32, 1)).await;
                    // The answer first, then the borrow: a borrow
                    // held across an await is one the next task cannot
                    // take, and this executor runs them in one thread.
                    let done = r.done().await;
                    out.borrow_mut().push(done);
                }))
            },
            200,
        );
        let got = got.borrow();
        assert_eq!(got.len(), 1, "the read was never answered");
        assert_eq!(got[0].data[0].raw(), 0xdead_beef);
        // Word 4 of the region, the base above the address's bits.
        assert_eq!(mem.word(4), 0xdead_beef);
    }

    /// Words written at pseudorandom addresses, with a memory that
    /// calibrates for a while and answers slowly, every one read back.
    #[test]
    fn slow_answers_and_a_calibration() {
        let mem = WbMem::<28>::new(5, 40);
        let done = Rc::new(RefCell::new(0));
        let count = done.clone();
        drive(
            mem.clone(),
            move |host| {
                Box::new(Box::pin(async move {
                    let mut x = 0x2545_f491u32;
                    let mut model = std::collections::HashMap::new();
                    for _ in 0..24 {
                        x ^= x << 13;
                        x ^= x >> 17;
                        x ^= x << 5;
                        let at = (x & 0x3f) << 2;
                        let v = x.rotate_left(7);
                        let a = host.write(Wr::at(at), &[U::from(v)]).await;
                        assert_eq!(a.done().await.resp, Resp::Okay);
                        model.insert(at, v);
                        let r = host.read(Rd::at(at, 1)).await.done().await;
                        assert_eq!(
                            r.data[0].raw() as u32,
                            model[&at],
                            "at {at:#x}"
                        );
                        *count.borrow_mut() += 1;
                    }
                }))
            },
            6000,
        );
        assert_eq!(*done.borrow(), 24, "every word written and read back");
    }
}
