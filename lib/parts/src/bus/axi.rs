// SPDX-License-Identifier: Apache-2.0
//! An AXI4 link: five channels, two ends, and a client that never
//! sees a channel.
//!
//! [`axi`] is to AXI what [`chan`] is to one transaction. It makes
//! every wire of a link at once and hands back two ends, a [`Host`]
//! and a [`Per`], and the ports of the two units that sit between
//! them. A client of either end is written as waiting on AXI
//! transactions: the host issues a burst and awaits its answer, the
//! peripheral accepts a whole decoded transaction and answers it.
//! Neither writes an identifier, counts a beat, watches `last`, or
//! names `aw`, `w`, `b`, `ar` or `r`.
//!
//! The layering, from a client down to the wire:
//!
//! ```text
//!     host client        issue / beat / grant / done  transaction level
//!     AxiHost            #[lower], replaceable
//!        aw ar w b r     the AXI4 channels, where a router goes
//!     AxiPer             #[lower], replaceable
//!     peripheral client  req / write data / answer    transaction level
//! ```
//!
//! [`AxiHost`] and [`AxiPer`] are units written in the lowered
//! subset, so each is a netlist as well as a simulation, and each
//! lowers to a module whose ports are the AXI channels a synthesis
//! tool expects. Between them is real AXI, which is where routing
//! attaches. Either may be replaced by a unit of the same ports: the
//! port types are named ([`HostIn`], [`HostOut`], [`PerIn`],
//! [`PerOut`]) so that a design may supply its own.
//!
//! The widths are stated rather than computed, as they are
//! everywhere in this library, because `U<{D / 8}>` needs nightly
//! Rust: `A` is the address width, `D` the data width, `S` the
//! strobe width, which is `D / 8`, `I` the identifier width and
//! `NIDS` the count of identifiers, which is `1 << I`.
//!
//! [`chan`]: txhdl::comp::chan
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::rc::Rc;
use txhdl::comp::{
    chan, join_all, mux, now, Clock, DefaultClock, Reg, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

// ---------------------------------------------------------------------
// The values on the wire

/// A response, as AXI4 encodes it.
#[derive(ValueDerive, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Resp {
    #[default]
    Okay,
    ExOkay,
    SlvErr,
    DecErr,
}

/// A burst type, as AXI4 encodes it. `Incr` is the usual one and the
/// default, since a burst that is not incrementing is the exception.
#[derive(ValueDerive, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum BurstKind {
    Fixed,
    #[default]
    Incr,
    Wrap,
    Reserved,
}

// begin{beats}
/// The address phase of a burst: every AXI4 field of `AW`, which are
/// the fields of `AR` under other names, so one struct serves both
/// channels and [`Aw`] and [`Ar`] name it as the channel does.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Addr<const A: usize, const I: usize> {
    pub id: U<I>,
    pub addr: U<A>,
    /// Beats in the burst, less one, as AXI counts them.
    pub len: U<8>,
    pub size: U<3>,
    pub burst: BurstKind,
    pub lock: Bit,
    pub cache: U<4>,
    pub prot: U<3>,
    pub qos: U<4>,
    pub region: U<4>,
}

/// The write address channel's beat.
pub type Aw<const A: usize, const I: usize> = Addr<A, I>;
/// The read address channel's beat. The same fields.
pub type Ar<const A: usize, const I: usize> = Addr<A, I>;

/// The write data channel's beat.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct W<const D: usize, const S: usize> {
    pub data: U<D>,
    pub strb: U<S>,
    pub last: Bit,
}

/// The write response channel's beat.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct B<const I: usize> {
    pub id: U<I>,
    pub resp: Resp,
}

/// The read data channel's beat.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct R<const D: usize, const I: usize> {
    pub id: U<I>,
    pub data: U<D>,
    pub resp: Resp,
    pub last: Bit,
}
// end{beats}

// ---------------------------------------------------------------------
// The transaction level, between a client and its tracker

/// What a host client hands its tracker: an address phase with no
/// identifier, and whether it is a read. The identifier is the
/// tracker's to allocate, which is why the client never writes one.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Issue<const A: usize> {
    pub read: Bit,
    pub addr: U<A>,
    pub len: U<8>,
    pub size: U<3>,
    pub burst: BurstKind,
    pub lock: Bit,
    pub cache: U<4>,
    pub prot: U<3>,
    pub qos: U<4>,
    pub region: U<4>,
}

/// The identifier the tracker allocated, told back to the client in
/// the cycle the burst went out.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Grant<const I: usize> {
    pub id: U<I>,
}

/// A write finished: what the tracker makes of a `B` beat.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Done<const I: usize> {
    pub id: U<I>,
    pub resp: Resp,
}

/// What a peripheral client is given: a whole request, decoded, from
/// whichever address channel carried it.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct PerReq<const A: usize, const I: usize> {
    pub read: Bit,
    pub id: U<I>,
    pub addr: U<A>,
    pub len: U<8>,
    pub size: U<3>,
    pub burst: BurstKind,
    pub lock: Bit,
    pub cache: U<4>,
    pub prot: U<3>,
    pub qos: U<4>,
    pub region: U<4>,
}

/// A peripheral client's answer to a write.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Answer<const I: usize> {
    pub id: U<I>,
    pub resp: Resp,
}

// ---------------------------------------------------------------------
// The two trackers, as hardware

/// The host side of a link, as a unit: the piece that turns a
/// client's issue into an address phase on the right channel, hands
/// out the identifiers so that no client writes one, passes the write
/// beats out, and reports what comes back.
///
/// The identifiers go round robin, one per burst. An identifier is
/// outstanding from its address phase until the client releases it,
/// which the client's end does when it has taken that burst's
/// answer, so an identifier is never handed out again while an
/// answer nobody has read still stands against it. A burst whose
/// turn falls on an identifier still outstanding waits, which is the
/// backpressure that bounds how many transactions are in flight.
/// Written in the lowered subset, so it is a netlist too.
// begin{hostunit}
#[derive(Trace, Default)]
pub struct AxiHost<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const NIDS: usize,
> {
    /// The identifier whose turn it is: the one the next burst takes.
    /// Not called `next`, which VHDL reserves.
    pub turn: Reg<U<I>>,
    /// Which identifiers are outstanding, a bit each.
    pub busy: Reg<U<NIDS>>,
}

#[lower]
impl<
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
        const NIDS: usize,
    > Unit for AxiHost<A, D, S, I, NIDS>
{
    async fn run(
        &mut self,
        (issue, wbeat, b, r, release): (
            Rx<Issue<A>>,
            Rx<W<D, S>>,
            Rx<B<I>>,
            Rx<R<D, I>>,
            Rx<Grant<I>>,
        ),
        (aw, ar, w, grant, done, rdata): (
            Tx<Aw<A, I>>,
            Tx<Ar<A, I>>,
            Tx<W<D, S>>,
            Tx<Grant<I>>,
            Tx<Done<I>>,
            Tx<R<D, I>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            // The identifier this burst would take, and whether it is
            // free; a burst waits for its own turn, nothing else.
            let id = self.turn.get();
            let free = !self.busy.get().bit(id.raw() as usize);
            let offered = issue.peek().is_some();
            let head = issue.head();
            let read = head.read;
            let room = mux(read, ar.ready(), aw.ready()) & grant.ready();
            let go = offered & free & room;
            let _ = issue.recv_if(free & room);
            // A write's beats go straight out. AXI4 puts no identifier
            // on W, so the beats belong to the oldest address phase,
            // which is why a client sends a burst's beats before it
            // issues the next one.
            let wr = wbeat.head();
            let wr_go = wbeat.peek().is_some() & w.ready();
            let _ = wbeat.recv_if(w.ready());
            // What comes back: a write response, and read beats.
            let bh = b.head();
            let b_go = b.peek().is_some() & done.ready();
            let _ = b.recv_if(done.ready());
            let rh = r.head();
            let r_go = r.peek().is_some() & rdata.ready();
            let _ = r.recv_if(rdata.ready());
            // An identifier is freed when the client releases it,
            // having taken that burst's answer.
            let rel = release.head();
            let rel_go = release.peek().is_some();
            let _ = release.recv_if(rel_go);
            let one = U::<NIDS>::from(1u8);
            let zero = U::<NIDS>::from(0u8);
            let id_bit = one << (id.raw() as usize);
            let rel_bit = one << (rel.id.raw() as usize);
            let freed = mux(rel_go, rel_bit, zero);
            with!(self <= {
                go ? turn: id + 1,
                busy: (self.busy.get() | mux(go, id_bit, zero)) & !freed,
            });
            if (go & read).to_bool() {
                ar.send(Ar {
                    id: id,
                    addr: head.addr,
                    len: head.len,
                    size: head.size,
                    burst: head.burst,
                    lock: head.lock,
                    cache: head.cache,
                    prot: head.prot,
                    qos: head.qos,
                    region: head.region,
                });
            }
            if (go & !read).to_bool() {
                aw.send(Aw {
                    id: id,
                    addr: head.addr,
                    len: head.len,
                    size: head.size,
                    burst: head.burst,
                    lock: head.lock,
                    cache: head.cache,
                    prot: head.prot,
                    qos: head.qos,
                    region: head.region,
                });
            }
            if go.to_bool() {
                grant.send(Grant { id: id });
            }
            if wr_go.to_bool() {
                w.send(W {
                    data: wr.data,
                    strb: wr.strb,
                    last: wr.last,
                });
            }
            if b_go.to_bool() {
                done.send(Done {
                    id: bh.id,
                    resp: bh.resp,
                });
            }
            if r_go.to_bool() {
                rdata.send(R {
                    id: rh.id,
                    data: rh.data,
                    resp: rh.resp,
                    last: rh.last,
                });
            }
        }
    }
}

// end{hostunit}

/// The peripheral side of a link, as a unit: the piece that merges
/// the two address channels into one stream of requests, hands the
/// write beats to the client, and turns the client's answers back
/// into `B` beats and `R` beats.
///
/// The two address channels take turns, so neither starves the other
/// however long one of them streams. Written in the lowered subset,
/// so it is a netlist too.
// begin{perunit}
#[derive(Trace, Default)]
pub struct AxiPer<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    /// Which address channel goes first: zero for the read channel.
    pub rr: Reg<U<1>>,
}

#[lower]
impl<const A: usize, const D: usize, const S: usize, const I: usize> Unit
    for AxiPer<A, D, S, I>
{
    async fn run(
        &mut self,
        (aw, ar, w, ans, rb): (
            Rx<Aw<A, I>>,
            Rx<Ar<A, I>>,
            Rx<W<D, S>>,
            Rx<Answer<I>>,
            Rx<R<D, I>>,
        ),
        (req, wd, b, r): (Tx<PerReq<A, I>>, Tx<W<D, S>>, Tx<B<I>>, Tx<R<D, I>>),
    ) {
        loop {
            DefaultClock::rising().await;
            // One request out per cycle, the two address channels
            // taking turns so that neither starves the other.
            let ar_off = ar.peek().is_some();
            let aw_off = aw.peek().is_some();
            let room = req.ready();
            let prefer_read = self.rr.get() == 0;
            let take_ar = ar_off & room & (prefer_read | !aw_off);
            let take_aw = aw_off & room & !take_ar;
            let rd = ar.head();
            let wa = aw.head();
            let _ = ar.recv_if(take_ar);
            let _ = aw.recv_if(take_aw);
            with!(self <= {
                take_ar ? rr: U::<1>::from(1u8),
                take_aw ? rr: U::<1>::from(0u8),
            });
            // The write beats, to the client that will answer them.
            let wr = w.head();
            let wr_go = w.peek().is_some() & wd.ready();
            let _ = w.recv_if(wd.ready());
            // The client's answers, back onto the response channels.
            let ah = ans.head();
            let a_go = ans.peek().is_some() & b.ready();
            let _ = ans.recv_if(b.ready());
            let rh = rb.head();
            let r_go = rb.peek().is_some() & r.ready();
            let _ = rb.recv_if(r.ready());
            if (take_ar | take_aw).to_bool() {
                req.send(PerReq {
                    read: take_ar,
                    id: mux(take_ar, rd.id, wa.id),
                    addr: mux(take_ar, rd.addr, wa.addr),
                    len: mux(take_ar, rd.len, wa.len),
                    size: mux(take_ar, rd.size, wa.size),
                    burst: mux(take_ar, rd.burst, wa.burst),
                    lock: mux(take_ar, rd.lock, wa.lock),
                    cache: mux(take_ar, rd.cache, wa.cache),
                    prot: mux(take_ar, rd.prot, wa.prot),
                    qos: mux(take_ar, rd.qos, wa.qos),
                    region: mux(take_ar, rd.region, wa.region),
                });
            }
            if wr_go.to_bool() {
                wd.send(W {
                    data: wr.data,
                    strb: wr.strb,
                    last: wr.last,
                });
            }
            if a_go.to_bool() {
                b.send(B {
                    id: ah.id,
                    resp: ah.resp,
                });
            }
            if r_go.to_bool() {
                r.send(R {
                    id: rh.id,
                    data: rh.data,
                    resp: rh.resp,
                    last: rh.last,
                });
            }
        }
    }
}
// end{perunit}

// ---------------------------------------------------------------------
// The link: every channel of it, made at once

/// The ports of a host tracker, what it reads.
pub type HostIn<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> = (
    Rx<Issue<A>>,
    Rx<W<D, S>>,
    Rx<B<I>>,
    Rx<R<D, I>>,
    Rx<Grant<I>>,
);

/// The ports of a host tracker, what it drives.
pub type HostOut<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> = (
    Tx<Aw<A, I>>,
    Tx<Ar<A, I>>,
    Tx<W<D, S>>,
    Tx<Grant<I>>,
    Tx<Done<I>>,
    Tx<R<D, I>>,
);

/// The ports of a peripheral tracker, what it reads.
pub type PerIn<const A: usize, const D: usize, const S: usize, const I: usize> = (
    Rx<Aw<A, I>>,
    Rx<Ar<A, I>>,
    Rx<W<D, S>>,
    Rx<Answer<I>>,
    Rx<R<D, I>>,
);

/// The ports of a peripheral tracker, what it drives.
pub type PerOut<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> = (Tx<PerReq<A, I>>, Tx<W<D, S>>, Tx<B<I>>, Tx<R<D, I>>);

/// The ends a host client holds when it is a unit rather than an
/// async client: what it drives, and what it reads. `Host` is a
/// simulation-side convenience and does not lower, so a client
/// written as hardware holds these instead. In order: the issue
/// channel, the write beats, the release, and then the grant, the
/// write responses and the read beats.
pub type HostClient<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> = (
    Tx<Issue<A>>,
    Tx<W<D, S>>,
    Tx<Grant<I>>,
    Rx<Grant<I>>,
    Rx<Done<I>>,
    Rx<R<D, I>>,
);

/// The same for a peripheral client that is a unit: the requests and
/// the write beats it reads, and the answers and read beats it
/// drives.
pub type PerClient<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> = (Rx<PerReq<A, I>>, Rx<W<D, S>>, Tx<Answer<I>>, Tx<R<D, I>>);

/// A link whose clients are units: the ports of the two trackers, and
/// the channel ends a hardware client holds on each side. [`axi`] is
/// this with the two ends wrapped in [`Host`] and [`Per`].
pub struct UnitLink<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    pub host_client: HostClient<A, D, S, I>,
    pub per_client: PerClient<A, D, S, I>,
    pub host_in: HostIn<A, D, S, I>,
    pub host_out: HostOut<A, D, S, I>,
    pub per_in: PerIn<A, D, S, I>,
    pub per_out: PerOut<A, D, S, I>,
}

/// Make a link for clients that are units. Every channel of the link
/// is made here, as [`axi`] makes them, but the two client ends are
/// handed out as the channel ends themselves.
#[allow(clippy::type_complexity)]
pub fn axi_units<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
>() -> UnitLink<A, D, S, I> {
    // The five AXI channels, host to peripheral and back.
    let (aw_tx, aw_rx) = chan::<Aw<A, I>, DefaultClock>();
    let (ar_tx, ar_rx) = chan::<Ar<A, I>, DefaultClock>();
    let (w_tx, w_rx) = chan::<W<D, S>, DefaultClock>();
    let (b_tx, b_rx) = chan::<B<I>, DefaultClock>();
    let (r_tx, r_rx) = chan::<R<D, I>, DefaultClock>();
    // The host client's channels.
    let (issue_tx, issue_rx) = chan::<Issue<A>, DefaultClock>();
    let (wbeat_tx, wbeat_rx) = chan::<W<D, S>, DefaultClock>();
    let (grant_tx, grant_rx) = chan::<Grant<I>, DefaultClock>();
    let (done_tx, done_rx) = chan::<Done<I>, DefaultClock>();
    let (rdata_tx, rdata_rx) = chan::<R<D, I>, DefaultClock>();
    let (release_tx, release_rx) = chan::<Grant<I>, DefaultClock>();
    // The peripheral client's channels.
    let (req_tx, req_rx) = chan::<PerReq<A, I>, DefaultClock>();
    let (wd_tx, wd_rx) = chan::<W<D, S>, DefaultClock>();
    let (ans_tx, ans_rx) = chan::<Answer<I>, DefaultClock>();
    let (rb_tx, rb_rx) = chan::<R<D, I>, DefaultClock>();
    UnitLink {
        host_client: (
            issue_tx, wbeat_tx, release_tx, grant_rx, done_rx, rdata_rx,
        ),
        per_client: (req_rx, wd_rx, ans_tx, rb_tx),
        host_in: (issue_rx, wbeat_rx, b_rx, r_rx, release_rx),
        host_out: (aw_tx, ar_tx, w_tx, grant_tx, done_tx, rdata_tx),
        per_in: (aw_rx, ar_rx, w_rx, ans_rx, rb_rx),
        per_out: (req_tx, wd_tx, b_tx, r_tx),
    }
}

/// A link, as [`axi`] makes it: the two client ends, and the ports of
/// the two units between them. A design joins [`AxiHost`] and
/// [`AxiPer`] on those ports, or a unit of its own with the same
/// ports, which is the seam a design uses to supply its own tracker.
pub struct Link<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const NIDS: usize,
> {
    /// The end a host client holds.
    pub host: Host<A, D, S, I, NIDS>,
    /// The end a peripheral client holds.
    pub per: Per<A, D, S, I>,
    pub host_in: HostIn<A, D, S, I>,
    pub host_out: HostOut<A, D, S, I>,
    pub per_in: PerIn<A, D, S, I>,
    pub per_out: PerOut<A, D, S, I>,
}

/// Make a link and get its ends, the way [`chan`] makes a channel and
/// gets its two. Every channel of the link is made here: the five AXI
/// channels between the trackers, and the transaction-level channels
/// between each tracker and its client.
///
/// [`chan`]: txhdl::comp::chan
#[allow(clippy::type_complexity)]
pub fn axi<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const NIDS: usize,
>() -> Link<A, D, S, I, NIDS> {
    let u = axi_units::<A, D, S, I>();
    let (issue, wbeat, release, grant, done, rdata) = u.host_client;
    let (req, wd, ans, rb) = u.per_client;
    Link {
        host: Host {
            issue,
            wbeat,
            grant,
            inbox: Rc::new(Inbox {
                done,
                rdata,
                release,
                slots: RefCell::new(vec![Slot::default(); NIDS]),
                freed: RefCell::new(VecDeque::new()),
                drained: Cell::new(u64::MAX),
            }),
        },
        per: Per {
            req,
            wd,
            port: Rc::new(Port { ans, rb }),
            gathering: Rc::new(Cell::new(false)),
        },
        host_in: u.host_in,
        host_out: u.host_out,
        per_in: u.per_in,
        per_out: u.per_out,
    }
}

// ---------------------------------------------------------------------
// The host end

/// What a client asks of a read: a burst at an address, with the
/// AXI4 fields it may set. [`Rd::at`] fills the rest with what an
/// ordinary incrementing burst of whole words uses.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rd<const A: usize> {
    pub addr: U<A>,
    /// Beats in the burst, counted as beats and not as AXI's `len`.
    pub words: usize,
    pub size: U<3>,
    pub burst: BurstKind,
    pub lock: Bit,
    pub cache: U<4>,
    pub prot: U<3>,
    pub qos: U<4>,
    pub region: U<4>,
}

impl<const A: usize> Rd<A> {
    /// A burst of `words` beats at `addr`, incrementing.
    pub fn at(addr: impl Into<U<A>>, words: usize) -> Self {
        Rd {
            addr: addr.into(),
            words,
            ..Rd::default()
        }
    }
}

/// What a client asks of a write. The burst's length is the slice it
/// is given, so it can never disagree with the beats that follow.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wr<const A: usize> {
    pub addr: U<A>,
    pub size: U<3>,
    pub burst: BurstKind,
    pub lock: Bit,
    pub cache: U<4>,
    pub prot: U<3>,
    pub qos: U<4>,
    pub region: U<4>,
}

impl<const A: usize> Wr<A> {
    /// A write at `addr`, incrementing.
    pub fn at(addr: impl Into<U<A>>) -> Self {
        Wr {
            addr: addr.into(),
            ..Wr::default()
        }
    }
}

/// An answer to a burst: the response, and the words a read read.
#[derive(Clone, Debug, Default)]
pub struct Reply<const D: usize> {
    pub resp: Resp,
    pub data: Vec<U<D>>,
}

/// One identifier's answer, as it is gathered.
#[derive(Clone, Default)]
struct Slot<const D: usize> {
    resp: Resp,
    data: Vec<U<D>>,
    finished: bool,
}

/// Where answers land until the client asks for them. A handle's wait
/// drains whatever the tracker offered into this, so an answer that
/// arrives while another burst is being awaited is kept rather than
/// held at the head of a channel: that is what lets a client await
/// its bursts in any order.
struct Inbox<const D: usize, const I: usize> {
    done: Rx<Done<I>>,
    rdata: Rx<R<D, I>>,
    release: Tx<Grant<I>>,
    slots: RefCell<Vec<Slot<D>>>,
    /// Identifiers whose answers have been taken, waiting to be handed
    /// back to the tracker.
    freed: RefCell<VecDeque<U<I>>>,
    drained: Cell<u64>,
}

impl<const D: usize, const I: usize> Inbox<D, I> {
    /// Take what the tracker offers this step, once per step however
    /// many handles ask.
    fn drain(&self) {
        if self.drained.get() == now() {
            return;
        }
        self.drained.set(now());
        let mut s = self.slots.borrow_mut();
        if let Some(d) = self.done.recv() {
            let k = d.id.raw() as usize;
            s[k].resp = d.resp;
            s[k].finished = true;
        }
        if let Some(b) = self.rdata.recv() {
            let k = b.id.raw() as usize;
            s[k].data.push(b.data);
            s[k].resp = b.resp;
            s[k].finished = b.last.to_bool();
        }
        drop(s);
        // Hand back one identifier a cycle, oldest first.
        let head = {
            let q = self.freed.borrow();
            q.front().copied()
        };
        if let Some(id) = head {
            if self.release.ready().to_bool() {
                self.release.send(Grant { id });
                self.freed.borrow_mut().pop_front();
            }
        }
    }

    fn take(&self, id: usize) -> Option<Reply<D>> {
        let mut s = self.slots.borrow_mut();
        if !s[id].finished {
            return None;
        }
        let slot = std::mem::take(&mut s[id]);
        drop(s);
        self.freed.borrow_mut().push_back(U::from(id));
        Some(Reply {
            resp: slot.resp,
            data: slot.data,
        })
    }
}

/// A burst in flight: the identifier the tracker gave it, and the
/// answer to come. Several may be in flight at once and they may be
/// awaited in any order, which is what out of order means here.
pub struct Pending<const D: usize, const I: usize> {
    id: U<I>,
    inbox: Rc<Inbox<D, I>>,
}

impl<const D: usize, const I: usize> Pending<D, I> {
    /// The identifier the tracker gave this burst. A client needs it
    /// for nothing; it is here to be printed and traced.
    pub fn id(&self) -> U<I> {
        self.id
    }

    /// Wait for this burst's answer. Awaiting several of these inside
    /// `parallel!` is how a client keeps them all moving; awaiting
    /// them one after another in an order of its own works too,
    /// because an answer that arrives early waits in the inbox.
    pub async fn done(self) -> Reply<D> {
        loop {
            DefaultClock::rising().await;
            self.inbox.drain();
            if let Some(r) = self.inbox.take(self.id.raw() as usize) {
                return r;
            }
        }
    }
}

/// The host end of a link: what a client that issues bursts holds.
/// One process issues on it, since a burst's identifier is told back
/// in the cycle the burst goes out.
pub struct Host<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const NIDS: usize,
> {
    issue: Tx<Issue<A>>,
    wbeat: Tx<W<D, S>>,
    grant: Rx<Grant<I>>,
    inbox: Rc<Inbox<D, I>>,
}

// begin{hostapi}
impl<
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
        const NIDS: usize,
    > Host<A, D, S, I, NIDS>
{
    /// Issue a read burst. Returns when the burst has gone out and
    /// the tracker has said which identifier it took, which is the
    /// point from which another burst may be issued.
    pub async fn read(&self, rd: Rd<A>) -> Pending<D, I> {
        let len = rd.words.saturating_sub(1) as u8;
        let id = self
            .issue(Issue {
                read: Bit::One,
                addr: rd.addr,
                len: U::from(len),
                size: rd.size,
                burst: rd.burst,
                lock: rd.lock,
                cache: rd.cache,
                prot: rd.prot,
                qos: rd.qos,
                region: rd.region,
            })
            .await;
        Pending {
            id,
            inbox: self.inbox.clone(),
        }
    }

    /// Issue a write burst and send its beats. The burst's length is
    /// `data`, and the last beat is marked as the last, so a client
    /// never writes `len` or `last`. The beats go out before this
    /// returns, which is what keeps two writes in flight in the order
    /// AXI4 requires of the write data channel.
    pub async fn write(&self, wr: Wr<A>, data: &[U<D>]) -> Pending<D, I> {
        assert!(!data.is_empty(), "a write burst with no beats");
        let len = (data.len() - 1) as u8;
        let id = self
            .issue(Issue {
                read: Bit::Zero,
                addr: wr.addr,
                len: U::from(len),
                size: wr.size,
                burst: wr.burst,
                lock: wr.lock,
                cache: wr.cache,
                prot: wr.prot,
                qos: wr.qos,
                region: wr.region,
            })
            .await;
        let strb = !U::<S>::from(0u8);
        for (i, d) in data.iter().enumerate() {
            let beat = W {
                data: *d,
                strb,
                last: Bit::from_bool(i + 1 == data.len()),
            };
            loop {
                DefaultClock::rising().await;
                self.inbox.drain();
                if self.wbeat.ready().to_bool() {
                    self.wbeat.send(beat);
                    break;
                }
            }
        }
        Pending {
            id,
            inbox: self.inbox.clone(),
        }
    }

    /// Offer the address phase, then take the identifier the tracker
    /// allocated for it. Answers keep arriving while this waits.
    async fn issue(&self, iss: Issue<A>) -> U<I> {
        loop {
            DefaultClock::rising().await;
            self.inbox.drain();
            if self.issue.ready().to_bool() {
                self.issue.send(iss);
                break;
            }
        }
        loop {
            DefaultClock::rising().await;
            self.inbox.drain();
            if let Some(g) = self.grant.recv() {
                return g.id;
            }
        }
    }
}

// end{hostapi}

// ---------------------------------------------------------------------
// The peripheral end

/// Where a peripheral client's answers go. An accepted transaction
/// holds one of these, so it may be moved to another process and
/// answered there; the channels inside stay unique, so there is still
/// one driver on each.
struct Port<const D: usize, const I: usize> {
    ans: Tx<Answer<I>>,
    rb: Tx<R<D, I>>,
}

/// A write, accepted whole: its address phase and every beat of its
/// data. It answers itself, and it may be answered from a process
/// other than the one that accepted it.
pub struct WriteXact<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    req: PerReq<A, I>,
    data: Vec<U<D>>,
    strb: Vec<U<S>>,
    port: Rc<Port<D, I>>,
}

/// A read, accepted: its address phase. It answers itself with the
/// words it asked for, and it may be answered from another process.
pub struct ReadXact<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
> {
    req: PerReq<A, I>,
    port: Rc<Port<D, I>>,
}

/// One accepted AXI transaction, decoded. Nothing of the protocol is
/// left in it: the beats of a write are gathered, the identifier is
/// carried, and answering is one call.
pub enum Xact<const A: usize, const D: usize, const S: usize, const I: usize> {
    Write(WriteXact<A, D, S, I>),
    Read(ReadXact<A, D, S, I>),
}

impl<const A: usize, const D: usize, const S: usize, const I: usize>
    Xact<A, D, S, I>
{
    /// The identifier, for a client that wants to hold transactions
    /// in a table of its own. Answering needs no identifier.
    pub fn id(&self) -> U<I> {
        match self {
            Xact::Write(w) => w.req.id,
            Xact::Read(r) => r.req.id,
        }
    }

    /// The address the burst starts at.
    pub fn addr(&self) -> U<A> {
        match self {
            Xact::Write(w) => w.req.addr,
            Xact::Read(r) => r.req.addr,
        }
    }

    /// Beats in the burst, counted as beats.
    pub fn words(&self) -> usize {
        match self {
            Xact::Write(w) => w.data.len(),
            Xact::Read(r) => r.words(),
        }
    }
}

impl<const A: usize, const D: usize, const S: usize, const I: usize>
    WriteXact<A, D, S, I>
{
    pub fn id(&self) -> U<I> {
        self.req.id
    }
    pub fn addr(&self) -> U<A> {
        self.req.addr
    }
    /// The whole burst's data, gathered before this was handed over.
    pub fn data(&self) -> &[U<D>] {
        &self.data
    }
    /// The lane enables, one per beat.
    pub fn strb(&self) -> &[U<S>] {
        &self.strb
    }
    /// The address phase, for the AXI4 fields a client may care
    /// about: `size`, `burst`, `prot` and the rest.
    pub fn req(&self) -> PerReq<A, I> {
        self.req
    }

    /// Answer it: the write took.
    pub async fn ok(self) {
        self.answer(Resp::Okay).await
    }

    /// Answer it with an error.
    pub async fn err(self, resp: Resp) {
        self.answer(resp).await
    }

    async fn answer(self, resp: Resp) {
        loop {
            DefaultClock::rising().await;
            if self.port.ans.ready().to_bool() {
                self.port.ans.send(Answer {
                    id: self.req.id,
                    resp,
                });
                return;
            }
        }
    }
}

impl<const A: usize, const D: usize, const S: usize, const I: usize>
    ReadXact<A, D, S, I>
{
    pub fn id(&self) -> U<I> {
        self.req.id
    }
    pub fn addr(&self) -> U<A> {
        self.req.addr
    }
    /// Beats asked for, counted as beats and not as AXI's `len`.
    pub fn words(&self) -> usize {
        self.req.len.raw() as usize + 1
    }
    /// The address phase, for the AXI4 fields a client may care about.
    pub fn req(&self) -> PerReq<A, I> {
        self.req
    }

    /// Answer it with the words it asked for. The beats go out one a
    /// cycle and the last is marked as the last, which the client
    /// neither writes nor counts.
    pub async fn data(self, words: &[U<D>]) {
        assert_eq!(
            words.len(),
            self.words(),
            "a read of {} beats answered with {}",
            self.words(),
            words.len()
        );
        self.beats(words, Resp::Okay).await
    }

    /// Answer it with an error, in as many beats as it asked for.
    pub async fn err(self, resp: Resp) {
        let zero = vec![U::<D>::from(0u8); self.words()];
        self.beats(&zero, resp).await
    }

    async fn beats(self, words: &[U<D>], resp: Resp) {
        for (i, d) in words.iter().enumerate() {
            let beat = R {
                id: self.req.id,
                data: *d,
                resp,
                last: Bit::from_bool(i + 1 == words.len()),
            };
            loop {
                DefaultClock::rising().await;
                if self.port.rb.ready().to_bool() {
                    self.port.rb.send(beat);
                    break;
                }
            }
        }
    }
}

/// The peripheral end of a link: what a client that answers bursts
/// holds. Several processes may share it, each accepting when it is
/// free, which is how a peripheral keeps more than one transaction
/// open at a time.
pub struct Per<const A: usize, const D: usize, const S: usize, const I: usize> {
    req: Rx<PerReq<A, I>>,
    wd: Rx<W<D, S>>,
    port: Rc<Port<D, I>>,
    gathering: Rc<Cell<bool>>,
}

// begin{accept}
impl<const A: usize, const D: usize, const S: usize, const I: usize>
    Per<A, D, S, I>
{
    /// Wait for a whole AXI transaction. A read is ready as soon as
    /// its address phase arrives; a write is ready when its last beat
    /// has, since AXI4 puts no identifier on the write data channel
    /// and the beats belong to the oldest address phase. One accept
    /// gathers at a time, so several processes may share an end
    /// without taking each other's beats.
    pub async fn accept(&self) -> Xact<A, D, S, I> {
        let req = loop {
            DefaultClock::rising().await;
            if !self.gathering.get() {
                if let Some(q) = self.req.recv() {
                    break q;
                }
            }
        };
        if req.read.to_bool() {
            return Xact::Read(ReadXact {
                req,
                port: self.port.clone(),
            });
        }
        self.gathering.set(true);
        let n = req.len.raw() as usize + 1;
        let mut data = Vec::with_capacity(n);
        let mut strb = Vec::with_capacity(n);
        while data.len() < n {
            DefaultClock::rising().await;
            if let Some(w) = self.wd.recv() {
                data.push(w.data);
                strb.push(w.strb);
            }
        }
        self.gathering.set(false);
        Xact::Write(WriteXact {
            req,
            data,
            strb,
            port: self.port.clone(),
        })
    }
}

// end{accept}

/// Transactions a client has accepted and not answered, by
/// identifier: what a client keeps when the process that accepts is
/// not the process that answers.
pub struct Open<const A: usize, const D: usize, const S: usize, const I: usize>(
    Rc<RefCell<HashMap<u128, Xact<A, D, S, I>>>>,
);

impl<const A: usize, const D: usize, const S: usize, const I: usize> Clone
    for Open<A, D, S, I>
{
    fn clone(&self) -> Self {
        Open(self.0.clone())
    }
}

impl<const A: usize, const D: usize, const S: usize, const I: usize> Default
    for Open<A, D, S, I>
{
    fn default() -> Self {
        Open(Rc::new(RefCell::new(HashMap::new())))
    }
}

impl<const A: usize, const D: usize, const S: usize, const I: usize>
    Open<A, D, S, I>
{
    pub fn new() -> Self {
        Self::default()
    }
    /// Keep a transaction until its answer is ready.
    pub fn put(&self, x: Xact<A, D, S, I>) {
        self.0.borrow_mut().insert(x.id().raw(), x);
    }
    /// Take back the transaction an answer belongs to.
    pub fn take(&self, id: impl Into<U<I>>) -> Option<Xact<A, D, S, I>> {
        self.0.borrow_mut().remove(&id.into().raw())
    }
    /// How many are open.
    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Serve a peripheral end with `slots` processes, each accepting a
/// transaction when it is free and answering it when `f` has
/// finished with it. Several are in flight at once and each is
/// answered as it finishes, so the answers leave in the order the
/// work finished and not the order it arrived.
///
/// This is [`txhdl::pipeline::drive`] for a bus: the client writes
/// the body and nothing of the protocol.
// begin{serve}
pub async fn serve<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    F,
    Fut,
>(
    per: Per<A, D, S, I>,
    slots: usize,
    f: F,
) where
    F: Fn(Xact<A, D, S, I>) -> Fut,
    Fut: Future<Output = ()>,
{
    let per = Rc::new(per);
    let f = Rc::new(f);
    join_all((0..slots).map(|_| {
        let per = per.clone();
        let f = f.clone();
        async move {
            loop {
                let x = per.accept().await;
                f(x).await;
            }
        }
    }))
    .await
}
// end{serve}

// ---------------------------------------------------------------------
// The link against the rule written with loops

/// A link driven by clients on both ends, checked against a model of
/// what AXI promises: every burst is answered, a read reads what the
/// writes left, the answers may come back in any order, and a
/// transaction may be answered by a process other than the one that
/// accepted it.
#[cfg(test)]
mod tests {
    use super::*;
    use txhdl::comp::{join2, Running};

    /// The link of the tests: sixteen-bit addresses, thirty-two-bit
    /// words, four lanes, two-bit identifiers, four of them.
    type TestHost = AxiHost<16, 32, 4, 2, 4>;
    type TestPer = AxiPer<16, 32, 4, 2>;
    type TestXact = Xact<16, 32, 4, 2>;

    /// A memory the peripheral clients answer from: the model of what
    /// a read must read.
    #[derive(Clone, Default)]
    struct Model(Rc<RefCell<HashMap<u128, u128>>>);

    impl Model {
        fn put(&self, addr: u128, i: usize, v: u128) {
            self.0.borrow_mut().insert(addr + i as u128, v);
        }
        fn words(&self, addr: u128, n: usize) -> Vec<U<32>> {
            (0..n)
                .map(|i| {
                    let m = self.0.borrow();
                    U::from(*m.get(&(addr + i as u128)).unwrap_or(&0) as u32)
                })
                .collect()
        }
    }

    /// Drive a link with the two clients given, for `cycles` cycles.
    fn run(
        host_in: HostIn<16, 32, 4, 2>,
        host_out: HostOut<16, 32, 4, 2>,
        per_in: PerIn<16, 32, 4, 2>,
        per_out: PerOut<16, 32, 4, 2>,
        client: impl Future<Output = ()>,
        server: impl Future<Output = ()>,
        cycles: usize,
    ) {
        let mut h = TestHost::default();
        let mut p = TestPer::default();
        let mut sim = Running::new(join2(
            join2(h.run(host_in, host_out), p.run(per_in, per_out)),
            join2(client, server),
        ));
        for _ in 0..cycles {
            sim.cycle();
        }
    }

    /// A peripheral client that answers one transaction at a time
    /// from `model`, for as long as the run lasts.
    async fn memory(per: Per<16, 32, 4, 2>, model: Model) {
        loop {
            match per.accept().await {
                Xact::Write(w) => {
                    let addr = w.addr().raw();
                    for (i, d) in w.data().iter().enumerate() {
                        model.put(addr, i, d.raw());
                    }
                    w.ok().await;
                }
                Xact::Read(r) => {
                    let words = model.words(r.addr().raw(), r.words());
                    r.data(&words).await;
                }
            }
        }
    }

    #[test]
    fn a_read_reads_what_the_write_wrote() {
        let Link {
            host,
            per,
            host_in,
            host_out,
            per_in,
            per_out,
        } = axi::<16, 32, 4, 2, 4>();
        let out = Rc::new(RefCell::new(Vec::new()));
        let o = out.clone();
        let client = async move {
            let words = [U::from(0x11u32), U::from(0x22u32), U::from(0x33u32)];
            let w = host.write(Wr::at(0x100u32), &words).await;
            let r = host.read(Rd::at(0x100u32, 3)).await;
            let wr = w.done().await;
            let rd = r.done().await;
            o.borrow_mut().push((wr.resp, rd.resp, rd.data));
        };
        run(
            host_in,
            host_out,
            per_in,
            per_out,
            client,
            memory(per, Model::default()),
            80,
        );
        let got = out.borrow();
        assert_eq!(got.len(), 1, "the burst was never answered");
        let (wresp, rresp, data) = &got[0];
        assert_eq!(*wresp, Resp::Okay);
        assert_eq!(*rresp, Resp::Okay);
        let raw: Vec<u128> = data.iter().map(|d| d.raw()).collect();
        assert_eq!(raw, vec![0x11, 0x22, 0x33]);
    }

    /// Three bursts in flight, answered by a peripheral that holds
    /// them open and answers the last one first. The client awaits
    /// them in issue order and still gets each its own answer, which
    /// is what the identifiers are for.
    #[test]
    fn answers_come_back_out_of_order() {
        let Link {
            host,
            per,
            host_in,
            host_out,
            per_in,
            per_out,
        } = axi::<16, 32, 4, 2, 4>();
        let model = Model::default();
        for (i, v) in [0xa0u128, 0xb0, 0xc0].iter().enumerate() {
            model.put(0x200 + i as u128, 0, *v);
        }
        let out = Rc::new(RefCell::new(Vec::new()));
        let o = out.clone();
        let client = async move {
            let r0 = host.read(Rd::at(0x200u32, 1)).await;
            let r1 = host.read(Rd::at(0x201u32, 1)).await;
            let r2 = host.read(Rd::at(0x202u32, 1)).await;
            let ids = (r0.id(), r1.id(), r2.id());
            let a0 = r0.done().await;
            let a1 = r1.done().await;
            let a2 = r2.done().await;
            o.borrow_mut()
                .push((ids, a0.data[0], a1.data[0], a2.data[0]));
        };
        // The order the answers leave in: the third, the first, the
        // second, however they arrived.
        let order = Rc::new(RefCell::new(Vec::new()));
        let seen = order.clone();
        let server = async move {
            let open = Open::<16, 32, 4, 2>::new();
            let mut held: Vec<u128> = Vec::new();
            // Accept all three, then answer them in a shuffled order.
            for _ in 0..3 {
                let x = per.accept().await;
                held.push(x.id().raw());
                open.put(x);
            }
            for k in [2usize, 0, 1] {
                if let Some(Xact::Read(r)) = open.take(U::<2>::new(held[k])) {
                    let words = model.words(r.addr().raw(), r.words());
                    seen.borrow_mut().push(held[k]);
                    r.data(&words).await;
                }
            }
        };
        run(host_in, host_out, per_in, per_out, client, server, 120);
        let got = out.borrow();
        assert_eq!(got.len(), 1, "the bursts were never all answered");
        let ((i0, i1, i2), d0, d1, d2) = &got[0];
        assert_ne!(i0.raw(), i1.raw(), "two bursts took one identifier");
        assert_ne!(i1.raw(), i2.raw(), "two bursts took one identifier");
        assert_eq!((d0.raw(), d1.raw(), d2.raw()), (0xa0, 0xb0, 0xc0));
        assert_eq!(
            *order.borrow(),
            vec![i2.raw(), i0.raw(), i1.raw()],
            "the peripheral did not answer out of order"
        );
    }

    /// A transaction accepted in one process and answered in another,
    /// which is what a peripheral that forwards its work does.
    #[test]
    fn a_transaction_is_answered_by_another_process() {
        let Link {
            host,
            per,
            host_in,
            host_out,
            per_in,
            per_out,
        } = axi::<16, 32, 4, 2, 4>();
        let model = Model::default();
        model.put(0x300, 0, 0x5eed);
        let out = Rc::new(RefCell::new(Vec::new()));
        let o = out.clone();
        let client = async move {
            let r = host.read(Rd::at(0x300u32, 1)).await;
            let a = r.done().await;
            o.borrow_mut().push(a.data[0].raw());
        };
        let open = Open::<16, 32, 4, 2>::new();
        let (taker, giver) = (open.clone(), open.clone());
        let accept = async move {
            loop {
                let x = per.accept().await;
                taker.put(x);
            }
        };
        let answer = async move {
            // A worker that takes its time, then answers whatever is
            // open, from a process that never called accept.
            loop {
                DefaultClock::rising().await;
                let ids: Vec<u128> = giver.0.borrow().keys().copied().collect();
                if let Some(id) = ids.first() {
                    if let Some(Xact::Read(r)) = giver.take(U::<2>::new(*id)) {
                        let words = model.words(r.addr().raw(), r.words());
                        r.data(&words).await;
                    }
                }
            }
        };
        let server = join2(accept, answer);
        run(host_in, host_out, per_in, per_out, client, server, 80);
        assert_eq!(*out.borrow(), vec![0x5eed], "no answer came back");
    }

    /// A soak: bursts of one to four words at pseudorandom addresses,
    /// reads and writes mixed, served by `serve` with four slots, so
    /// several are open at once. Every read must read what the last
    /// write to that address left.
    #[test]
    fn every_burst_is_answered_and_reads_agree_with_writes() {
        let Link {
            host,
            per,
            host_in,
            host_out,
            per_in,
            per_out,
        } = axi::<16, 32, 4, 2, 4>();
        let model = Model::default();
        let shadow = model.clone();
        let out = Rc::new(RefCell::new(Vec::<(u128, Vec<u128>)>::new()));
        let o = out.clone();
        let asked = Rc::new(RefCell::new(Vec::<(u128, Vec<u128>)>::new()));
        let a = asked.clone();
        let client = async move {
            let mut x = 0x2545_f491u32;
            let mut next = 1u32;
            for _ in 0..24 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let words = (x as usize % 4) + 1;
                let addr = ((x >> 8) % 8) as u32 * 4;
                if x & 0x40 == 0 {
                    let vs: Vec<U<32>> =
                        (0..words).map(|i| U::from(next + i as u32)).collect();
                    next += words as u32;
                    // What this write leaves behind, for the check.
                    a.borrow_mut().push((
                        u128::MAX,
                        vs.iter().map(|v| v.raw()).collect(),
                    ));
                    let w = host.write(Wr::at(addr), &vs).await;
                    let r = w.done().await;
                    assert_eq!(r.resp, Resp::Okay, "a write was refused");
                } else {
                    let want: Vec<u128> = shadow
                        .words(addr as u128, words)
                        .iter()
                        .map(|v| v.raw())
                        .collect();
                    let r = host.read(Rd::at(addr, words)).await;
                    let got = r.done().await;
                    assert_eq!(got.resp, Resp::Okay, "a read was refused");
                    o.borrow_mut().push((
                        addr as u128,
                        got.data.iter().map(|v| v.raw()).collect(),
                    ));
                    assert_eq!(
                        got.data.iter().map(|v| v.raw()).collect::<Vec<_>>(),
                        want,
                        "a read of {words} words at {addr:#x} disagreed"
                    );
                }
            }
        };
        let server = serve(per, 4, move |x: TestXact| {
            let model = model.clone();
            async move {
                match x {
                    Xact::Write(w) => {
                        let addr = w.addr().raw();
                        for (i, d) in w.data().iter().enumerate() {
                            model.put(addr, i, d.raw());
                        }
                        w.ok().await;
                    }
                    Xact::Read(r) => {
                        let words = model.words(r.addr().raw(), r.words());
                        r.data(&words).await;
                    }
                }
            }
        });
        run(host_in, host_out, per_in, per_out, client, server, 1200);
        assert!(
            out.borrow().len() >= 6,
            "too few reads completed: {}",
            out.borrow().len()
        );
        assert!(!asked.borrow().is_empty(), "no writes were made at all");
    }
}
