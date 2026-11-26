// SPDX-License-Identifier: Apache-2.0
//! The arbiter and the router in two halves each (issue 2077). Two
//! hosts reach one peripheral through an arbiter and a router, twice:
//! in `Path` the units are whole and every channel between them is
//! registered, as every channel is by default; in `PathU` each unit is
//! its request half and its answer half, joined in the order of the
//! path, and every channel between them is marked `#[unregistered]`,
//! the answers' as well as the requests'.
//!
//! The whole units cannot be joined that way. The receiver of an
//! unregistered channel has to run after its sender in the step, and a
//! router that carries the requests down and the answers back would
//! have to run both before the peripheral's tracker and after it. Its
//! request half runs before and its answer half after, and the same
//! goes for the arbiter and for the tracker, `AxiPer`, which has had
//! its two halves since issue 1755.
//!
//! Both paths get the same three bursts: a read of the peripheral, a
//! write to it, and a read of an address that is nobody's, which the
//! router answers itself. The unregistered path answers each sooner,
//! the read of the peripheral by the four hops it no longer waits at:
//! arbiter to router and router to tracker on the way down, and the
//! same two on the way back. The channels at the path's own ports stay
//! registered here.
//!
//! An unregistered hop is not free in time: on the unregistered path a
//! request's address is decoded twice and an answer chosen twice
//! within one cycle. Which hops a design can afford is its timing's to
//! say; the halves only make the choice possible.
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, Clock, DefaultClock, Running, Rx, Tx, Unit,
};
use txhdl::map::AddrMap;
use txhdl::types::U;
use txhdl::{lower, Trace};
use txhdl_parts::bus::arbiter::{Arbiter2, ArbiterReq, ArbiterRsp};
use txhdl_parts::bus::axi::{
    axi, serve, Answer, Ar, Aw, AxiHost, AxiPer, AxiPerReq, AxiPerRsp, Host,
    Link, Per, PerReq, Rd, Resp, Wr, Xact, B, R, W,
};
use txhdl_parts::bus::router::{Router, RouterReq, RouterRsp};

/// A host's link: sixteen-bit addresses, thirty-two-bit words, two-bit
/// identifiers. Past the arbiter an identifier has a third bit, the
/// host's number.
type HostUnit = AxiHost![ADDR = 16, DATA = 32, STRB = 4, ID = 2, IDS = 4];
type Client = Host![ADDR = 16, DATA = 32, STRB = 4, ID = 2, IDS = 4];
type End = Per![ADDR = 16, DATA = 32, STRB = 4, ID = 3];
type Job = Xact<16, 32, 4, 3>;

/// One peripheral, a nibble of the address space; the rest is a hole.
pub struct OneMap;

impl AddrMap<1> for OneMap {
    const RANGES: [(usize, usize); 1] = [(0x1000, 0xf000)];
}

type Arb = Arbiter2![
    ADDR = 16,
    DATA = 32,
    STRB = 4,
    ID = 2,
    PER_ID = 3,
    FIXED = 0
];
type Rtr = Router![
    PORTS = 1,
    Map = OneMap,
    ADDR = 16,
    DATA = 32,
    STRB = 4,
    ID = 3
];
type Trk = AxiPer![ADDR = 16, DATA = 32, STRB = 4, ID = 3];
type ArbReq = ArbiterReq![
    HOSTS = 2,
    ADDR = 16,
    DATA = 32,
    STRB = 4,
    ID = 2,
    PER_ID = 3,
    FIXED = 0
];
type ArbRsp = ArbiterRsp![HOSTS = 2, DATA = 32, ID = 2, PER_ID = 3];
type RtrReq = RouterReq![
    PORTS = 1,
    Map = OneMap,
    ADDR = 16,
    DATA = 32,
    STRB = 4,
    ID = 3
];
type RtrRsp = RouterRsp![PORTS = 1, DATA = 32, ID = 3];
type TrkReq = AxiPerReq![ADDR = 16, DATA = 32, STRB = 4, ID = 3];
type TrkRsp = AxiPerRsp![ADDR = 16, DATA = 32, STRB = 4, ID = 3];

// begin{paths}
/// The path with its units whole: every channel inside registered.
#[derive(Trace, Default)]
pub struct Path {
    pub arb: Arb,
    pub rtr: Rtr,
    pub trk: Trk,
}

#[lower]
impl Unit for Path {
    async fn run(
        &mut self,
        (aw0, ar0, w0, aw1, ar1, w1, ans, rb): (
            Rx<Aw<16, 2>>,
            Rx<Ar<16, 2>>,
            Rx<W<32, 4>>,
            Rx<Aw<16, 2>>,
            Rx<Ar<16, 2>>,
            Rx<W<32, 4>>,
            Rx<Answer<3>>,
            Rx<R<32, 3>>,
        ),
        (b0, r0, b1, r1, req, wd): (
            Tx<B<2>>,
            Tx<R<32, 2>>,
            Tx<B<2>>,
            Tx<R<32, 2>>,
            Tx<PerReq<16, 3>>,
            Tx<W<32, 4>>,
        ),
    ) {
        let (xaw_tx, xaw_rx) = chan::<Aw<16, 3>, DefaultClock>();
        let (xar_tx, xar_rx) = chan::<Ar<16, 3>, DefaultClock>();
        let (xw_tx, xw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (xb_tx, xb_rx) = chan::<B<3>, DefaultClock>();
        let (xr_tx, xr_rx) = chan::<R<32, 3>, DefaultClock>();
        let (paw_tx, paw_rx) = chan::<Aw<16, 3>, DefaultClock>();
        let (par_tx, par_rx) = chan::<Ar<16, 3>, DefaultClock>();
        let (pw_tx, pw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (pb_tx, pb_rx) = chan::<B<3>, DefaultClock>();
        let (pr_tx, pr_rx) = chan::<R<32, 3>, DefaultClock>();
        join2(
            self.arb.run(
                ([aw0, aw1], [ar0, ar1], [w0, w1], xb_rx, xr_rx),
                (xaw_tx, xar_tx, xw_tx, [b0, b1], [r0, r1]),
            ),
            join2(
                self.rtr.run(
                    (xaw_rx, xar_rx, xw_rx, [pb_rx], [pr_rx]),
                    ([paw_tx], [par_tx], [pw_tx], xb_tx, xr_tx),
                ),
                self.trk.run(
                    (paw_rx, par_rx, pw_rx, ans, rb),
                    (req, wd, pb_tx, pr_tx),
                ),
            ),
        )
        .await;
    }
}

/// The same path with each unit in its two halves, joined in the
/// order a request and its answer pass them, so that every channel
/// inside can be unregistered: each sender before its receiver, the
/// request halves on the way down and the answer halves on the way
/// back. `eb` and `er` carry the router's own answers, for an address
/// that is nobody's, from its request half to its answer half.
#[derive(Trace, Default)]
pub struct PathU {
    pub areq: ArbReq,
    pub rreq: RtrReq,
    pub treq: TrkReq,
    pub trsp: TrkRsp,
    pub rrsp: RtrRsp,
    pub arsp: ArbRsp,
}

#[lower]
impl Unit for PathU {
    async fn run(
        &mut self,
        (aw0u, ar0u, w0u, aw1u, ar1u, w1u, ansu, rbu): (
            Rx<Aw<16, 2>>,
            Rx<Ar<16, 2>>,
            Rx<W<32, 4>>,
            Rx<Aw<16, 2>>,
            Rx<Ar<16, 2>>,
            Rx<W<32, 4>>,
            Rx<Answer<3>>,
            Rx<R<32, 3>>,
        ),
        (b0u, r0u, b1u, r1u, requ, wdu): (
            Tx<B<2>>,
            Tx<R<32, 2>>,
            Tx<B<2>>,
            Tx<R<32, 2>>,
            Tx<PerReq<16, 3>>,
            Tx<W<32, 4>>,
        ),
    ) {
        #[unregistered]
        let (xaw_tx, xaw_rx) = chan::<Aw<16, 3>, DefaultClock>();
        #[unregistered]
        let (xar_tx, xar_rx) = chan::<Ar<16, 3>, DefaultClock>();
        #[unregistered]
        let (xw_tx, xw_rx) = chan::<W<32, 4>, DefaultClock>();
        #[unregistered]
        let (xb_tx, xb_rx) = chan::<B<3>, DefaultClock>();
        #[unregistered]
        let (xr_tx, xr_rx) = chan::<R<32, 3>, DefaultClock>();
        #[unregistered]
        let (paw_tx, paw_rx) = chan::<Aw<16, 3>, DefaultClock>();
        #[unregistered]
        let (par_tx, par_rx) = chan::<Ar<16, 3>, DefaultClock>();
        #[unregistered]
        let (pw_tx, pw_rx) = chan::<W<32, 4>, DefaultClock>();
        #[unregistered]
        let (pb_tx, pb_rx) = chan::<B<3>, DefaultClock>();
        #[unregistered]
        let (pr_tx, pr_rx) = chan::<R<32, 3>, DefaultClock>();
        #[unregistered]
        let (eb_tx, eb_rx) = chan::<B<3>, DefaultClock>();
        #[unregistered]
        let (er_tx, er_rx) = chan::<R<32, 3>, DefaultClock>();
        join2(
            join2(
                self.areq.run(
                    ([aw0u, aw1u], [ar0u, ar1u], [w0u, w1u]),
                    (xaw_tx, xar_tx, xw_tx),
                ),
                join2(
                    self.rreq.run(
                        (xaw_rx, xar_rx, xw_rx),
                        ([paw_tx], [par_tx], [pw_tx], eb_tx, er_tx),
                    ),
                    self.treq.run((paw_rx, par_rx, pw_rx), (requ, wdu)),
                ),
            ),
            join2(
                self.trsp.run((ansu, rbu), (pb_tx, pr_tx)),
                join2(
                    self.rrsp
                        .run(([pb_rx], [pr_rx], eb_rx, er_rx), (xb_tx, xr_tx)),
                    self.arsp.run((xb_rx, xr_rx), ([b0u, b1u], [r0u, r1u])),
                ),
            ),
        )
        .await;
    }
}
// end{paths}

/// The peripheral's client: a read is answered with its own address in
/// every word, a write with `Okay`.
fn memory(per: End) -> impl Future<Output = ()> {
    serve(per, 2, move |x: Job| async move {
        match x {
            Xact::Read(rd) => {
                let addr = rd.addr().raw();
                let words: Vec<U<32>> = (0..rd.words())
                    .map(|i| U::from((addr + i as u128 * 4) as u32))
                    .collect();
                rd.data(&words).await
            }
            Xact::Write(wr) => wr.ok().await,
        }
    })
}

/// What a path's clients saw: each burst and the cycle its answer was
/// whole.
type Log = Rc<RefCell<Vec<(&'static str, u64)>>>;

/// The two hosts' clients, the same for both paths. The first host
/// reads the peripheral; the second writes it and then reads an
/// address that is nobody's.
fn clients(h0: Client, h1: Client, log: Log) -> impl Future<Output = ()> {
    let (l0, l1) = (log.clone(), log);
    let first = async move {
        let p = h0.read(Rd::at(0x1008u32, 1)).await;
        let r = p.done().await;
        assert_eq!(r.resp, Resp::Okay, "the read of the peripheral");
        assert_eq!(r.data[0].raw(), 0x1008, "answered by the peripheral");
        l0.borrow_mut()
            .push(("host 0 read 0x1008", now() / DefaultClock::PERIOD));
    };
    let second = async move {
        let w = h1.write(Wr::at(0x1004u32), &[U::from(0xa5u32)]).await;
        assert_eq!(w.done().await.resp, Resp::Okay, "the write");
        l1.borrow_mut()
            .push(("host 1 write 0x1004", now() / DefaultClock::PERIOD));
        let p = h1.read(Rd::at(0x8000u32, 2)).await;
        let r = p.done().await;
        assert_eq!(r.resp, Resp::DecErr, "a read of a hole");
        assert_eq!(r.data.len(), 2, "in the beats it asked for");
        l1.borrow_mut()
            .push(("host 1 read 0x8000", now() / DefaultClock::PERIOD));
    };
    join2(first, second)
}

fn main() {
    // The registered path: a link per host, whose peripheral-facing
    // ends are the path's ports, and the peripheral's link, whose
    // client-facing ends are.
    let Link {
        host: h0,
        host_in: hin0,
        host_out: hout0,
        per_in: haw0,
        per_out: hbr0,
        ..
    } = axi::<16, 32, 4, 2, 4>();
    let Link {
        host: h1,
        host_in: hin1,
        host_out: hout1,
        per_in: haw1,
        per_out: hbr1,
        ..
    } = axi::<16, 32, 4, 2, 4>();
    let Link {
        per,
        per_in: pin,
        per_out: pout,
        ..
    } = axi::<16, 32, 4, 3, 8>();
    // The unregistered path, the same again.
    let Link {
        host: h0u,
        host_in: hin0u,
        host_out: hout0u,
        per_in: haw0u,
        per_out: hbr0u,
        ..
    } = axi::<16, 32, 4, 2, 4>();
    let Link {
        host: h1u,
        host_in: hin1u,
        host_out: hout1u,
        per_in: haw1u,
        per_out: hbr1u,
        ..
    } = axi::<16, 32, 4, 2, 4>();
    let Link {
        per: peru,
        per_in: pinu,
        per_out: poutu,
        ..
    } = axi::<16, 32, 4, 3, 8>();

    let mut host0 = HostUnit::default();
    let mut host1 = HostUnit::default();
    let mut host0u = HostUnit::default();
    let mut host1u = HostUnit::default();
    let mut path = Path::default();
    let mut pathu = PathU::default();

    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // Every channel under the name of the path's port on it, so
        // each netlist is checked at its ports with no alias.
        w.add("aw0", &haw0.0);
        w.add("ar0", &haw0.1);
        w.add("w0", &haw0.2);
        w.add("b0", &hbr0.2);
        w.add("r0", &hbr0.3);
        w.add("aw1", &haw1.0);
        w.add("ar1", &haw1.1);
        w.add("w1", &haw1.2);
        w.add("b1", &hbr1.2);
        w.add("r1", &hbr1.3);
        w.add("req", &pout.0);
        w.add("wd", &pout.1);
        w.add("ans", &pin.3);
        w.add("rb", &pin.4);
        w.add("aw0u", &haw0u.0);
        w.add("ar0u", &haw0u.1);
        w.add("w0u", &haw0u.2);
        w.add("b0u", &hbr0u.2);
        w.add("r0u", &hbr0u.3);
        w.add("aw1u", &haw1u.0);
        w.add("ar1u", &haw1u.1);
        w.add("w1u", &haw1u.2);
        w.add("b1u", &hbr1u.2);
        w.add("r1u", &hbr1u.3);
        w.add("requ", &poutu.0);
        w.add("wdu", &poutu.1);
        w.add("ansu", &pinu.3);
        w.add("rbu", &pinu.4);
        w.add("path", &path);
        w.add("pathu", &pathu);
        w.start();
    }

    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let logu: Log = Rc::new(RefCell::new(Vec::new()));
    let registered = join2(
        join2(host0.run(hin0, hout0), host1.run(hin1, hout1)),
        join2(
            path.run(
                (haw0.0, haw0.1, haw0.2, haw1.0, haw1.1, haw1.2, pin.3, pin.4),
                (hbr0.2, hbr0.3, hbr1.2, hbr1.3, pout.0, pout.1),
            ),
            join2(clients(h0, h1, log.clone()), memory(per)),
        ),
    );
    let unregistered = join2(
        join2(host0u.run(hin0u, hout0u), host1u.run(hin1u, hout1u)),
        join2(
            pathu.run(
                (
                    haw0u.0, haw0u.1, haw0u.2, haw1u.0, haw1u.1, haw1u.2,
                    pinu.3, pinu.4,
                ),
                (hbr0u.2, hbr0u.3, hbr1u.2, hbr1u.3, poutu.0, poutu.1),
            ),
            join2(clients(h0u, h1u, logu.clone()), memory(peru)),
        ),
    );

    let mut sim = Running::new(join2(registered, unregistered));
    for _ in 0..80 {
        sim.cycle();
    }
    stop();

    let (reg, unreg) = (log.borrow().clone(), logu.borrow().clone());
    assert_eq!(reg.len(), 3, "the registered path answered every burst");
    assert_eq!(unreg.len(), 3, "and so did the unregistered one");
    println!("answered at cycle        registered  unregistered  sooner by");
    for (what, t) in &reg {
        let tu = unreg
            .iter()
            .find(|(w, _)| w == what)
            .map(|(_, t)| *t)
            .expect("the same burst on the unregistered path");
        assert!(tu < *t, "{what}: sooner unregistered");
        println!("{what:<24} {t:>10}  {tu:>12}  {:>9}", t - tu);
        if *what == "host 0 read 0x1008" {
            assert_eq!(t - tu, 4, "two hops down and two back cost none");
        }
    }
    txhdl::netlist::write_netlists_from_env(&[
        &Path::lowered("axi_halves_path"),
        &PathU::lowered("axi_halves_path_u"),
    ]);
}
