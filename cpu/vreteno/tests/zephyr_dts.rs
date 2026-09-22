// SPDX-License-Identifier: Apache-2.0
//! The Zephyr port against the hardware it describes.
//!
//! `zephyr/` holds a device tree and a driver that state where this
//! machine's peripherals are. Nothing in Zephyr's build can check
//! those against the design, because Zephyr is not in this tree and
//! the design is not in Zephyr's; the two would drift silently, and
//! the symptom of drift is a board that comes up and says nothing.
//!
//! So the numbers are checked here, against the constants the
//! hardware itself uses. A port that disagrees with the router's map
//! fails this test rather than the board.
use vreteno32::isa::{CLINT_BASE, MSIP_OFF, MTIMECMP_OFF, MTIME_OFF, UART_BASE};

// The port, read at compile time, so the test needs no runfiles and
// the build knows these files are inputs: editing one reruns this.
const DTSI: &str =
    include_str!("../../../zephyr/dts/riscv/hdlfactory/vreteno.dtsi");
const DRIVER: &str =
    include_str!("../../../zephyr/drivers/serial/uart_vreteno.c");
const SOC_KCONFIG: &str =
    include_str!("../../../zephyr/soc/hdlfactory/vreteno/Kconfig.soc");


/// The device tree's `reg` for a node, as its first address.
fn reg_of(dts: &str, node: &str) -> u64 {
    let at = dts
        .find(node)
        .unwrap_or_else(|| panic!("no node `{node}` in the device tree"));
    let rest = &dts[at..];
    let reg = rest
        .find("reg = <")
        .expect("the node has no reg")
        + "reg = <".len();
    let tail = &rest[reg..];
    let end = tail.find([' ', '>']).expect("a reg that never ends");
    let text = tail[..end].trim_start_matches("0x");
    u64::from_str_radix(text, 16).expect("a reg that is not a number")
}

/// Every peripheral the port names is where the design put it.
#[test]
fn the_device_tree_holds_the_addresses_the_hardware_decodes() {
    let dts = DTSI;
    assert_eq!(
        reg_of(dts, "uart0: serial@"),
        UART_BASE as u64,
        "the serial port"
    );
    assert_eq!(
        reg_of(dts, "clint: timer@"),
        CLINT_BASE as u64,
        "the machine timer"
    );
    // The interrupt controller is where RISC-V machines put it, and
    // the board's router decodes the 64 MiB from there.
    assert_eq!(
        reg_of(dts, "plic: interrupt-controller@"),
        0x0c00_0000,
        "the interrupt controller"
    );
    assert_eq!(reg_of(dts, "ddr: memory@"), 0x4000_0000, "the DDR3");
    assert_eq!(reg_of(dts, "dmem: memory@"), 0x1000, "the data memory");
}

/// The timer's node is a CLINT because the hardware is one, at the
/// offsets a stock driver expects. If any of these move the driver
/// reads the wrong word and the kernel's clock stops, which is a
/// symptom a long way from its cause.
#[test]
fn the_timer_is_a_clint_at_the_offsets_a_driver_expects() {
    assert_eq!(MSIP_OFF, 0x0000, "msip");
    assert_eq!(MTIMECMP_OFF, 0x4000, "mtimecmp");
    assert_eq!(MTIME_OFF, 0xbff8, "mtime");
    let dts = DTSI;
    assert!(
        dts.contains("\"sifive,clint0\""),
        "the node claims the driver those offsets belong to"
    );
}

/// The driver's register map is the one the hardware states, and the
/// device tree's console is the port the driver binds to.
#[test]
fn the_driver_reads_the_registers_the_port_has() {
    let c = DRIVER;
    // `cpu/vreteno/src/uart.rs`: a byte written to the first word
    // goes out, the second word is the status, and a read of the
    // third takes the oldest byte received.
    assert!(c.contains("#define VRETENO_UART_DATA   0x00"), "data");
    assert!(c.contains("#define VRETENO_UART_STATUS 0x04"), "status");
    assert!(c.contains("#define VRETENO_UART_RX     0x08"), "receive");
    // Bit 0 busy, bit 1 a byte received, bit 2 the buffer full.
    assert!(c.contains("VRETENO_STATUS_BUSY BIT(0)"), "busy");
    assert!(c.contains("VRETENO_STATUS_RX   BIT(1)"), "received");
    assert!(c.contains("VRETENO_STATUS_FULL BIT(2)"), "full");
    let dts = DTSI;
    assert!(
        dts.contains("compatible = \"hdlfactory,vreteno-uart\""),
        "the node the driver binds to"
    );
    assert!(
        dts.contains("zephyr,console = &uart0"),
        "and the console is that port"
    );
}

/// The core is RV32IMC, and the port says so in both places that
/// matter: the ISA string the toolchain reads and the SoC's Kconfig,
/// which must not select an A extension this core does not have.
#[test]
fn the_port_asks_for_the_instruction_set_the_core_has() {
    let dts = DTSI;
    assert!(dts.contains("riscv,isa = \"rv32imc_zicsr\""), "the ISA");
    let kconfig = SOC_KCONFIG;
    assert!(kconfig.contains("RISCV_ISA_EXT_M"), "multiply");
    assert!(kconfig.contains("RISCV_ISA_EXT_C"), "compressed");
    assert!(
        !kconfig.contains("RISCV_ISA_EXT_A"),
        "the core has no atomics and the port must not claim them"
    );
    assert!(
        kconfig.contains("ATOMIC_OPERATIONS_C"),
        "so the atomics are the C ones"
    );
}
