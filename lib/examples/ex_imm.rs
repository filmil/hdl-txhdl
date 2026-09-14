// SPDX-License-Identifier: Apache-2.0
//! An immediate decoder: RV32I's five immediates from the fields of a
//! word, by `slice`, `concat` and `sext`, and the one the opcode calls
//! for chosen by `select!`, which is `match` on a value that lowers to
//! a chain of multiplexers. What a processor's decode is made of, in
//! the subset the lowering reads, simulated under nvc against the
//! trace.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Out, Reg, Running, Unit};
use txhdl::types::U;
use txhdl::{lower, select, Trace};

#[derive(Trace, Default)]
pub struct Decoder {
    pub value: Reg<U<32>>,
    pub dest: Reg<U<5>>,
}

#[lower]
impl Unit<In<U<32>>, (Out<U<32>>, Out<U<5>>)> for Decoder {
    async fn run(&mut self, ir: In<U<32>>, (imm, rd): (Out<U<32>>, Out<U<5>>)) {
        loop {
            DefaultClock::rising().await;
            let ir = ir.get();
            let opcode = ir.slice::<0, 7>();
            // The five immediates, as the manual draws them.
            let imm_i = ir.slice::<20, 12>().sext::<32>();
            let imm_s = ir
                .slice::<25, 7>()
                .concat::<5, 12>(ir.slice::<7, 5>())
                .sext::<32>();
            let imm_b = ir
                .slice::<31, 1>()
                .concat::<1, 2>(ir.slice::<7, 1>())
                .concat::<6, 8>(ir.slice::<25, 6>())
                .concat::<4, 12>(ir.slice::<8, 4>())
                .concat::<1, 13>(U::<1>::from(0u8))
                .sext::<32>();
            let imm_u =
                ir.slice::<12, 20>().concat::<12, 32>(U::<12>::from(0u8));
            let imm_j = ir
                .slice::<31, 1>()
                .concat::<8, 9>(ir.slice::<12, 8>())
                .concat::<1, 10>(ir.slice::<20, 1>())
                .concat::<10, 20>(ir.slice::<21, 10>())
                .concat::<1, 21>(U::<1>::from(0u8))
                .sext::<32>();
            self.value.set(select!(opcode.raw() => {
                0x23 => imm_s,
                0x63 => imm_b,
                0x37 | 0x17 => imm_u,
                0x6f => imm_j,
                _ => imm_i,
            }));
            self.dest.set(ir.slice::<7, 5>());
            imm.set(self.value.get());
            rd.set(self.dest.get());
        }
    }
}

fn main() {
    let (ir_out, ir) = signal::<U<32>, DefaultClock>();
    let (imm_out, imm) = signal::<U<32>, DefaultClock>();
    let (rd_out, rd) = signal::<U<5>, DefaultClock>();
    let mut dec = Decoder::default();
    let value = dec.value.clone();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("ir", &ir);
        w.add("dec", &dec);
        w.add("imm", &imm);
        w.add("rd", &rd);
        w.start();
    }
    let mut sim = Running::new(dec.run(ir, (imm_out, rd_out)));
    // A word of each format, and what its immediate and rd are.
    let words: [(u32, &str); 7] = [
        (0xffb00093, "addi x1, x0, -5"),   // imm -5, rd 1
        (0xfe712a23, "sw x7, -12(x2)"),    // imm -12, rd 0
        (0x00520663, "beq x4, x5, 12"),    // imm 12, rd 0
        (0xfffff1b7, "lui x3, 0xfffff"),   // imm -4096, rd 3
        (0xff9ff0ef, "jal x1, -8"),        // imm -8, rd 1
        (0x00812303, "lw x6, 8(x2)"),      // imm 8, rd 6
        (0x80209063, "bne x1, x2, -4096"), // imm -4096, rd 0
    ];
    for (w, text) in words {
        ir_out.set(w);
        sim.cycle();
        let v = value.get().raw() as u32 as i32;
        println!("{w:#010x}  {text:<18} imm {v:>6}  rd {}", dec_rd(w));
    }
    sim.cycle();
    stop();
    txhdl::netlist::write_vhdl_from_env(&Decoder::lowered("decoder"));
    print!("\n{}", Decoder::verilog("decoder"));
}

fn dec_rd(w: u32) -> u32 {
    w >> 7 & 0x1f
}
