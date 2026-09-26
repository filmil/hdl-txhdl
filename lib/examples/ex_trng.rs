// SPDX-License-Identifier: Apache-2.0
//! An entropy source behind AXI-Lite: ring oscillators sampled, the
//! samples folded and debiased, the words buffered, and a health test
//! watching the stream.
//!
//! A host client on an AXI4 link reaches the peripheral through the
//! AXI-Lite bridge and does what a driver would: it turns the source
//! on, waits for `status` to say a word is ready, and takes eight
//! words, which it prints. It then reads `raw`, the last thirty-two
//! samples before the extractor, which is what a measurement on a
//! board reads in a loop. Last it stops the source and drains what
//! the buffer holds, which shows the count in `status` going down.
//!
//! The rings are a Verilog module the netlist instantiates and does
//! not write, and in this run they are a model with the shape of the
//! samples and none of their physics: nothing printed here is
//! evidence of randomness, only of the machinery. The peripheral is
//! lowered, and the build simulates its netlist against this run
//! under nvc and Verilator with the samples as the run recorded them.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, Clock, DefaultClock, Running, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge, LitePort};
use txhdl_parts::trng::{
    RingOsc, Trng, CTRL, CTRL_RUN, DATA, RAW, RINGS, STATUS, STATUS_READY,
};

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the peripheral at `0x1000`.
type Bridge = LiteBridge<1, TrngMap, 32, 32, 4, 2>;

/// Where the bridge's one peripheral is: a nibble of the address
/// space at 0x1000.
pub struct TrngMap;

impl AddrMap<1> for TrngMap {
    const RANGES: [(usize, usize); 1] = [(0x1000, 0xf000)];
}

/// The peripheral's words, as the host addresses them.
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
    let (raw_o, raw) = signal::<U<RINGS>, DefaultClock>();
    let (en_o, en) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut ring = RingOsc::default();
    let mut trng = Trng::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("raw", &raw);
        wave.add("en", &en);
        wave.add("trng", &trng);
        wave.start();
    }

    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        let at = |off: u32| BASE + off;
        let get = |off: u32| {
            let h = &host;
            async move {
                let got = h.read(Rd::at(at(off), 1)).await.done().await;
                got.data[0].raw() as u32
            }
        };
        let s = get(STATUS).await;
        println!("{:3}  before the run bit: status {s:#x}", now());
        let ok = host
            .write(Wr::at(at(CTRL)), &word(CTRL_RUN))
            .await
            .done()
            .await;
        assert_eq!(ok.resp, Resp::Okay, "the write was answered");
        let mut words = Vec::new();
        while words.len() < 8 {
            if get(STATUS).await & STATUS_READY != 0 {
                words.push(get(DATA).await);
            }
        }
        println!("{:3}  eight words: {:08x?}", now(), words);
        let mut sorted = words.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 8, "the eight words differ");
        let raw = get(RAW).await;
        println!("{:3}  the last 32 samples: {raw:032b}", now());
        // Left alone, the buffer fills and holds four; then the
        // source is stopped and the four are read out.
        for _ in 0..1200 {
            DefaultClock::rising().await;
        }
        host.write(Wr::at(at(CTRL)), &word(0)).await.done().await;
        let s = get(STATUS).await;
        println!(
            "{:3}  stopped: {} words waiting, fault {}",
            now(),
            (s >> 1) & 7,
            (s >> 8) & 1
        );
        let mut left = Vec::new();
        while get(STATUS).await & STATUS_READY != 0 {
            get(DATA).await;
            left.push((get(STATUS).await >> 1) & 7);
        }
        println!("{:3}  drained: the count went {:?}", now(), left);
        assert_eq!(get(DATA).await, 0, "a read of nothing is zero");
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, [lb], [lr]), ([law], [lar], [lw], b, r)),
        ),
        join2(
            join2(ring.run(en, raw_o), trng.run(bus, (raw, en_o))),
            client,
        ),
    ));
    println!("  t  what the program saw");
    for _ in 0..3000 {
        sim.cycle();
    }
    stop();
    let net = Trng::lowered("trng");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
