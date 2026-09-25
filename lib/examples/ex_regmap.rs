// SPDX-License-Identifier: Apache-2.0
//! A register map declared once: three registers behind AXI-Lite, the
//! map said in one `regmap!` and nowhere else.
//!
//! `Knobs` is the smallest peripheral that has a map: an identity a
//! host reads, a control word it writes, and a counter that runs
//! while the control word's bit 0 is set. Its `run` decodes nothing
//! by hand. The map's read mux answers every read, its write enables
//! guard every write, and the constants it wrote are what the host
//! client addresses. The same declaration is what the lowering reads,
//! so the netlist decodes the map the program sees; and it is what the
//! tools read, so the example ends by printing the C header a driver
//! would include and the rows a datasheet's register table holds.
//!
//! A host client on an AXI4 link reaches the peripheral through the
//! AXI-Lite bridge, reads the identity, starts the counter, reads the
//! count twice, stops it, and reads a word the map does not name,
//! which answers zero. The peripheral is lowered, and the build
//! simulates its netlist against this run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, Clock, DefaultClock, Reg, Running, Unit};
use txhdl::regmap;
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{
    axi_lite, LiteB, LiteBridge1, LitePort, LiteR,
};

// begin{map}
regmap! { knobs (knobs_read, knobs_we), 2: [
    (0, id, ro, "who this is: 0x4b4e4f42, `KNOB`"),
    (1, ctrl, rw, "the control word", [
        (run, 0, 1, rw, 0, "runs the counter"),
        (step, 4, 4, rw, 1, "what a count adds"),
    ]),
    (2, count, ro, "cycles since the run bit rose"),
] }
// end{map}

// begin{unit}
/// Three registers: who it is, a control word, and a count.
#[derive(Trace, Default)]
pub struct Knobs {
    /// The control word's run bit.
    pub run: Reg<Bit>,
    /// The control word's step: what a count adds.
    pub step: Reg<U<4>>,
    /// Cycles since the run bit rose.
    pub count: Reg<U<32>>,
}

#[lower]
impl Unit for Knobs {
    async fn run(&mut self, bus: LitePort<32, 32, 4>, _o: ()) {
        loop {
            DefaultClock::rising().await;
            let run = self.run.get();
            let step = self.step.get();
            let count = self.count.get();
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let rsel = arh.addr.slice::<2, 2>();
            let wsel = awh.addr.slice::<2, 2>();
            let rgo = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let written = wh.data;
            // The map: the word a read answers, and a bit a register
            // for the writes, both from the declaration.
            let word = knobs_read(
                rsel,
                U::<32>::from(0x4b4e_4f42u32),
                knobs_ctrl_pack(run, step),
                count,
            );
            let we = knobs_we(wgo, wsel);
            with!(self <= {
                we.bit(1) ? {
                    run: knobs_ctrl_run(written),
                    step: knobs_ctrl_step(written),
                },
                we.bit(1) & knobs_ctrl_run(written) & !run ? count:
                    U::<32>::from(0u8),
                run ? count: count + step.zext::<32>(),
            });
            if rgo.to_bool() {
                bus.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
        }
    }
}
// end{unit}

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the peripheral at `0x1000`.
type Bridge = LiteBridge1<32, 32, 4, 2, 0x1000, 0xf000>;

const BASE: u32 = 0x1000;

fn main() {
    let Link {
        host,
        host_in,
        host_out,
        per_in,
        per_out,
        ..
    } = axi::<32, 32, 4, 2, 4>();
    let (aw, ar, w, _, _) = per_in;
    let (_, _, b, r) = per_out;
    let lite = axi_lite::<32, 32, 4>();
    let (law, lar, lw, lb, lr) = lite.host;
    let bus: LitePort<32, 32, 4> = lite.per.into();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut knobs_unit = Knobs::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("knobs", &knobs_unit);
        wave.start();
    }

    let client = async move {
        let get = |off: u32| {
            let h = &host;
            async move {
                let got = h.read(Rd::at(BASE + off, 1)).await.done().await;
                got.data[0].raw() as u32
            }
        };
        let put = |off: u32, v: u32| {
            let h = &host;
            async move {
                let ok = h
                    .write(Wr::at(BASE + off), &[U::<32>::from(v)])
                    .await
                    .done()
                    .await;
                assert_eq!(ok.resp, Resp::Okay, "the write was answered");
            }
        };
        let id = get(knobs::id).await;
        println!("{:3}  id     {id:#010x}", now());
        assert_eq!(id, 0x4b4e_4f42);
        // The run bit and a step of three, each field set on its own.
        put(
            knobs::ctrl,
            knobs::ctrl_run.with(1) | knobs::ctrl_step.with(3),
        )
        .await;
        let ctrl = get(knobs::ctrl).await;
        println!(
            "{:3}  ctrl   {ctrl:#x}: run {} step {}",
            now(),
            knobs::ctrl_run.get(ctrl),
            knobs::ctrl_step.get(ctrl)
        );
        let first = get(knobs::count).await;
        println!("{:3}  count  {first}", now());
        for _ in 0..20 {
            DefaultClock::rising().await;
        }
        let second = get(knobs::count).await;
        println!("{:3}  count  {second}", now());
        assert!(second > first, "the counter runs");
        assert_eq!((second - first) % 3, 0, "by threes");
        put(knobs::ctrl, 0).await;
        let stopped = get(knobs::count).await;
        for _ in 0..20 {
            DefaultClock::rising().await;
        }
        assert_eq!(get(knobs::count).await, stopped, "and stops");
        println!("{:3}  stopped at {stopped}", now());
        let hole = get(0xc).await;
        println!("{:3}  a word not named reads {hole}", now());
        assert_eq!(hole, 0);
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, lb, lr), (law, lar, lw, b, r)),
        ),
        join2(knobs_unit.run(bus, ()), client),
    ));
    println!("  t  what the program saw");
    for _ in 0..400 {
        sim.cycle();
    }
    stop();
    let net = Knobs::lowered("knobs");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", knobs::MAP.c_header("knobs"));
    println!();
    for row in knobs::MAP.tex_rows() {
        println!("{row}");
    }
    print!("\n{}", net.verilog());
}
