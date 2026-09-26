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
use vreteno32::isa::{
    CLINT_BASE, ETH_BASE, ETH_BUF_BASE, MSIP_OFF, MTIMECMP_OFF, MTIME_OFF,
    TRNG_BASE, UART_BASE,
};
use vreteno32::uart::serial;

// The port, read at compile time, so the test needs no runfiles and
// the build knows these files are inputs: editing one reruns this.
const DTSI: &str =
    include_str!("../../../zephyr/dts/riscv/hdlfactory/vreteno.dtsi");
const DRIVER: &str =
    include_str!("../../../zephyr/drivers/serial/uart_vreteno.c");
const SOC_KCONFIG: &str =
    include_str!("../../../zephyr/soc/hdlfactory/vreteno/Kconfig");
const UART_KCONFIG: &str =
    include_str!("../../../zephyr/drivers/serial/Kconfig.vreteno");
const ETH_DRIVER: &str =
    include_str!("../../../zephyr/drivers/ethernet/eth_vreteno.c");
const ETH_KCONFIG: &str =
    include_str!("../../../zephyr/drivers/ethernet/Kconfig.vreteno");
const TRNG_DRIVER: &str =
    include_str!("../../../zephyr/drivers/entropy/entropy_vreteno.c");
const TRNG_KCONFIG: &str =
    include_str!("../../../zephyr/drivers/entropy/Kconfig.vreteno");
/// The hardware the Ethernet driver talks to. The two register maps
/// are held to each other rather than each to a document, which is
/// the only arrangement in which they cannot quietly disagree.
const ETHSLOTS: &str = include_str!("../../../lib/parts/src/ethslots.rs");
const BOARD_DEFCONFIG: &str = include_str!(
    "../../../zephyr/boards/hdlfactory/ax7a200b/ax7a200b_defconfig"
);

/// The device tree's `reg` for a node, as its first address.
fn reg_of(dts: &str, node: &str) -> u64 {
    let at = dts
        .find(node)
        .unwrap_or_else(|| panic!("no node `{node}` in the device tree"));
    let rest = &dts[at..];
    let reg =
        rest.find("reg = <").expect("the node has no reg") + "reg = <".len();
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
    // The timer's node names `mtime` first, since Zephyr's driver
    // takes the two registers separately rather than the block.
    assert_eq!(
        reg_of(dts, "mtimer: timer@"),
        (CLINT_BASE + MTIME_OFF) as u64,
        "the machine timer's mtime"
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
    assert_eq!(
        reg_of(dts, "trng0: rng@"),
        TRNG_BASE as u64,
        "the entropy source"
    );
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
    // The node claims the binding Zephyr's own machine timer driver
    // reads, and names the two registers it takes separately.
    //
    // It said `sifive,clint0` first, which is a different binding
    // that `RISCV_MACHINE_TIMER` does not select on. The image built
    // and had no clock, and the only sign was a Kconfig line saying
    // the timer's dependency was unmet, several hundred lines up.
    assert!(dts.contains("\"riscv,machine-timer\""), "the binding");
    assert!(dts.contains("reg-names = \"mtime\", \"mtimecmp\""), "named");
    assert!(
        !dts.contains("compatible = \"sifive,clint0\""),
        "and not the binding nothing binds to"
    );
}

/// The driver's register map is the one the hardware states, and the
/// device tree's console is the port the driver binds to.
#[test]
fn the_driver_reads_the_registers_the_port_has() {
    let c = DRIVER;
    // The port's own map, `regmap!`'s `serial` in
    // `cpu/vreteno/src/uart.rs` (issue 669): a byte written to the
    // first word goes out, the second word is the status, and a read
    // of the third takes the oldest byte received.
    let word = |name: &str, off: u32| {
        let d = format!("#define VRETENO_UART_{name:<6} 0x{off:02x}");
        assert!(c.contains(d.trim_end()), "the driver has no `{d}`");
    };
    word("DATA", serial::tx);
    word("STATUS", serial::status);
    word("RX", serial::rx);
    let bit = |name: &str, f: txhdl::regmap::Field| {
        let d = format!("VRETENO_STATUS_{name:<4} BIT({})", f.shift);
        assert!(c.contains(&d), "the driver has no `{d}`");
    };
    bit("BUSY", serial::status_busy);
    bit("RX", serial::status_ready);
    bit("FULL", serial::status_full);
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

/// The two ways this port has already built an image that says
/// nothing. Both are silent: the build is green, the ELF links, and
/// the board comes up with no console. Neither is visible in a diff
/// of the driver, so both are asserted here.
#[test]
fn the_console_is_reachable_and_not_merely_compiled() {
    // `UART_CONSOLE` depends on `SERIAL_HAS_DRIVER`, which every
    // serial driver is expected to select. Without it the symbol
    // never becomes visible, the board's `CONFIG_UART_CONSOLE=y` is
    // dropped without a word, and the image has no console at all.
    assert!(
        UART_KCONFIG.contains("select SERIAL_HAS_DRIVER"),
        "the driver must announce itself, or there is no console"
    );
    // `sys_read32` and `sys_write32` are declared by the
    // architecture's header, which `zephyr/arch/cpu.h` reaches.
    // `zephyr/sys/sys_io.h` alone leaves them implicit.
    assert!(
        DRIVER.contains("#include <zephyr/arch/cpu.h>"),
        "the accessors come from the architecture, not the generic header"
    );
    // The board asks for the console the driver provides.
    assert!(
        BOARD_DEFCONFIG.contains("CONFIG_UART_CONSOLE=y"),
        "and the board asks for it"
    );
}

/// The word a `#define VRETENO_ETH_<NAME>   0x..` in the driver names.
fn eth_word(name: &str) -> u32 {
    let pat = format!("#define VRETENO_ETH_{name}");
    let at = ETH_DRIVER
        .find(&pat)
        .unwrap_or_else(|| panic!("no `{name}` in the driver"));
    let tail = &ETH_DRIVER[at + pat.len()..];
    let end = tail.find('\n').expect("a define that never ends");
    let text = tail[..end].trim().trim_start_matches("0x");
    let off = u32::from_str_radix(text, 16)
        .unwrap_or_else(|_| panic!("`{name}` is not an offset"));
    assert_eq!(off % 4, 0, "`{name}` is not on a word boundary");
    off / 4
}

/// The driver's register map is the one the hardware decodes.
///
/// Both files are read here, so neither can be edited into
/// disagreement on its own. That is the whole point: the serial
/// port's map lived in two places and agreed only because nobody had
/// touched either, and this one is the same shape with four more
/// registers.
#[test]
fn the_ethernet_driver_reads_the_words_the_hardware_decodes() {
    // The hardware selects a word and the driver names a byte
    // offset. They have to be the same register.
    for (name, word) in [
        ("RX_SLOT", 0),
        ("RX_LENGTH", 1),
        ("RX_EV_PENDING", 2),
        ("RX_EV_ENABLE", 3),
        ("TX_SLOT", 4),
        ("TX_LENGTH", 5),
        ("TX_START", 6),
        ("TX_READY", 7),
        ("TX_EV_PENDING", 8),
        ("TX_EV_ENABLE", 9),
    ] {
        assert_eq!(eth_word(name), word, "`{name}` should be word {word}");
    }

    // And the hardware decodes each of them, rather than the driver
    // naming an offset nothing answers, which is issue 415 in a
    // peripheral instead of a program. Reads and writes are decoded
    // separately there, so each is checked on the side it is used.
    for word in [0, 1, 7, 8] {
        assert!(
            ETHSLOTS.contains(&format!("rsel == {word}")),
            "the hardware does not answer a read of word {word}"
        );
    }
    for word in [2, 3, 4, 5, 6, 8, 9] {
        assert!(
            ETHSLOTS.contains(&format!("wsel == {word}")),
            "the hardware does not take a write of word {word}"
        );
    }
}

/// An arrival is acknowledged by writing one, on both sides.
///
/// A driver that wrote zero would leave the bit set, take the
/// interrupt again at once, and spin. `lib/examples/ex_ethslots.rs`
/// says the same thing from the other direction: it writes zero and
/// checks the bit survives.
#[test]
fn the_ethernet_driver_acknowledges_by_writing_one() {
    assert!(
        ETHSLOTS.contains("wsel == 2) & data.bit(0)"),
        "the hardware clears the arrival on a written one"
    );
    assert!(
        ETH_DRIVER.contains("VRETENO_ETH_EVENT"),
        "and the driver has a one to write"
    );

    // The acknowledgement is after the receive and not before it.
    // The hardware applies acknowledgements before arrivals so that a
    // frame landing in the same cycle keeps the pending bit set; this
    // order is what puts the driver inside that window. Acknowledging
    // first would leave the hardware correct and the case untested.
    let recv = ETH_DRIVER
        .find("eth_vreteno_receive(dev);")
        .expect("the handler does not receive");
    let ack = ETH_DRIVER[recv..]
        .find("VRETENO_ETH_RX_EV_PENDING")
        .expect("the handler never acknowledges");
    assert!(ack > 0, "the acknowledgement comes after the receive");
}

/// The port is where the design puts it, and so are its buffers.
#[test]
fn the_ethernet_node_is_at_the_address_the_board_decodes() {
    assert_eq!(
        reg_of(DTSI, "eth0: ethernet@"),
        ETH_BASE as u64,
        "the Ethernet registers"
    );

    // The buffers are two regions and not one of four slots. One
    // region lets a transmit slot be computed at a receive slot's
    // address, which is a frame landing on one waiting to go out: it
    // compiles, and it simulates whenever a test drives one direction
    // at a time. The hardware had exactly that once.
    assert!(
        DTSI.contains(r#"reg-names = "registers", "rx_buffers", "tx_buffers""#),
        "named separately, so the two directions cannot alias"
    );
    assert!(
        DTSI.contains(&format!("{:x}", ETH_BUF_BASE)),
        "the buffers at `ETH_BUF_BASE`"
    );
}

/// The two things that would leave the driver silently absent.
///
/// `SERIAL_HAS_DRIVER` taught this on the console: a driver that does
/// not announce itself is one the subsystem never looks for, and the
/// build stays green while the interface never appears. `ETH_DRIVER`
/// is that bit for the network stack.
#[test]
fn the_ethernet_driver_is_reachable_and_not_merely_present() {
    assert!(
        ETH_KCONFIG.contains("select ETH_DRIVER"),
        "the driver must announce itself to the stack"
    );
    // And only where there is a stack: `ETH_DRIVER` depends on
    // `NETWORKING`, so selecting it in a build without one refuses
    // the configuration outright.
    assert!(
        ETH_KCONFIG.contains("depends on NETWORKING"),
        "and only where there is a stack to announce itself to"
    );
    // The accessors are the architecture's, as in the console's
    // driver: `zephyr/sys/sys_io.h` alone leaves them implicit.
    assert!(
        ETH_DRIVER.contains("#include <zephyr/arch/cpu.h>"),
        "the accessors come from the architecture"
    );
}

/// The entropy driver reads the registers the source has, the device
/// tree names the source as the entropy device, and the driver is what
/// the network image's configuration test asks for (issue 458).
#[test]
fn the_entropy_driver_reads_the_registers_the_source_has() {
    use txhdl_parts::trng as t;
    let c = TRNG_DRIVER;
    // The source's own map, `regs` in `lib/parts/src/trng.rs`: data,
    // status, control, raw.
    assert!(c.contains("#define VRETENO_TRNG_DATA   0x00"), "data");
    assert_eq!(t::DATA, 0x0, "data, hardware");
    assert!(c.contains("#define VRETENO_TRNG_STATUS 0x04"), "status");
    assert_eq!(t::STATUS, 0x4, "status, hardware");
    assert!(c.contains("#define VRETENO_TRNG_CTRL   0x08"), "ctrl");
    assert_eq!(t::CTRL, 0x8, "ctrl, hardware");
    assert!(c.contains("#define VRETENO_TRNG_RAW    0x0c"), "raw");
    assert_eq!(t::RAW, 0xc, "raw, hardware");
    // Bit 0 ready and bit 8 the fault in the status; bit 0 run and
    // bit 1 clear in the control.
    assert!(c.contains("VRETENO_TRNG_STATUS_READY BIT(0)"), "ready");
    assert_eq!(t::STATUS_READY, 1, "ready, hardware");
    assert!(c.contains("VRETENO_TRNG_STATUS_FAULT BIT(8)"), "fault");
    assert_eq!(t::STATUS_FAULT, 1 << 8, "fault, hardware");
    assert!(c.contains("VRETENO_TRNG_CTRL_RUN     BIT(0)"), "run");
    assert_eq!(t::CTRL_RUN, 1, "run, hardware");
    assert!(c.contains("VRETENO_TRNG_CTRL_CLEAR   BIT(1)"), "clear");
    assert_eq!(t::CTRL_CLEAR, 2, "clear, hardware");
    let dts = DTSI;
    assert!(
        dts.contains("compatible = \"hdlfactory,vreteno-trng\""),
        "the node the driver binds to"
    );
    assert!(
        dts.contains("zephyr,entropy = &trng0"),
        "and it is the chosen entropy device"
    );
    // The driver says it is a true entropy driver, which is what turns
    // the random subsystem away from the counter.
    let k = TRNG_KCONFIG;
    assert!(k.contains("select ENTROPY_HAS_DRIVER"), "a true source");
    assert!(
        k.contains("depends on DT_HAS_HDLFACTORY_VRETENO_TRNG_ENABLED"),
        "bound to the node"
    );
}
