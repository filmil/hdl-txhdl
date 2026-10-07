// SPDX-License-Identifier: Apache-2.0
//! The Vreteno board's device tree, for Linux, from the maps the design
//! decodes (issue 1015, item M8 of #279).
//!
//! A kernel finds its machine in a device tree, and a tree typed by hand
//! is one more copy of the router's map to drift from it. This writes the
//! tree from the design instead: memory, the boot and data memories, the
//! core-local interruptor, the platform-level interrupt controller and
//! the serial port are where `BoardMap` and `SlotMap` put them, the
//! hart's ISA string is what `misa` reports, and each interrupt is the
//! PLIC source `PLIC_SOURCES` names.
//!
//! The bindings are Linux's stock ones, so that its drivers, OpenSBI's
//! and U-Boot's bind unchanged, as Zephyr's tree does with Zephyr's:
//!
//! * the serial port as `sifive,uart0`, the register map #1011 gives the
//!   hardware;
//! * the core-local interruptor as `sifive,clint0`, with the hart's
//!   machine software and timer interrupts;
//! * the PLIC as `sifive,plic-1.0.0`, with two contexts, the hart's
//!   machine external interrupt and its supervisor one. The second is
//!   #1013's, which gives the hardware a supervisor context; until it
//!   lands the tree describes the machine Linux needs rather than the
//!   one built.
//!
//! What the hart is comes from the design too, so the items of #279
//! that change it change the tree with them. `riscv,isa` and
//! `riscv,isa-extensions` are `misa`'s letters: the A extension, or the
//! S of a supervisor mode, appears when `misa` gains it. `mmu-type` is
//! `isa::MMU`, which `misa` has no letter for: `riscv,sv32` since the
//! hart has Sv32 (#1014). Without it, OpenSBI disables the hart in the
//! tree it hands the kernel.
//!
//! The build compiles the tree with `dtc`, which refuses one that is
//! malformed, and a test here holds every address to the maps.
use txhdl::map::AddrMap;
use vreteno32::board::{BoardMap, SlotMap, PLIC_SOURCES};
use vreteno32::isa::{ETH_BUF_BASE, MISA, MMU};

/// The board's clock, which the timer counts and the serial port
/// divides: 100 MHz, the DDR3 controller's user clock.
pub const CLOCK_HZ: u32 = 100_000_000;

/// The serial port's speed at reset, which the loader and every program
/// use.
pub const BAUD: u32 = 115_200;

/// Where a router range starts and how long it is: the mask's low zeros
/// are its size.
fn range<const N: usize, M: AddrMap<N>>(i: usize) -> (usize, usize) {
    let (base, mask) = M::RANGES[i];
    (base, (!mask & 0xffff_ffff) + 1)
}

/// The index of the range a map names with `what`, so that a reordered
/// map moves the tree with it rather than misdescribing it.
fn named<const N: usize, M: AddrMap<N>>(what: &str) -> usize {
    M::NAMES
        .iter()
        .position(|n| n.contains(what))
        .unwrap_or_else(|| panic!("no range in the map is {what}"))
}

/// A PLIC source's number: its place in `PLIC_SOURCES`, from one.
fn source(what: &str) -> usize {
    1 + PLIC_SOURCES
        .iter()
        .position(|n| *n == what)
        .unwrap_or_else(|| panic!("no PLIC source is {what}"))
}

/// The hart's ISA string, from `misa`'s extension bits in the order
/// the specification gives them, with the CSR extension the core has.
fn isa() -> String {
    let mut s = String::from("rv32");
    for c in "imafdqc".chars() {
        let bit = c as u32 - 'a' as u32;
        if MISA & (1 << bit) != 0 {
            s.push(c);
        }
    }
    s.push_str("_zicsr");
    s
}

/// The extensions one by one, as `riscv,isa-extensions` lists them.
fn extensions() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for c in "imafdqc".chars() {
        let bit = c as u32 - 'a' as u32;
        if MISA & (1 << bit) != 0 {
            v.push(c.to_string());
        }
    }
    v.push("zicsr".to_string());
    v
}

/// The kernel's command line when nothing else is asked for: the
/// console on the SiFive port from the first line the kernel prints,
/// so a boot that dies before the console driver binds still says why
/// (issue 1125).
pub const BOOTARGS: &str = "earlycon console=ttySIF0";

/// What a boot image adds to `/chosen`: where the initramfs is, and
/// the kernel's command line (issue 1019), [`BOOTARGS`] by default.
pub struct Chosen {
    /// The initramfs's first byte and the byte after its last.
    pub initrd: Option<(u32, u32)>,
    /// The kernel's command line.
    pub bootargs: Option<String>,
}

impl Default for Chosen {
    fn default() -> Self {
        Chosen {
            initrd: None,
            bootargs: Some(BOOTARGS.to_string()),
        }
    }
}

/// The tree, as `dtc` reads it, with nothing chosen but the console and
/// the default command line.
pub fn dts() -> String {
    dts_with(&Chosen::default())
}

/// The tree, with what a boot image chooses.
pub fn dts_with(chosen: &Chosen) -> String {
    let mut extra = String::new();
    if let Some((start, end)) = chosen.initrd {
        extra.push_str(&format!(
            "\n\t\tlinux,initrd-start = <{start:#010x}>;\n\t\tlinux,initrd-end = <{end:#010x}>;"
        ));
    }
    if let Some(args) = &chosen.bootargs {
        extra.push_str(&format!("\n\t\tbootargs = \"{args}\";"));
    }
    let (ddr, ddr_len) = range::<8, BoardMap>(named::<8, BoardMap>("DDR3"));
    let (rom, rom_len) = range::<8, BoardMap>(named::<8, BoardMap>("boot"));
    let (dmem, dmem_len) =
        range::<8, BoardMap>(named::<8, BoardMap>("data memory"));
    let (clint, clint_len) =
        range::<8, BoardMap>(named::<8, BoardMap>("timer"));
    let (plic, plic_len) =
        range::<8, BoardMap>(named::<8, BoardMap>("interrupt controller"));
    let (uart, uart_len) = range::<10, SlotMap>(named::<10, SlotMap>("serial"));
    let (mac, mac_len) =
        range::<10, SlotMap>(named::<10, SlotMap>("Ethernet port's registers"));
    let ethernet = source("ethernet");
    let exts = extensions()
        .iter()
        .map(|e| format!("\"{e}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let isa = isa();
    let serial = source("serial");
    let mmu = match MMU {
        Some(m) => format!("\n\t\t\tmmu-type = \"{m}\";"),
        None => String::new(),
    };
    let ndev = PLIC_SOURCES.len();
    format!(
        r#"// SPDX-License-Identifier: Apache-2.0
// The Vreteno board's device tree, for Linux. Written by
// //tools/devtree from the maps the design decodes; do not edit.

/dts-v1/;

/ {{
	#address-cells = <1>;
	#size-cells = <1>;
	model = "Alinx AX7A200B (Vreteno)";
	compatible = "hdlfactory,ax7a200b", "hdlfactory,vreteno";

	aliases {{
		serial0 = &uart0;
	}};

	chosen {{
		stdout-path = "serial0:{BAUD}n8";{extra}
	}};

	cpus {{
		#address-cells = <1>;
		#size-cells = <0>;
		timebase-frequency = <{CLOCK_HZ}>;

		cpu0: cpu@0 {{
			device_type = "cpu";
			compatible = "hdlfactory,vreteno", "riscv";
			reg = <0>;
			riscv,isa = "{isa}";
			riscv,isa-base = "rv32i";
			riscv,isa-extensions = {exts};{mmu}
			status = "okay";

			cpu0_intc: interrupt-controller {{
				compatible = "riscv,cpu-intc";
				#address-cells = <0>;
				#interrupt-cells = <1>;
				interrupt-controller;
			}};
		}};
	}};

	memory@{ddr:x} {{
		device_type = "memory";
		reg = <{ddr:#010x} {ddr_len:#x}>;
	}};

	reserved-memory {{
		#address-cells = <1>;
		#size-cells = <1>;
		ranges;

		// The Ethernet port's frame buffers, which its engines write.
		eth_bufs: memory@{eth:x} {{
			reg = <{eth:#010x} 0x2000>;
			no-map;
		}};
	}};

	sysclk: clock {{
		compatible = "fixed-clock";
		#clock-cells = <0>;
		clock-frequency = <{CLOCK_HZ}>;
	}};

	soc {{
		#address-cells = <1>;
		#size-cells = <1>;
		compatible = "simple-bus";
		ranges;

		boot: memory@{rom:x} {{
			compatible = "mmio-sram";
			reg = <{rom:#010x} {rom_len:#x}>;
		}};

		dmem: memory@{dmem:x} {{
			compatible = "mmio-sram";
			reg = <{dmem:#010x} {dmem_len:#x}>;
		}};

		clint: timer@{clint:x} {{
			compatible = "sifive,clint0", "riscv,clint0";
			reg = <{clint:#010x} {clint_len:#x}>;
			interrupts-extended = <&cpu0_intc 3>, <&cpu0_intc 7>;
		}};

		plic: interrupt-controller@{plic:x} {{
			compatible = "sifive,plic-1.0.0", "riscv,plic0";
			reg = <{plic:#010x} {plic_len:#x}>;
			#address-cells = <0>;
			#interrupt-cells = <1>;
			interrupt-controller;
			interrupts-extended = <&cpu0_intc 11>, <&cpu0_intc 9>;
			riscv,ndev = <{ndev}>;
		}};

		uart0: serial@{uart:x} {{
			compatible = "sifive,fu540-c000-uart", "sifive,uart0";
			reg = <{uart:#010x} {uart_len:#x}>;
			interrupt-parent = <&plic>;
			interrupts = <{serial}>;
			clocks = <&sysclk>;
		}};

		// The Ethernet port, which Linux's own litex_liteeth drives:
		// its registers are LiteEth's at LiteX's offsets, and its two
		// receive and two transmit slots are the reserved buffers
		// above, in that order (issue 1203).
		eth0: ethernet@{mac:x} {{
			compatible = "litex,liteeth";
			reg = <{mac:#010x} {mac_len:#x}>, <{eth:#010x} 0x2000>;
			reg-names = "mac", "buffer";
			litex,rx-slots = <2>;
			litex,tx-slots = <2>;
			litex,slot-size = <0x800>;
			interrupt-parent = <&plic>;
			interrupts = <{ethernet}>;
			local-mac-address = [00 0a 35 00 00 01];
		}};
	}};
}};
"#,
        eth = ETH_BUF_BASE,
    )
}

/// `devtree [--initrd START END] [--bootargs ARGS]`: the tree, with a
/// boot image's choices when given.
fn main() {
    let mut chosen = Chosen::default();
    let mut args = std::env::args().skip(1);
    let num = |s: String| {
        let h = s.trim_start_matches("0x").replace('_', "");
        u32::from_str_radix(&h, 16).unwrap_or_else(|_| panic!("not hex: {s}"))
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--initrd" => {
                let start = num(args.next().expect("--initrd START END"));
                let end = num(args.next().expect("--initrd START END"));
                chosen.initrd = Some((start, end));
            }
            "--bootargs" => {
                chosen.bootargs = Some(args.next().expect("--bootargs ARGS"))
            }
            _ => panic!("unknown argument {a}"),
        }
    }
    print!("{}", dts_with(&chosen));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value after `key = <` in the node named `node`, as words.
    fn cells(tree: &str, node: &str, key: &str) -> Vec<u64> {
        let at = tree
            .find(node)
            .unwrap_or_else(|| panic!("no node {node} in the tree"));
        let rest = &tree[at..];
        let k = rest
            .find(&format!("{key} = <"))
            .unwrap_or_else(|| panic!("{node} has no {key}"));
        let rest = &rest[k + key.len() + 4..];
        let end = rest.find('>').unwrap();
        rest[..end]
            .split_whitespace()
            .map(|w| {
                let w = w.trim_start_matches("0x");
                u64::from_str_radix(w, 16)
                    .unwrap_or_else(|_| w.parse().unwrap())
            })
            .collect()
    }

    /// Every node's address, and its length, is the range the design's
    /// map gives it: a router range by its name, a slot by its.
    #[test]
    fn every_address_is_where_the_maps_put_it() {
        let t = dts();
        let want = |i: usize| -> (u64, u64) {
            let (b, m) = BoardMap::RANGES[i];
            (b as u64, ((!m & 0xffff_ffff) + 1) as u64)
        };
        for (node, what) in [
            ("memory@40000000", "DDR3"),
            ("boot: memory@", "boot"),
            ("dmem: memory@", "data memory"),
            ("clint: timer@", "timer"),
            ("plic: interrupt-controller@", "interrupt controller"),
        ] {
            let i = BoardMap::NAMES
                .iter()
                .position(|n| n.contains(what))
                .unwrap();
            let (b, l) = want(i);
            assert_eq!(cells(&t, node, "reg"), [b, l], "{node} against {what}");
        }
        let i = SlotMap::NAMES
            .iter()
            .position(|n| n.contains("serial"))
            .unwrap();
        let (b, m) = SlotMap::RANGES[i];
        assert_eq!(
            cells(&t, "uart0: serial@", "reg"),
            [b as u64, ((!m & 0xffff_ffff) + 1) as u64],
            "the serial port against its slot"
        );
        assert_eq!(
            cells(&t, "eth_bufs: memory@", "reg")[0],
            ETH_BUF_BASE as u64,
            "the Ethernet buffers where the port's engines write"
        );
        // The port's registers on their slot, and its slots the
        // reserved buffers, which litex_liteeth maps as `buffer`.
        let i = SlotMap::NAMES
            .iter()
            .position(|n| n.contains("Ethernet port's registers"))
            .unwrap();
        let (b, m) = SlotMap::RANGES[i];
        assert_eq!(
            cells(&t, "eth0: ethernet@", "reg"),
            [b as u64, ((!m & 0xffff_ffff) + 1) as u64],
            "the Ethernet port's registers"
        );
        let node = &t[t.find("eth0: ethernet@").unwrap()..];
        let reg = &node[node.find("reg = ").unwrap()..];
        let reg = &reg[..reg.find(';').unwrap()];
        assert!(
            reg.ends_with(&format!(", <{ETH_BUF_BASE:#010x} 0x2000>")),
            "the Ethernet port's buffers, its second region: {reg}"
        );
        assert_eq!(
            cells(&t, "eth0: ethernet@", "interrupts"),
            [
                1 + PLIC_SOURCES.iter().position(|s| *s == "ethernet").unwrap()
                    as u64
            ],
            "the Ethernet port's PLIC source"
        );
    }

    /// The interrupts: the serial port's PLIC source, the PLIC's two
    /// contexts on the hart's machine and supervisor external lines, and
    /// the CLINT's machine software and timer lines.
    #[test]
    fn the_interrupts_are_the_ones_the_design_wires() {
        let t = dts();
        assert_eq!(
            cells(&t, "uart0: serial@", "interrupts"),
            [
                1 + PLIC_SOURCES.iter().position(|s| *s == "serial").unwrap()
                    as u64
            ]
        );
        assert_eq!(
            cells(&t, "plic: interrupt-controller@", "riscv,ndev"),
            [PLIC_SOURCES.len() as u64]
        );
        assert!(
            t.contains(
                "interrupts-extended = <&cpu0_intc 11>, <&cpu0_intc 9>;"
            ),
            "the PLIC's machine and supervisor contexts"
        );
        assert!(
            t.contains("interrupts-extended = <&cpu0_intc 3>, <&cpu0_intc 7>;"),
            "the CLINT's software and timer interrupts"
        );
    }

    /// The hart's ISA string is what `misa` says, whatever it gains:
    /// every letter it reports, in the specification's order, except S
    /// and U, which name privilege modes rather than extensions. A
    /// literal here went stale twice, when A (#1010) and then S and U
    /// (#1012) reached `misa`. A letter the generator does not know
    /// fails here, so it is added there rather than left out.
    #[test]
    fn the_isa_is_what_misa_reports() {
        let order = "imafdqc";
        let mut want: Vec<char> = ('a'..='z')
            .filter(|c| MISA & (1 << (*c as u32 - 'a' as u32)) != 0)
            .filter(|c| !"su".contains(*c))
            .collect();
        want.sort_by_key(|c| {
            order
                .find(*c)
                .unwrap_or_else(|| panic!("misa has {c}, unknown to isa()"))
        });
        let letters: String = want.iter().collect();
        assert_eq!(isa(), format!("rv32{letters}_zicsr"));
        let mut ext: Vec<String> = want.iter().map(|c| c.to_string()).collect();
        ext.push("zicsr".to_string());
        assert_eq!(extensions(), ext);
        // What the core has whatever else it gains.
        assert!(letters.starts_with("im") && letters.ends_with('c'));
    }

    /// A boot image's choices land in `/chosen`.
    #[test]
    fn the_initrd_and_the_command_line_are_chosen() {
        let t = dts_with(&Chosen {
            initrd: Some((0x4080_0000, 0x4090_0000)),
            bootargs: Some("earlycon console=ttySIF0".into()),
        });
        assert_eq!(cells(&t, "chosen {", "linux,initrd-start"), [0x4080_0000]);
        assert_eq!(cells(&t, "chosen {", "linux,initrd-end"), [0x4090_0000]);
        assert!(t.contains("bootargs = \"earlycon console=ttySIF0\";"));
        assert!(!dts().contains("initrd"), "nothing chosen by default");
        assert!(
            dts().contains("bootargs = \"earlycon console=ttySIF0\";"),
            "but the console from the first line (issue 1125)"
        );
    }

    /// The hart names its MMU exactly when the design says it has one.
    #[test]
    fn the_mmu_is_what_the_design_states() {
        let t = dts();
        match MMU {
            Some(m) => assert!(t.contains(&format!("mmu-type = \"{m}\";"))),
            None => assert!(!t.contains("mmu-type"), "no MMU, no mmu-type"),
        }
    }
}
