// SPDX-License-Identifier: Apache-2.0
//! A reservation station of two inputs, `Station2`, with a byte on
//! one input and a half word on the other, four lines by a two-bit
//! tag. The two sources offer their tags in different orders, one of
//! them with gaps, and the sink holds the output off for two cycles
//! at a time, so a line waits for room and a repeated tag waits for
//! its cell. Each line comes out as its tag and both values; the
//! netlist is checked against this run. Beside it runs the same
//! station written by hand in Verilog, `station.v`, as a foreign
//! unit fed the same offers, and every line the two send is
//! compared. With `--source` it prints the text `station!` wrote
//! for the two-input station instead.
use station_verilog::StationV;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, DefaultClock, Running, Unit};
use txhdl::types::U;
use txhdl_parts::station::{station2, Line2, Station2, Tagged};

/// The station of this example: two-bit tags, four lines, a byte and
/// a half word.
type Station = Station2<2, 4, U<8>, U<16>>;

fn main() {
    if std::env::args().any(|a| a == "--source") {
        print!("{}", station2::SOURCE);
        return;
    }
    let (a_tx, a_rx) = chan::<Tagged<2, U<8>>, DefaultClock>();
    let (b_tx, b_rx) = chan::<Tagged<2, U<16>>, DefaultClock>();
    let (o_tx, o_rx) = chan::<Line2<2, U<8>, U<16>>, DefaultClock>();
    // The Verilog station's channels carry the same bits, tag above
    // value, as plain words.
    let (va_tx, va_rx) = chan::<U<10>, DefaultClock>();
    let (vb_tx, vb_rx) = chan::<U<18>, DefaultClock>();
    let (vo_tx, vo_rx) = chan::<U<26>, DefaultClock>();
    let mut station = Station::default();
    let mut verilog = StationV::default();
    let (occ0, occ1) = (station.occ0, station.occ1);
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("in0", &a_rx);
        w.add("in1", &b_rx);
        w.add("out", &o_rx);
        w.add("station", &station);
        w.start();
    }
    let mut sim = Running::new(join2(
        station.run((a_rx, b_rx), o_tx),
        verilog.run((va_rx, vb_rx), vo_tx),
    ));
    let mut agreed = 0;
    // The bytes come tagged 0, 1, 2, 3, then 0 and 2 again; the half
    // words come tagged 2, 0, 3, 1, 2, 0, with a gap every third
    // cycle; the sink takes on two cycles in four.
    let a_tags = [0u8, 1, 2, 3, 0, 2];
    let b_tags = [2u8, 0, 3, 1, 2, 0];
    let (mut ai, mut bi) = (0, 0);
    println!(" t a(tag) b(tag) occ0 occ1  line");
    for t in 0..22 {
        let a = if ai < a_tags.len() && a_tx.ready().to_bool() {
            let (tag, value) = (a_tags[ai], 0x10 + ai as u8);
            a_tx.send(Tagged {
                tag: U::from(tag),
                value: U::from(value),
            });
            va_tx.send(U::from((tag as u32) << 8 | value as u32));
            ai += 1;
            format!("{tag}")
        } else {
            "-".to_string()
        };
        let b = if bi < b_tags.len() && t % 3 != 2 && b_tx.ready().to_bool() {
            let (tag, value) = (b_tags[bi], 0x100 * (bi as u16 + 1));
            b_tx.send(Tagged {
                tag: U::from(tag),
                value: U::from(value),
            });
            vb_tx.send(U::from((tag as u32) << 16 | value as u32));
            bi += 1;
            format!("{tag}")
        } else {
            "-".to_string()
        };
        let (line, vline) = if t % 4 >= 2 {
            (o_rx.recv(), vo_rx.recv())
        } else {
            (None, None)
        };
        // The Verilog station's line, as the same bits.
        let bits =
            line.map(|l| (l.tag.raw() << 24) | (l.v0.raw() << 16) | l.v1.raw());
        assert_eq!(
            bits,
            vline.map(|v| v.raw()),
            "the Verilog station and the TxHDL one differ at cycle {t}"
        );
        agreed += line.is_some() as usize;
        sim.cycle();
        println!(
            "{:2} {:>6} {:>6} {:04b} {:04b}  {}",
            now() / 2,
            a,
            b,
            occ0.get().raw(),
            occ1.get().raw(),
            line.map_or("-".to_string(), |l| format!(
                "tag {} = ({:#x}, {:#x})",
                l.tag.raw(),
                l.v0.raw(),
                l.v1.raw()
            ))
        );
    }
    stop();
    println!("{agreed} lines, the same from both stations");
    txhdl::netlist::write_vhdl_from_env(&Station::lowered("station"));
    print!("\n{}", Station::verilog("station"));
}
