// SPDX-License-Identifier: Apache-2.0
//! Simulation-only AXI peripherals: what a testbench, a model or a
//! bring-up puts on a link when the point is the other end.
//!
//! Nothing here lowers, and nothing here is meant to. These are
//! written in plain Rust, against the [`Per`] end rather than against
//! the five channels, so they may be as large and as simple as the
//! run wants: a memory of a megabyte is a `Vec`, not a `Mem` the
//! netlist would have to hold. A peripheral that is to become
//! hardware is written in the lowered subset instead, as the GPU's
//! framebuffer is.
//!
//! The module's name is the contract. `bus::axi` is the link and the
//! two units that do its protocol; `bus::axi::sim` is what only ever
//! runs in a simulation.
//!
//! [`Per`]: super::Per
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::types::U;

use super::{serve, BurstKind, Per, Resp, Xact};

// begin{ram}
/// A RAM behind an AXI peripheral end: `words` words of `D` bits,
/// byte-addressed, answering bursts of any length.
///
/// It is a handle on the words, so a testbench keeps one while the
/// run has another: load a program into it before the run and read
/// the result out after. The widths are the link's: `A` the address
/// width, `D` the data width, `S` the strobe width, which is `D / 8`,
/// and `I` the identifier width.
pub struct Ram<const A: usize, const D: usize, const S: usize, const I: usize> {
    words: Rc<RefCell<Vec<U<D>>>>,
}
// end{ram}

impl<const A: usize, const D: usize, const S: usize, const I: usize> Clone
    for Ram<A, D, S, I>
{
    fn clone(&self) -> Self {
        Ram {
            words: self.words.clone(),
        }
    }
}

impl<const A: usize, const D: usize, const S: usize, const I: usize>
    Ram<A, D, S, I>
{
    /// A RAM of `words` words, every one zero.
    pub fn new(words: usize) -> Self {
        Ram {
            words: Rc::new(RefCell::new(vec![U::<D>::new(0); words])),
        }
    }

    /// A RAM of `words` words with `image` at its first word.
    pub fn with(words: usize, image: &[U<D>]) -> Self {
        let ram = Ram::new(words);
        ram.load(0, image);
        ram
    }

    /// How many words it holds.
    pub fn len(&self) -> usize {
        self.words.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Put `image` at word `at`. Words past the end are dropped, so a
    /// program too long for the RAM is short rather than a panic.
    pub fn load(&self, at: usize, image: &[U<D>]) {
        let mut w = self.words.borrow_mut();
        for (i, v) in image.iter().enumerate() {
            if at + i < w.len() {
                w[at + i] = *v;
            }
        }
    }

    /// The word at index `at`, or zero past the end.
    pub fn word(&self, at: usize) -> U<D> {
        self.words
            .borrow()
            .get(at)
            .copied()
            .unwrap_or(U::<D>::new(0))
    }

    /// Put one word at index `at`.
    pub fn set(&self, at: usize, v: U<D>) {
        let mut w = self.words.borrow_mut();
        if at < w.len() {
            w[at] = v;
        }
    }

    /// The bytes a beat carries, which is the strobe's width.
    const BYTES: usize = S;

    /// The word a byte address names, and whether it is in the RAM.
    fn index(&self, addr: u128) -> Option<usize> {
        let at = (addr as usize) / Self::BYTES;
        (at < self.len()).then_some(at)
    }

    /// The address of beat `i` of a burst, as AXI counts them: fixed
    /// stays put, incrementing steps a beat's width, and wrapping
    /// steps and comes back to the start of its aligned block.
    fn beat_addr(
        &self,
        start: u128,
        kind: BurstKind,
        len: usize,
        i: usize,
    ) -> u128 {
        let step = Self::BYTES as u128;
        match kind {
            BurstKind::Fixed => start,
            BurstKind::Wrap => {
                let span = step * (len as u128 + 1);
                let base = start - (start % span);
                base + ((start - base) + step * i as u128) % span
            }
            _ => start + step * i as u128,
        }
    }

    /// One beat written, under its strobe: a lane the strobe does not
    /// cover keeps what it held.
    fn write_beat(&self, at: usize, data: U<D>, strb: U<S>) {
        let old = self.word(at).raw();
        let new = data.raw();
        let mut out = old;
        for b in 0..Self::BYTES {
            if strb.bit(b).to_bool() {
                let shift = b * 8;
                let mask = 0xffu128 << shift;
                out = (out & !mask) | (new & mask);
            }
        }
        self.set(at, U::<D>::new(out));
    }

    /// Serve `per` from this RAM for as long as the run lasts, with
    /// `slots` transactions open at once. A burst wholly inside the
    /// RAM is answered `Okay`; one that reaches past the end is
    /// answered `SlvErr`, which is what a peripheral says to an
    /// address it has but cannot serve.
    pub async fn serve(self, per: Per<A, D, S, I>, slots: usize) {
        serve(per, slots, move |x: Xact<A, D, S, I>| {
            let ram = self.clone();
            async move {
                match x {
                    Xact::Write(w) => {
                        let req = w.req();
                        let beats = w.data().to_vec();
                        let strbs = w.strb().to_vec();
                        let mut ok = true;
                        for (i, (d, s)) in beats.iter().zip(&strbs).enumerate()
                        {
                            let a = ram.beat_addr(
                                req.addr.raw(),
                                req.burst,
                                beats.len() - 1,
                                i,
                            );
                            match ram.index(a) {
                                Some(at) => ram.write_beat(at, *d, *s),
                                None => ok = false,
                            }
                        }
                        if ok {
                            w.ok().await
                        } else {
                            w.err(Resp::SlvErr).await
                        }
                    }
                    Xact::Read(r) => {
                        let req = r.req();
                        let n = r.words();
                        let mut out = Vec::with_capacity(n);
                        let mut ok = true;
                        for i in 0..n {
                            let a = ram.beat_addr(
                                req.addr.raw(),
                                req.burst,
                                n - 1,
                                i,
                            );
                            match ram.index(a) {
                                Some(at) => out.push(ram.word(at)),
                                None => {
                                    ok = false;
                                    out.push(U::<D>::new(0));
                                }
                            }
                        }
                        if ok {
                            r.data(&out).await
                        } else {
                            r.err(Resp::SlvErr).await
                        }
                    }
                }
            }
        })
        .await
    }
}

/// The RAM against a `Vec`: what a burst writes is what a burst
/// reads, the strobe covers the lanes it says and no others, a burst
/// walks the words its burst type says, and an address past the end
/// is answered rather than panicked.
#[cfg(test)]
mod tests {
    use super::Ram;
    use crate::bus::axi::{
        axi, AxiHost, AxiPer, BurstKind, Link, Rd, Resp, Wr,
    };
    use std::cell::RefCell;
    use std::future::Future;
    use std::rc::Rc;
    use txhdl::comp::{join2, Running, Unit};
    use txhdl::types::U;

    type TestRam = Ram<16, 32, 4, 2>;

    /// Run a client against a RAM of `words` words for `cycles`
    /// cycles, with the whole link between them.
    fn drive(
        ram: TestRam,
        client: impl FnOnce(
            crate::bus::axi::Host<16, 32, 4, 2, 4>,
        ) -> Box<dyn Future<Output = ()> + Unpin>,
        cycles: usize,
    ) {
        let Link {
            host,
            per,
            host_in,
            host_out,
            per_in,
            per_out,
        } = axi::<16, 32, 4, 2, 4>();
        let mut h = AxiHost::<16, 32, 4, 2, 4>::default();
        let mut p = AxiPer::<16, 32, 4, 2>::default();
        let mut sim = Running::new(join2(
            join2(h.run(host_in, host_out), p.run(per_in, per_out)),
            join2(client(host), ram.serve(per, 2)),
        ));
        for _ in 0..cycles {
            sim.cycle();
        }
    }

    /// A word, for the tests to write and look for.
    fn w(v: u32) -> U<32> {
        U::from(v)
    }

    #[test]
    fn a_burst_reads_what_a_burst_wrote() {
        let ram = TestRam::new(64);
        let got = Rc::new(RefCell::new(Vec::new()));
        let out = got.clone();
        drive(
            ram.clone(),
            move |host| {
                Box::new(Box::pin(async move {
                    let beats = [w(0x1111), w(0x2222), w(0x3333)];
                    let a = host.write(Wr::at(0x10u32), &beats).await;
                    assert_eq!(a.done().await.resp, Resp::Okay);
                    let r = host.read(Rd::at(0x10u32, 3)).await;
                    let reply = r.done().await;
                    out.borrow_mut().push(reply);
                }))
            },
            200,
        );
        let got = got.borrow();
        assert_eq!(got.len(), 1, "the read was never answered");
        assert_eq!(got[0].resp, Resp::Okay);
        let raw: Vec<u128> = got[0].data.iter().map(|d| d.raw()).collect();
        assert_eq!(raw, vec![0x1111, 0x2222, 0x3333]);
        // The words are where the byte addresses say.
        assert_eq!(ram.word(4).raw(), 0x1111);
        assert_eq!(ram.word(5).raw(), 0x2222);
        assert_eq!(ram.word(6).raw(), 0x3333);
    }

    #[test]
    fn a_strobe_covers_the_lanes_it_says() {
        let ram = TestRam::new(16);
        ram.set(2, w(0xaabb_ccdd));
        drive(
            ram.clone(),
            move |host| {
                Box::new(Box::pin(async move {
                    // `Host::write` covers every lane, so the burst
                    // here checks the whole-word path; the narrowed
                    // strobe is checked against the rule below, and
                    // through a link by the core, which is the client
                    // that sends one.
                    let a =
                        host.write(Wr::at(0x08u32), &[w(0x1234_5678)]).await;
                    assert_eq!(a.done().await.resp, Resp::Okay);
                }))
            },
            120,
        );
        assert_eq!(ram.word(2).raw(), 0x1234_5678, "a full write");
        // A narrowed strobe, written straight into the model, is the
        // same rule the burst used.
        ram.set(3, w(0xffff_ffff));
        ram.write_beat(3, w(0x0000_1100), U::<4>::from(2u8));
        assert_eq!(ram.word(3).raw(), 0xffff_11ff, "one lane replaced");
    }

    #[test]
    fn a_fixed_burst_stays_on_one_word() {
        let ram = TestRam::new(16);
        drive(
            ram.clone(),
            move |host| {
                Box::new(Box::pin(async move {
                    let mut wr = Wr::at(0x20u32);
                    wr.burst = BurstKind::Fixed;
                    let a = host.write(wr, &[w(1), w(2), w(3)]).await;
                    assert_eq!(a.done().await.resp, Resp::Okay);
                }))
            },
            200,
        );
        assert_eq!(ram.word(8).raw(), 3, "the last beat of a fixed burst");
        assert_eq!(ram.word(9).raw(), 0, "a fixed burst moved on");
    }

    #[test]
    fn an_address_past_the_end_is_answered() {
        let ram = TestRam::new(8);
        let got = Rc::new(RefCell::new(Vec::new()));
        let out = got.clone();
        drive(
            ram,
            move |host| {
                Box::new(Box::pin(async move {
                    let a = host.write(Wr::at(0x400u32), &[w(7)]).await;
                    let wr = a.done().await;
                    let r = host.read(Rd::at(0x400u32, 2)).await;
                    let rd = r.done().await;
                    out.borrow_mut().push((wr.resp, rd.resp, rd.data.len()));
                }))
            },
            200,
        );
        let got = got.borrow();
        assert_eq!(got.len(), 1, "neither burst was answered");
        assert_eq!(got[0].0, Resp::SlvErr, "a write past the end");
        assert_eq!(got[0].1, Resp::SlvErr, "a read past the end");
        assert_eq!(got[0].2, 2, "a refused read still answers its beats");
    }

    #[test]
    fn an_image_is_loaded_and_read_back() {
        let image: Vec<U<32>> = (0..8).map(|i| w(0x100 + i)).collect();
        let ram = TestRam::with(32, &image);
        let got = Rc::new(RefCell::new(Vec::new()));
        let out = got.clone();
        drive(
            ram,
            move |host| {
                Box::new(Box::pin(async move {
                    let r = host.read(Rd::at(0x00u32, 8)).await;
                    out.borrow_mut().push(r.done().await);
                }))
            },
            200,
        );
        let got = got.borrow();
        let raw: Vec<u128> = got[0].data.iter().map(|d| d.raw()).collect();
        assert_eq!(raw, (0..8).map(|i| 0x100 + i as u128).collect::<Vec<_>>());
    }
}
