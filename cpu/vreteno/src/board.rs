// SPDX-License-Identifier: Apache-2.0
//! The whole of the design that goes on the board, as one lowered unit.
//!
//! The core, its tracker, a router, and eight ranges behind the router,
//! from the boot memory at `0x0000` to the configuration flash, read
//! as memory, at `0x2000_0000`, which `BoardMap` names in the order of
//! the router's ports. The stack memory at `0x1_0000` (issue 1278) is
//! in the core, on its own port, and not on the bus (issue 1275). The third of them is the peripheral page: nine
//! slots of 256 bytes from `0x3000` behind an AXI-Lite bridge, which
//! `SlotMap` names. `//tools/memmap` writes both maps into the
//! documents from these types, so they are not listed again here
//! (#444). The platform-level interrupt controller's source 1 is the
//! serial port's receive interrupt, its source 2 the board's `irq`
//! input and its source 3 a frame arriving on the Ethernet port,
//! and its line is the core's external interrupt. `run.rs` wires
//! the same parts for a simulation, with a Rust `join` of their runs;
//! this is that wiring written as a unit of units, so `#[lower]` makes
//! its netlist, one module holding the rest, and the only thing a board
//! top has to do by hand is the clocks, the reset and the pins.
//!
//! The DDR3 controller is a foreign module inside it, so the netlist
//! names the controller's wrapper and does not write it, and the
//! memory's pins, the data and strobe pads among them, are this unit's
//! own ports, as are the board's clock going in and the design's clock
//! coming out, since the controller makes it. `DIV` is the serial
//! port's clock divider.
use crate::core::Writeback;
use crate::debug::Dm;
use crate::dmem::Dmem;
use crate::hart::Hart;
use crate::isa;
use crate::rom::Rom;
use crate::timer::Timer;
use crate::uart::Uart;
use ddr3::Ddr3Per;
use razboj::doorbell::Doorbell;
use razboj::raster::Raster;
use txhdl::comp::{
    chan, join2, signal, DefaultClock, In, Out, Pad, Rx, Tx, Unit,
};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};
use txhdl_parts::bus::arbiter::{Arbiter2, Arbiter7};
use txhdl_parts::bus::axi::{
    Answer, Ar, Aw, AxiHost, AxiPer, Done, Grant, Issue, PerPort, PerReq, B, R,
    W,
};
use txhdl_parts::bus::axi_lite::{
    LiteAr, LiteAw, LiteB, LiteBridge, LitePort, LiteR, LiteW,
};
use txhdl_parts::bus::axi_pins::{AxiHostPins, AxiPins, AxiPinsIn, AxiPinsOut};
use txhdl_parts::bus::router::Router;
use txhdl_parts::cdc::ChanCdc;
use txhdl_parts::cfgflash::CfgFlash;
use txhdl_parts::dma::{LineFetch, LineStore, NoBeats, NoReads};
use txhdl_parts::dtm::{Dtm, DtmBridge, Tck};
use txhdl_parts::eth::EthByte;
use txhdl_parts::ethdma::{FrameIn, FrameLen, FrameOut};
use txhdl_parts::ethshare::EthShare;
use txhdl_parts::ethslots::EthSlots;
use txhdl_parts::flashwin::FlashWin;
use txhdl_parts::mdio::{Mdio, MdioLines};
use txhdl_parts::plic::Plic3;
use txhdl_parts::pwm::Pwm;
use txhdl_parts::remote::eth::{RemoteLink, ETHERTYPE};
use txhdl_parts::remote::{Answer as RemoteAnswer, Ask, Remote};
use txhdl_parts::scanout::ScanFetch;
use txhdl_parts::sd::{Sd, SdLines};
use txhdl_parts::spi::{Spi, SpiLines};
use txhdl_parts::trng::Entropy;

/// Which device this board answers to on the wire. Every frame the
/// remote peripheral sends carries it, and a program answers each
/// device by the number it was asked under, so a wire with two of
/// these boards on it gives them a number each.
pub const REMOTE_DEV: usize = 1;

/// How long the remote peripheral waits for its program before it
/// answers the bus `SlvErr` itself: one second, at the board's 100 MHz.
/// A round trip is two frames, a program on another machine, and
/// whatever the network between them adds. Twenty milliseconds, the
/// first value, was a budget for a program on the board's own network
/// and ruled out a program anywhere else: measured on September 22,
/// 2026, a program 175 ms away by round trip answered every
/// transaction and the core read zero, the bus having refused first
/// (issue 397). One second is five times that path with room for a
/// worse one, and it is what a device that has gone away costs the
/// bus per access, which a person waits out and the serial watcher
/// outlasts. The count is cycles of the board's clock; a board on
/// another clock states its own.
pub const REMOTE_WAIT: usize = 100_000_000;

/// The window's clock divider: a half of a bit is four cycles, the
/// least `CfgFlash` allows, so the flash is clocked at an eighth of
/// the board's 100 MHz.
pub const FLASH_DIV: usize = 3;

// begin{map}
/// The address map: each range's base and the bits of an address that
/// must equal it, in the order of the router's ports. The data memory,
/// the peripheral page and the boot memory are 4 KiB each; the timer
/// and the software interrupt, and the debug module (issue 154), 64 KiB
/// each; the memory is the quarter of the address space from
/// `0x4000_0000`, the interrupt controller the 64 MiB from
/// `0x0c00_0000`, and the configuration flash, read as memory, the
/// 16 MiB from `0x2000_0000` (issue 312).
pub struct BoardMap;

impl AddrMap<8> for BoardMap {
    const RANGES: [(usize, usize); 8] = [
        (0x1000, 0xffff_f000),
        (0x0200_0000, 0xffff_0000),
        (0x3000, 0xffff_f000),
        (0x4000_0000, 0xc000_0000),
        (0x0c00_0000, 0xfc00_0000),
        (0x0000_0000, 0xffff_f000),
        (0x1000_0000, 0xffff_0000),
        (0x2000_0000, 0xff00_0000),
    ];
    const NAMES: [&'static str; 8] = [
        "the data memory",
        "the timer and the software interrupt",
        "the peripheral page, behind an AXI-Lite bridge",
        "the DDR3 memory",
        "the platform-level interrupt controller",
        "the boot memory, read only",
        "the debug module",
        "the configuration flash, read as memory",
    ];
}

/// The platform-level interrupt controller's sources, in the order of
/// the array `Board` gives it: source `n` is entry `n - 1`. Source 0
/// is reserved, as every PLIC's is. The device tree reads the numbers
/// from here (issue 1015).
pub const PLIC_SOURCES: [&str; 3] = ["serial", "irq", "ethernet"];

pub type BoardRouter = Router<8, BoardMap, 32, 32, 4, 5>;

/// Where Razboj draws, and where its display list is: both in the DDR3
/// (issue 985). The frame is the one the scanout shows, rows of 1024
/// words, so the scanout's stride is 4096 bytes. The list may hold the
/// 65 535 entries a count allows, four megabytes.
pub const RAZBOJ_FB: usize = 0x4200_0000;
pub const RAZBOJ_DL: usize = 0x4280_0000;
/// Razboj's doorbell, the tenth slot of the peripheral page: the count
/// a program writes last to start a list, which Razboj reads and clears.
pub const RAZBOJ_DOORBELL: usize = 0x3900;
// end{map}

// begin{litemaps}
/// Where the slots behind the peripheral bridge are: 256 bytes each
/// from 0x3000, in the order of the bridge's ports.
pub struct SlotMap;

impl AddrMap<10> for SlotMap {
    const RANGES: [(usize, usize); 10] = [
        (0x3000, 0xffff_ff00),
        (0x3100, 0xffff_ff00),
        (0x3200, 0xffff_ff00),
        (0x3300, 0xffff_ff00),
        (0x3400, 0xffff_ff00),
        (0x3500, 0xffff_ff00),
        (0x3600, 0xffff_ff00),
        (0x3700, 0xffff_ff00),
        (0x3800, 0xffff_ff00),
        (0x3900, 0xffff_ff00),
    ];
    const NAMES: [&'static str; 10] = [
        "the serial port",
        "the pulse width modulator",
        "the third slot, brought out of the unit",
        "the remote peripheral",
        "the Ethernet port's registers",
        "the entropy source",
        "the configuration flash's SPI master",
        "the Ethernet PHY's management interface",
        "the SD card host",
        "Razboj's doorbell",
    ];
}

/// Where the interrupt controller is, behind a bridge of its own.
pub struct PlicMap;

impl AddrMap<1> for PlicMap {
    const RANGES: [(usize, usize); 1] = [(0x0c00_0000, 0xfc00_0000)];
}

/// Where the debug module is, behind a bridge of its own.
pub struct DmMap;

impl AddrMap<1> for DmMap {
    const RANGES: [(usize, usize); 1] = [(0x1000_0000, 0xffff_0000)];
}
// end{litemaps}

/// Where the transport's bridge finds the debug module's registers:
/// `DmMap`'s base, as a number a type can carry.
pub const DM_AT: usize = <DmMap as AddrMap<1>>::RANGES[0].0;

// begin{board}
/// The board's design.
#[derive(Trace, Default)]
pub struct Board<const DIV: u32> {
    pub cpu: Hart<2>,
    pub host: AxiHost<32, 32, 4, 2, 4>,
    /// The second host: Vivado's JTAG-to-AXI master, on the top beside
    /// the board's, reached over the cable that programs the part, so
    /// that memory and every peripheral can be read and written whatever
    /// the core is doing (issue 241). Its pins arrive as the board's
    /// own, and this joins them to the link's channels; a top with no
    /// master ties them off.
    pub jtag: AxiPins<32, 32, 4, 1>,
    /// The RISC-V debug transport (issue 154), behind the top's
    /// `BSCANE2` on `USER4` and on the cable's clock, so that a stock
    /// OpenOCD, and gdb through it, reaches the debug module from the
    /// cable that programs the part.
    pub dtm: Dtm,
    /// The transport's accesses across to the board's clock, and the
    /// answers back across to the cable's.
    pub dreq: ChanCdc<U<41>, 2, 4, 3, Tck, DefaultClock>,
    pub dans: ChanCdc<U<34>, 2, 4, 3, DefaultClock, Tck>,
    /// The transport's half on the board's clock: each access a read or
    /// a write on the link, the debug module's registers at `DM_AT`
    /// and its system bus access served here, through a host of its
    /// own. One access at a time, so one bit of identifier.
    pub dbridge: DtmBridge<DM_AT, 1>,
    pub dhost: AxiHost<32, 32, 4, 1, 2>,
    /// The JTAG master and the transport's bridge share the arbiter's
    /// second host, each with one bit of identifier, so that nothing
    /// past the arbiter widens: the JTAG master's IP only ever used one
    /// of the two bits it had. Taking turns, as the arbiter does.
    pub jarb: Arbiter2<32, 32, 4, 1, 2, 0>,
    /// The seven hosts onto one link: the core, the JTAG master, the
    /// Ethernet port's two engines, the one that fetches a frame to
    /// send and the one that stores a frame received, the video
    /// scanout's fetch (issue 151), the SD card's two engines behind
    /// `sdarb` (issue 912), and Razboj's rasteriser (issue 985). The peripheral side carries five
    /// bits of identifier, two for the hosts' own and three for the
    /// port, which is room for eight: a sixth, seventh and eighth host
    /// raise the count and widen nothing (issue 1022). Four bits held
    /// exactly four hosts, and the scanout had shared the send engine's
    /// port until the user chose widening over nesting further.
    ///
    /// Taking turns rather than fixed priority, so that an engine
    /// moving a frame cannot hold the core off the bus for the length
    /// of it. With every host offering, each wins one turn in seven.
    pub arb: Arbiter7<32, 32, 4, 2, 5, 0>,
    pub router: BoardRouter,
    pub pdmem: AxiPer<32, 32, 4, 5>,
    pub ptimer: AxiPer<32, 32, 4, 5>,
    // begin{vslot}
    /// Ten small peripherals share the page at `0x3000`: the serial
    /// port at `0x3000`, the pulse width modulator at `0x3100`,
    /// whatever the board hangs on the third slot at `0x3200`, the
    /// remote peripheral at `0x3300`, the Ethernet port's registers
    /// on the fifth slot at `0x3400`, the entropy source on the
    /// sixth at `0x3500`, the configuration flash's SPI master on the
    /// seventh at `0x3600`, the Ethernet PHY's MDIO master on the
    /// eighth at `0x3700`, the SD card host on the ninth at `0x3800`,
    /// and Razboj's doorbell on the tenth at `0x3900`, each a
    /// sixteenth of the page. The
    /// router's ports go to memories and to the bus's own peripherals,
    /// and a peripheral of six registers does not want one of its own.
    ///
    /// The page was never the constraint and is not now. It is 4 KiB
    /// and a slot is 256 bytes, so it holds sixteen and six are
    /// still free; what was full was the bridge in front of it, which
    /// had four ports. So no address moves to make room for the fifth,
    /// and nothing that names one of the first four changes.
    ///
    /// The third slot leaves this unit as ports rather than reaching a
    /// field, because what sits there runs on a clock of its own: on
    /// the board it is the video peripheral on the pixel clock, and
    /// the crossing between the two is the board top's business. A
    /// design with nothing there ties the slot off, and a read of it
    /// answers when the tie-off does.
    ///
    /// The fifth is a field, because `EthSlots` runs on the bus clock
    /// like every other peripheral here and wants no crossing.
    pub puart: LiteBridge<10, SlotMap, 32, 32, 4, 5>,
    // end{vslot}
    pub pplic: LiteBridge<1, PlicMap, 32, 32, 4, 5>,
    /// The debug module, on a router port of its own behind a bridge
    /// of its own, so the JTAG host reaches it while the core is
    /// halted (issue 154).
    pub pdm: LiteBridge<1, DmMap, 32, 32, 4, 5>,
    pub dmod: Dm,
    /// The boot memory on the bus, at address zero, readable and not
    /// writable: the same words the core fetches from inside itself,
    /// so a load can read a constant beside the code (#268).
    pub prom: AxiPer<32, 32, 4, 5>,
    pub rom: Rom<5>,
    pub dmem: Dmem<5>,
    pub timer: Timer<5>,
    pub uart: Uart<DIV>,
    pub pwm: Pwm,
    pub ddr3: Ddr3Per,
    // begin{remote}
    /// The peripheral at `0x3300`, whose behaviour is a program on
    /// another machine, and the link that puts its transactions on the
    /// Ethernet port as frames. Both run on the core's clock; the
    /// crossing to the port's two clocks is the board top's business,
    /// as the video slot's is, and it carries a byte and a last bit,
    /// which is what the MAC speaks.
    pub remote: Remote<REMOTE_WAIT>,
    pub link: RemoteLink<REMOTE_DEV>,
    // end{remote}
    // begin{ethslot}
    /// The Ethernet port's registers, on the fifth slot at `0x3400`,
    /// with its four frame buffers in the memory from
    /// [`isa::ETH_BUF_BASE`](crate::isa::ETH_BUF_BASE).
    ///
    /// It answers its registers and moves no frames itself. The seven
    /// ports it has besides the bus face the engines that carry the
    /// bytes, below with the rest of issue 151: it says which slot and
    /// how many bytes, and they move them.
    pub eth: EthSlots<{ isa::ETH_BUF_BASE as usize }>,
    // end{ethslot}
    // begin{entropy}
    /// The entropy source, on the sixth slot at `0x3500`: eight ring
    /// oscillators as a foreign module, and the peripheral that folds,
    /// checks, debiases and buffers their samples (issue 458). Zephyr's
    /// entropy driver reads it, and the network stack's random numbers
    /// come from there rather than from a counter.
    pub entropy: Entropy,
    // end{entropy}
    // begin{cfgflash}
    /// The configuration flash, the chip that holds the bitstream
    /// (issue 312): the SPI master on the seventh slot at `0x3600`, for
    /// commands such as the chip's identity; the window at
    /// `0x2000_0000`, where an ordinary read is answered with what the
    /// flash holds; and between them and the chip, the startup block
    /// and the pins, which wait for the end of configuration and refuse
    /// write enable, so nothing the core runs can change the
    /// bitstream.
    pub spi: Spi,
    /// The Ethernet PHY's management interface (issue 864): an MDIO
    /// master on the eighth slot at `0x3700`, so a program can read the
    /// PHY's registers. Its interrupt is not wired, since a program waits
    /// on it.
    pub mdio: Mdio,
    /// The SD card host on the ninth slot at `0x3800` (issue 153). Its
    /// interrupt is not wired, as the SPI master's is not: the
    /// interrupt controller's three sources are taken, and a program
    /// waits on the status register.
    pub sd: Sd,
    /// The SD host's engines (issue 912): a read's blocks stored into
    /// memory as the card sends them, a write's fetched out of it as
    /// the card takes them, each with a host of its own. One command
    /// is a read or a write and never both, and the host starts no
    /// command until the last one's engine has finished, so the two
    /// never move words at once: the `dmaon` state with the command
    /// word's read and write bits makes them exclusive, and the host
    /// checks that the two busy lines are never high together.
    pub sdstore: LineStore<32, 1, 16, 16>,
    pub sdfetch: LineFetch<32, 1, 16, 16>,
    pub sdshost: AxiHost<32, 32, 4, 1, 2>,
    pub sdfhost: AxiHost<32, 32, 4, 1, 2>,
    /// The store engine never reads, and the fetch engine never writes.
    pub sdnoreads: NoReads<1>,
    pub sdnobeats: NoBeats,
    /// The two engines onto the arbiter's sixth port, a bit of
    /// identifier each. This is a nest, which the arbiter's widening
    /// was meant to end, and it is one because the reason does not
    /// apply: hosts that share a port share its turns, and these two
    /// never offer at the same time, so neither takes a turn from the
    /// other. It keeps two ports free, for Razboj and a texture fetch.
    pub sdarb: Arbiter2<32, 32, 4, 1, 2, 0>,
    /// The window's tracker, on the router's eighth port.
    pub pflash: AxiPer<32, 32, 4, 5>,
    /// The window.
    pub flashwin: FlashWin<FLASH_DIV, 5>,
    /// The startup block and the pins, write enable refused.
    pub cfgflash: CfgFlash<1>,
    // end{cfgflash}
    // begin{ethdma}
    /// The engines behind the Ethernet port's registers, and what
    /// stands between them and the wire (issue 151).
    ///
    /// Sending: the fetch engine reads the frame's words out of its
    /// slot and `FrameOut` turns them into bytes, the last of them the
    /// frame's last. Receiving: `FrameLen` finds the frame's length
    /// again after the clock crossing, `FrameIn` packs its bytes into
    /// words, and the store engine writes them into a slot.
    ///
    /// Each engine is a host of its own on the link, since each issues
    /// bursts of its own, and the wire is shared with the remote
    /// peripheral by EtherType.
    pub fhost: AxiHost<32, 32, 4, 2, 4>,
    pub shost: AxiHost<32, 32, 4, 2, 4>,
    pub fetch: LineFetch<32, 2, 16, 16>,
    pub store: LineStore<32, 2, 16, 16>,
    pub fout: FrameOut,
    pub flen: FrameLen,
    pub fin: FrameIn,
    /// The one wire, split by EtherType: the remote peripheral's frames
    /// to it, every other frame to the Ethernet port.
    pub share: EthShare<{ ETHERTYPE as usize }>,
    /// Each engine uses one direction of its host, and a board joins
    /// every channel to a unit, so the direction each engine never uses
    /// is held by one that uses it no more: the fetch engine's write
    /// beats, and the store engine's read data.
    pub fnobeats: NoBeats,
    pub snoreads: NoReads<2>,
    // end{ethdma}
    // begin{scan}
    /// The video scanout's bus side (issue 151): `ScanFetch` takes each
    /// line request that arrives on `scan_req` from the pixel clock and
    /// starts `vfetch` on it, whose words leave on `scan_words` for the
    /// line pair. The crossings are the board top's, as the video
    /// slot's are.
    pub scan: ScanFetch<32, 16, 640>,
    /// Bursts of 64 beats, a line in ten, so that a line pays the DDR3
    /// path's latency and waits its turn at the arbiter ten times rather
    /// than forty: under the core's copying and the Ethernet port's
    /// sending, a line took 4682 cycles of its 3175 in bursts of
    /// sixteen, and takes 1206 (issue 1209). Bursts of 128 took 942,
    /// but a load of the core's behind one waited up to 153 cycles
    /// where it waits 94 behind 64 and 88 with no scanout.
    pub vfetch: LineFetch<32, 2, 64, 16>,
    pub vhost: AxiHost<32, 32, 4, 2, 4>,
    /// The scanout's fetch never writes.
    pub vnobeats: NoBeats,
    // end{scan}
    /// Razboj's rasteriser (issue 985), the arbiter's seventh host. It
    /// draws into the frame the scanout shows, at `RAZBOJ_FB`, from
    /// the display list at `RAZBOJ_DL`, and polls its doorbell on the
    /// tenth slot for the list's length.
    pub raster: Raster<32, 2, 10, 480, RAZBOJ_FB, RAZBOJ_DL, RAZBOJ_DOORBELL>,
    pub rhost: AxiHost<32, 32, 4, 2, 4>,
    pub doorbell: Doorbell,
    /// Three sources, each asking while its line is high: the serial
    /// port's receive interrupt, the board's own `irq` input, and the
    /// Ethernet port's arrival.
    pub plic: Plic3<0>,
}
// end{board}

// begin{ports}
/// The board's inputs: the resets, the interrupt, the serial line, the
/// board's clock and the controller's reset, the video slot's answers, the frames arriving, and
/// the JTAG master's pins, named as AXI4 names them under `jtag_`,
/// as one field.
pub struct BoardIn {
    pub rst: In<Bit>,
    pub irq: In<Bit>,
    pub rx: In<Bit>,
    pub sys_clk: In<Bit>,
    pub sys_rst: In<Bit>,
    pub vb: Rx<LiteB>,
    pub vr: Rx<LiteR<32>>,
    pub net_rx: Rx<EthByte>,
    /// The MAC's drops ahead of the slots, room, check and size, each
    /// sixteen bits in Gray code: the receiver counts them on the
    /// PHY's clock, and the top samples them through two flip-flops
    /// onto this one (issue 1404).
    pub mac_drops: In<U<48>>,
    /// The JTAG master's pins, one field; its ports are `jtag_awid`
    /// and the rest (issue 579).
    pub jtag: AxiHostPins<32, 32, 4, 1>,
    /// The configuration flash's data out, pin D01.
    pub fl_miso: In<Bit>,
    /// The Ethernet PHY's management data line, as read at the pad.
    pub phy_mdio_in: In<Bit>,
    /// The SD card's command line and four data lines, as read at
    /// the pads.
    pub sd_cmd_in: In<Bit>,
    pub sd_dat_in: In<U<4>>,
    /// The top's `BSCANE2` on `USER4`, on the cable's clock: selected,
    /// the TAP's shift, capture and update, the data in, and the TAP's
    /// reset (issue 154).
    pub bscan_sel: In<Bit, Tck>,
    pub bscan_shift: In<Bit, Tck>,
    pub bscan_capture: In<Bit, Tck>,
    pub bscan_update: In<Bit, Tck>,
    pub bscan_tdi: In<Bit, Tck>,
    pub bscan_reset: In<Bit, Tck>,
    /// The scanout's line requests, from the pixel clock's side.
    pub scan_req: Rx<U<32>>,
}

/// The board's outputs: the halt and the serial line, the modulator,
/// the memory's pins, the video slot's requests, the frames leaving,
/// and the JTAG master's answers.
pub struct BoardOut {
    pub halt: Out<Bit>,
    pub tx: Out<Bit>,
    pub pwm_pins: Out<U<4>>,
    pub calib: Out<Bit>,
    pub ui_clk: Out<Bit>,
    pub ui_rst: Out<Bit>,
    pub ck_p: Out<Bit>,
    pub ck_n: Out<Bit>,
    pub mem_rst_n: Out<Bit>,
    pub cke: Out<Bit>,
    pub cs_n: Out<Bit>,
    pub ras_n: Out<Bit>,
    pub cas_n: Out<Bit>,
    pub we_n: Out<Bit>,
    pub row: Out<U<15>>,
    pub bank: Out<U<3>>,
    pub dm: Out<U<4>>,
    pub odt: Out<Bit>,
    pub dq: Pad<U<32>>,
    pub dqs: Pad<U<4>>,
    pub dqs_n: Pad<U<4>>,
    pub vaw: Tx<LiteAw<32>>,
    pub var: Tx<LiteAr<32>>,
    pub vw: Tx<LiteW<32, 4>>,
    pub net_tx: Tx<EthByte>,
    pub jtag_awready: Out<Bit>,
    pub jtag_wready: Out<Bit>,
    pub jtag_bid: Out<U<1>>,
    pub jtag_bresp: Out<U<2>>,
    pub jtag_bvalid: Out<Bit>,
    pub jtag_arready: Out<Bit>,
    pub jtag_rid: Out<U<1>>,
    pub jtag_rdata: Out<U<32>>,
    pub jtag_rresp: Out<U<2>>,
    pub jtag_rlast: Out<Bit>,
    pub jtag_rvalid: Out<Bit>,
    /// The configuration flash's select and data in, pins FCS_B and
    /// D00; its clock pin as the startup block's model shows it, which
    /// a board leaves open, since the primitive drives the real one;
    /// and whether the pins refused write enable.
    pub fl_cs_n: Out<Bit>,
    /// The flash's data in.
    pub fl_mosi: Out<Bit>,
    /// The flash's clock, in simulation only.
    pub fl_cclk: Out<Bit>,
    /// Write enable was refused.
    pub fl_refused: Out<Bit>,
    /// The Ethernet PHY's management clock.
    pub phy_mdc: Out<Bit>,
    /// The level driven on the management data line.
    pub phy_mdio_out: Out<Bit>,
    /// High while the board drives the management data line; the top
    /// makes the pad three-state from the two.
    pub phy_mdio_oe: Out<Bit>,
    /// The SD card's clock, its command line and four data lines as
    /// driven, and each one's drive enable; the top makes the pads
    /// three-state from the pairs.
    pub sd_clk: Out<Bit>,
    pub sd_cmd_out: Out<Bit>,
    pub sd_cmd_oe: Out<Bit>,
    pub sd_dat_out: Out<U<4>>,
    pub sd_dat_oe: Out<Bit>,
    /// The transport's data out, to `BSCANE2`'s TDO.
    pub bscan_tdo: Out<Bit, Tck>,
    /// The scanout's words, to the pixel clock's side.
    pub scan_words: Tx<U<32>>,
}
// end{ports}

#[lower]
impl<const DIV: u32> Unit for Board<DIV> {
    async fn run(
        &mut self,
        BoardIn {
            rst,
            irq,
            rx,
            sys_clk,
            sys_rst,
            vb,
            vr,
            net_rx,
            mac_drops,
            jtag,
            fl_miso,
            phy_mdio_in,
            sd_cmd_in,
            sd_dat_in,
            bscan_sel,
            bscan_shift,
            bscan_capture,
            bscan_update,
            bscan_tdi,
            bscan_reset,
            scan_req,
        }: BoardIn,
        BoardOut {
            halt,
            tx,
            pwm_pins,
            calib,
            ui_clk,
            ui_rst,
            ck_p,
            ck_n,
            mem_rst_n,
            cke,
            cs_n,
            ras_n,
            cas_n,
            we_n,
            row,
            bank,
            dm,
            odt,
            dq,
            dqs,
            dqs_n,
            vaw,
            var,
            vw,
            net_tx,
            jtag_awready,
            jtag_wready,
            jtag_bid,
            jtag_bresp,
            jtag_bvalid,
            jtag_arready,
            jtag_rid,
            jtag_rdata,
            jtag_rresp,
            jtag_rlast,
            jtag_rvalid,
            fl_cs_n,
            fl_mosi,
            fl_cclk,
            fl_refused,
            phy_mdc,
            phy_mdio_out,
            phy_mdio_oe,
            sd_clk,
            sd_cmd_out,
            sd_cmd_oe,
            sd_dat_out,
            sd_dat_oe,
            bscan_tdo,
            scan_words,
        }: BoardOut,
    ) {
        // The reset, read by the core, the timer and the serial port.
        let rst_timer = rst.clone();
        let rst_uart = rst.clone();
        let rst_plic = rst.clone();
        let rst_bell = rst.clone();
        // The core and its tracker.
        let (issue_tx, issue_rx) = chan::<Issue<32>, DefaultClock>();
        let (wbeat_tx, wbeat_rx) = chan::<W<32, 4>, DefaultClock>();
        let (release_tx, release_rx) = chan::<Grant<2>, DefaultClock>();
        let (grant_tx, grant_rx) = chan::<Grant<2>, DefaultClock>();
        let (done_tx, done_rx) = chan::<Done<2>, DefaultClock>();
        let (rdata_tx, rdata_rx) = chan::<R<32, 2>, DefaultClock>();
        let (instr_o, _instr_i) = signal::<U<32>, DefaultClock>();
        // The debug module's lines to the core and back: the two
        // requests, the register access, and debug mode with the word
        // read (issue 154).
        let (haltreq_o, haltreq_i) = signal::<Bit, DefaultClock>();
        let (resumereq_o, resumereq_i) = signal::<Bit, DefaultClock>();
        let (dbg_regno_o, dbg_regno_i) = signal::<U<16>, DefaultClock>();
        let (dbg_wdata_o, dbg_wdata_i) = signal::<U<32>, DefaultClock>();
        let (dbg_we_o, dbg_we_i) = signal::<Bit, DefaultClock>();
        let (debug_o, debug_i) = signal::<Bit, DefaultClock>();
        let (dbg_rdata_o, dbg_rdata_i) = signal::<U<32>, DefaultClock>();
        let (retire_o, _retire_i) = signal::<Writeback, DefaultClock>();
        let (tirq_o, tirq_i) = signal::<Bit, DefaultClock>();
        // The software interrupt the controller raises for a program.
        let (sirq_o, sirq_i) = signal::<Bit, DefaultClock>();
        let (time_o, time_i) = signal::<U<64>, DefaultClock>();
        let (uirq_o, uirq_i) = signal::<Bit, DefaultClock>();
        let (eirq_o, eirq_i) = signal::<Bit, DefaultClock>();
        // The PLIC's supervisor line, `mip.SEIP`'s, to the core (issue
        // 1094).
        let (seirq_o, seirq_i) = signal::<Bit, DefaultClock>();
        // The tracker and the router.
        let (aw_tx, aw_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (ar_tx, ar_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (w_tx, w_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b_tx, b_rx) = chan::<B<2>, DefaultClock>();
        let (r_tx, r_rx) = chan::<R<32, 2>, DefaultClock>();
        // The JTAG master's pins and the arbiter, under the hosts' own
        // identifiers; and the arbiter and the router, under the wider.
        let (jaw_tx, jaw_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (jar_tx, jar_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (jw_tx, jw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (jb_tx, jb_rx) = chan::<B<2>, DefaultClock>();
        let (jr_tx, jr_rx) = chan::<R<32, 2>, DefaultClock>();
        // The two that share the arbiter's second host, one bit of
        // identifier each: the JTAG master's pins and the transport's
        // bridge (issue 154).
        let (kaw_tx, kaw_rx) = chan::<Aw<32, 1>, DefaultClock>();
        let (kar_tx, kar_rx) = chan::<Ar<32, 1>, DefaultClock>();
        let (kw_tx, kw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (kb_tx, kb_rx) = chan::<B<1>, DefaultClock>();
        let (kr_tx, kr_rx) = chan::<R<32, 1>, DefaultClock>();
        let (taw_tx, taw_rx) = chan::<Aw<32, 1>, DefaultClock>();
        let (tar_tx, tar_rx) = chan::<Ar<32, 1>, DefaultClock>();
        let (tw_tx, tw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (tb_tx, tb_rx) = chan::<B<1>, DefaultClock>();
        let (tr_tx, tr_rx) = chan::<R<32, 1>, DefaultClock>();
        let (tissue_tx, tissue_rx) = chan::<Issue<32>, DefaultClock>();
        let (twbeat_tx, twbeat_rx) = chan::<W<32, 4>, DefaultClock>();
        let (trelease_tx, trelease_rx) = chan::<Grant<1>, DefaultClock>();
        let (tgrant_tx, tgrant_rx) = chan::<Grant<1>, DefaultClock>();
        let (tdone_tx, tdone_rx) = chan::<Done<1>, DefaultClock>();
        let (trdata_tx, trdata_rx) = chan::<R<32, 1>, DefaultClock>();
        // The transport's accesses and answers, on each clock.
        let (dreq_tx, dreq_rx) = chan::<U<41>, Tck>();
        let (dreqx_tx, dreqx_rx) = chan::<U<41>, DefaultClock>();
        let (dans_tx, dans_rx) = chan::<U<34>, DefaultClock>();
        let (dansx_tx, dansx_rx) = chan::<U<34>, Tck>();
        let (xaw_tx, xaw_rx) = chan::<Aw<32, 5>, DefaultClock>();
        let (xar_tx, xar_rx) = chan::<Ar<32, 5>, DefaultClock>();
        let (xw_tx, xw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (xb_tx, xb_rx) = chan::<B<5>, DefaultClock>();
        let (xr_tx, xr_rx) = chan::<R<32, 5>, DefaultClock>();
        // The router and each peripheral's tracker.
        let (aw0_tx, aw0_rx) = chan::<Aw<32, 5>, DefaultClock>();
        let (ar0_tx, ar0_rx) = chan::<Ar<32, 5>, DefaultClock>();
        let (w0_tx, w0_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b0_tx, b0_rx) = chan::<B<5>, DefaultClock>();
        #[unregistered]
        let (r0_tx, r0_rx) = chan::<R<32, 5>, DefaultClock>();
        let (aw1_tx, aw1_rx) = chan::<Aw<32, 5>, DefaultClock>();
        let (ar1_tx, ar1_rx) = chan::<Ar<32, 5>, DefaultClock>();
        let (w1_tx, w1_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b1_tx, b1_rx) = chan::<B<5>, DefaultClock>();
        let (r1_tx, r1_rx) = chan::<R<32, 5>, DefaultClock>();
        let (aw2_tx, aw2_rx) = chan::<Aw<32, 5>, DefaultClock>();
        let (ar2_tx, ar2_rx) = chan::<Ar<32, 5>, DefaultClock>();
        let (w2_tx, w2_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b2_tx, b2_rx) = chan::<B<5>, DefaultClock>();
        let (r2_tx, r2_rx) = chan::<R<32, 5>, DefaultClock>();
        let (aw3_tx, aw3_rx) = chan::<Aw<32, 5>, DefaultClock>();
        let (ar3_tx, ar3_rx) = chan::<Ar<32, 5>, DefaultClock>();
        let (w3_tx, w3_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b3_tx, b3_rx) = chan::<B<5>, DefaultClock>();
        let (r3_tx, r3_rx) = chan::<R<32, 5>, DefaultClock>();
        let (aw4_tx, aw4_rx) = chan::<Aw<32, 5>, DefaultClock>();
        let (ar4_tx, ar4_rx) = chan::<Ar<32, 5>, DefaultClock>();
        let (w4_tx, w4_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b4_tx, b4_rx) = chan::<B<5>, DefaultClock>();
        let (r4_tx, r4_rx) = chan::<R<32, 5>, DefaultClock>();
        let (aw5_tx, aw5_rx) = chan::<Aw<32, 5>, DefaultClock>();
        let (ar5_tx, ar5_rx) = chan::<Ar<32, 5>, DefaultClock>();
        let (w5_tx, w5_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b5_tx, b5_rx) = chan::<B<5>, DefaultClock>();
        let (r5_tx, r5_rx) = chan::<R<32, 5>, DefaultClock>();
        // The boot memory's tracker and the memory.
        let (req5_tx, req5_rx) = chan::<PerReq<32, 5>, DefaultClock>();
        let (wd5_tx, wd5_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans5_tx, ans5_rx) = chan::<Answer<5>, DefaultClock>();
        let (rb5_tx, rb5_rx) = chan::<R<32, 5>, DefaultClock>();
        // Each peripheral's tracker and the peripheral.
        #[unregistered]
        let (req0_tx, req0_rx) = chan::<PerReq<32, 5>, DefaultClock>();
        let (wd0_tx, wd0_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans0_tx, ans0_rx) = chan::<Answer<5>, DefaultClock>();
        let (rb0_tx, rb0_rx) = chan::<R<32, 5>, DefaultClock>();
        let (req1_tx, req1_rx) = chan::<PerReq<32, 5>, DefaultClock>();
        let (wd1_tx, wd1_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans1_tx, ans1_rx) = chan::<Answer<5>, DefaultClock>();
        let (rb1_tx, rb1_rx) = chan::<R<32, 5>, DefaultClock>();
        // The serial port speaks AXI-Lite, behind its bridge.
        let (law_tx, law_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (lar_tx, lar_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (lw_tx, lw_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (lb_tx, lb_rx) = chan::<LiteB, DefaultClock>();
        let (lr_tx, lr_rx) = chan::<LiteR<32>, DefaultClock>();
        // The modulator's side of the same bridge.
        let (paw_pwm_tx, paw_pwm_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_pwm_tx, par_pwm_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_pwm_tx, pw_pwm_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_pwm_tx, pb_pwm_rx) = chan::<LiteB, DefaultClock>();
        let (pr_pwm_tx, pr_pwm_rx) = chan::<LiteR<32>, DefaultClock>();
        // The entropy source's side of the same bridge.
        let (paw_trng_tx, paw_trng_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_trng_tx, par_trng_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_trng_tx, pw_trng_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_trng_tx, pb_trng_rx) = chan::<LiteB, DefaultClock>();
        let (pr_trng_tx, pr_trng_rx) = chan::<LiteR<32>, DefaultClock>();
        // The flash's master on the seventh slot, and its wires to the
        // pins; its interrupt is not used, since a program waits on it.
        let (paw_spi_tx, paw_spi_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_spi_tx, par_spi_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_spi_tx, pw_spi_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_spi_tx, pb_spi_rx) = chan::<LiteB, DefaultClock>();
        let (pr_spi_tx, pr_spi_rx) = chan::<LiteR<32>, DefaultClock>();
        let (m_sclk_o, m_sclk_i) = signal::<Bit, DefaultClock>();
        let (m_mosi_o, m_mosi_i) = signal::<Bit, DefaultClock>();
        let (m_cs_n_o, m_cs_n_i) = signal::<Bit, DefaultClock>();
        let (spi_irq_o, _spi_irq_i) = signal::<Bit, DefaultClock>();
        // The PHY's management master on the eighth slot.
        let (paw_mdio_tx, paw_mdio_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_mdio_tx, par_mdio_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_mdio_tx, pw_mdio_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_mdio_tx, pb_mdio_rx) = chan::<LiteB, DefaultClock>();
        let (pr_mdio_tx, pr_mdio_rx) = chan::<LiteR<32>, DefaultClock>();
        // The SD card host on the ninth.
        let (paw_sd_tx, paw_sd_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_sd_tx, par_sd_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_sd_tx, pw_sd_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_sd_tx, pb_sd_rx) = chan::<LiteB, DefaultClock>();
        let (pr_sd_tx, pr_sd_rx) = chan::<LiteR<32>, DefaultClock>();
        let (sd_irq_o, _sd_irq_i) = signal::<Bit, DefaultClock>();
        // The SD host's engines, their hosts, and `sdarb`'s side of the
        // arbiter's sixth port (issue 912).
        let (sdout_tx, sdout_rx) = chan::<U<32>, DefaultClock>();
        let (sdin_tx, sdin_rx) = chan::<U<32>, DefaultClock>();
        let (sdat_o, sdat_i) = signal::<U<32>, DefaultClock>();
        let sdat_f = sdat_i.clone();
        let (sdbytes_o, sdbytes_i) = signal::<U<16>, DefaultClock>();
        let (sdwords_o, sdwords_i) = signal::<U<16>, DefaultClock>();
        let (sdsgo_o, sdsgo_i) = signal::<Bit, DefaultClock>();
        let (sdfgo_o, sdfgo_i) = signal::<Bit, DefaultClock>();
        let (sdsbusy_o, sdsbusy_i) = signal::<Bit, DefaultClock>();
        let (sdfbusy_o, sdfbusy_i) = signal::<Bit, DefaultClock>();
        let (dsissue_tx, dsissue_rx) = chan::<Issue<32>, DefaultClock>();
        let (dswbeat_tx, dswbeat_rx) = chan::<W<32, 4>, DefaultClock>();
        let (dsrelease_tx, dsrelease_rx) = chan::<Grant<1>, DefaultClock>();
        let (dsgrant_tx, dsgrant_rx) = chan::<Grant<1>, DefaultClock>();
        let (dsdone_tx, dsdone_rx) = chan::<Done<1>, DefaultClock>();
        let (dsrdata_tx, dsrdata_rx) = chan::<R<32, 1>, DefaultClock>();
        let (dfissue_tx, dfissue_rx) = chan::<Issue<32>, DefaultClock>();
        let (dfwbeat_tx, dfwbeat_rx) = chan::<W<32, 4>, DefaultClock>();
        let (dfrelease_tx, dfrelease_rx) = chan::<Grant<1>, DefaultClock>();
        let (dfgrant_tx, dfgrant_rx) = chan::<Grant<1>, DefaultClock>();
        let (dfdone_tx, dfdone_rx) = chan::<Done<1>, DefaultClock>();
        let (dfrdata_tx, dfrdata_rx) = chan::<R<32, 1>, DefaultClock>();
        let (dsaw_tx, dsaw_rx) = chan::<Aw<32, 1>, DefaultClock>();
        let (dsar_tx, dsar_rx) = chan::<Ar<32, 1>, DefaultClock>();
        let (dsw_tx, dsw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (dsb_tx, dsb_rx) = chan::<B<1>, DefaultClock>();
        let (dsr_tx, dsr_rx) = chan::<R<32, 1>, DefaultClock>();
        let (dfaw_tx, dfaw_rx) = chan::<Aw<32, 1>, DefaultClock>();
        let (dfar_tx, dfar_rx) = chan::<Ar<32, 1>, DefaultClock>();
        let (dfw_tx, dfw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (dfb_tx, dfb_rx) = chan::<B<1>, DefaultClock>();
        let (dfr_tx, dfr_rx) = chan::<R<32, 1>, DefaultClock>();
        let (sdaw_tx, sdaw_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (sdar_tx, sdar_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (sdw_tx, sdw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (sdb_tx, sdb_rx) = chan::<B<2>, DefaultClock>();
        let (sdr_tx, sdr_rx) = chan::<R<32, 2>, DefaultClock>();
        // The window on the router's eighth port, its wires to the pins,
        // and the reset the pins hold it in until the flash is ready.
        let (aw7_tx, aw7_rx) = chan::<Aw<32, 5>, DefaultClock>();
        let (ar7_tx, ar7_rx) = chan::<Ar<32, 5>, DefaultClock>();
        let (w7_tx, w7_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b7_tx, b7_rx) = chan::<B<5>, DefaultClock>();
        let (r7_tx, r7_rx) = chan::<R<32, 5>, DefaultClock>();
        let (req7_tx, req7_rx) = chan::<PerReq<32, 5>, DefaultClock>();
        let (wd7_tx, wd7_rx) = chan::<W<32, 4>, DefaultClock>();
        let (ans7_tx, ans7_rx) = chan::<Answer<5>, DefaultClock>();
        let (rb7_tx, rb7_rx) = chan::<R<32, 5>, DefaultClock>();
        let (w_sclk_o, w_sclk_i) = signal::<Bit, DefaultClock>();
        let (w_mosi_o, w_mosi_i) = signal::<Bit, DefaultClock>();
        let (w_cs_n_o, w_cs_n_i) = signal::<Bit, DefaultClock>();
        let (w_rst_o, w_rst_i) = signal::<Bit, DefaultClock>();
        // The flash's data out goes to both.
        let fl_miso_win = fl_miso.clone();
        // The remote peripheral's side of the same bridge, and the two
        // channels between it and the link that makes the frames.
        let (paw_rem_tx, paw_rem_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_rem_tx, par_rem_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_rem_tx, pw_rem_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_rem_tx, pb_rem_rx) = chan::<LiteB, DefaultClock>();
        let (pr_rem_tx, pr_rem_rx) = chan::<LiteR<32>, DefaultClock>();
        let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<RemoteAnswer, DefaultClock>();
        // The fifth slot, to the Ethernet port's registers.
        let (paw_eth_tx, paw_eth_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_eth_tx, par_eth_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_eth_tx, pw_eth_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_eth_tx, pb_eth_rx) = chan::<LiteB, DefaultClock>();
        let (pr_eth_tx, pr_eth_rx) = chan::<LiteR<32>, DefaultClock>();
        // Its arrival line, which is the interrupt controller's third
        // source.
        let (eth_irq_o, eth_irq_i) = signal::<Bit, DefaultClock>();
        // What it says to the engines that move the frames, and what
        // they say back (issue 151).
        //
        // `tx_busy` is the byte side's `running` rather than the fetch
        // engine's, and there is no race in that. The fetch engine
        // goes idle the cycle after it hands over its last word, and
        // the byte side cannot finish until it has taken that word and
        // sent at least one more byte, so the fetch engine is always
        // idle strictly before the byte side is. A second transmit
        // therefore never finds the fetch engine still busy.
        //
        // `rx_busy` is the store engine's `running`, whose fall is the
        // first moment a received frame is certainly in memory, which
        // is what the register block waits for before it raises the
        // arrival.
        let (eth_tx_busy_o, eth_tx_busy_i) = signal::<Bit, DefaultClock>();
        let (eth_rx_busy_o, eth_rx_busy_i) = signal::<Bit, DefaultClock>();
        let (eth_rx_len_o, eth_rx_len_i) = signal::<U<16>, DefaultClock>();
        let (eth_rx_which_o, eth_rx_which_i) = signal::<U<1>, DefaultClock>();
        // Both receive slots hold a frame the driver has not released,
        // and the frames dropped for it (issue 1313).
        let (eth_rx_full_o, eth_rx_full_i) = signal::<Bit, DefaultClock>();
        let (eth_rx_drops_o, eth_rx_drops_i) = signal::<U<32>, DefaultClock>();
        let (eth_tx_base_o, eth_tx_base_i) = signal::<U<32>, DefaultClock>();
        let (eth_tx_bytes_o, eth_tx_bytes_i) = signal::<U<16>, DefaultClock>();
        let (eth_tx_start_o, eth_tx_start_i) = signal::<Bit, DefaultClock>();
        let (eth_rx_base_o, eth_rx_base_i) = signal::<U<32>, DefaultClock>();
        // A start goes to both sending units; a received frame's length
        // to both the store engine and the register block; and the
        // store engine's busy line to the register block and to the
        // unit that must not start the next frame under it.
        let eth_tx_start_fetch = eth_tx_start_i.clone();
        let eth_rx_len_store = eth_rx_len_i.clone();
        let eth_rx_busy_hold = eth_rx_busy_i.clone();
        // Sending: words from the fetch engine to the byte side, and the
        // word count the byte side works out for the engine.
        let (fword_tx, fword_rx) = chan::<U<32>, DefaultClock>();
        let (fnwords_o, fnwords_i) = signal::<U<16>, DefaultClock>();
        let (fetch_run_o, _fetch_run_i) = signal::<Bit, DefaultClock>();
        // Receiving: a frame with its length found again, then words to
        // the store engine.
        let (lenbyte_tx, lenbyte_rx) = chan::<EthByte, DefaultClock>();
        let (flen_len_o, flen_len_i) = signal::<U<16>, DefaultClock>();
        let (sword_tx, sword_rx) = chan::<U<32>, DefaultClock>();
        let (store_go_o, store_go_i) = signal::<Bit, DefaultClock>();
        // The one wire, with its two users on the sharing unit's sides:
        // the remote peripheral's link, and the Ethernet port.
        let (tolink_tx, tolink_rx) = chan::<EthByte, DefaultClock>();
        let (fromlink_tx, fromlink_rx) = chan::<EthByte, DefaultClock>();
        let (toeth_tx, toeth_rx) = chan::<EthByte, DefaultClock>();
        let (frometh_tx, frometh_rx) = chan::<EthByte, DefaultClock>();
        // Each engine's host, and its place on the arbiter.
        let (fissue_tx, fissue_rx) = chan::<Issue<32>, DefaultClock>();
        let (fwbeat_tx, fwbeat_rx) = chan::<W<32, 4>, DefaultClock>();
        let (frelease_tx, frelease_rx) = chan::<Grant<2>, DefaultClock>();
        let (fgrant_tx, fgrant_rx) = chan::<Grant<2>, DefaultClock>();
        let (fdone_tx, fdone_rx) = chan::<Done<2>, DefaultClock>();
        let (frdata_tx, frdata_rx) = chan::<R<32, 2>, DefaultClock>();
        // The scanout's host onto the arbiter's fifth port.
        let (scaw_tx, scaw_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (scar_tx, scar_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (scw_tx, scw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (scb_tx, scb_rx) = chan::<B<2>, DefaultClock>();
        let (scr_tx, scr_rx) = chan::<R<32, 2>, DefaultClock>();
        // The scanout's fetch and its host.
        let (scissue_tx, scissue_rx) = chan::<Issue<32>, DefaultClock>();
        let (scwbeat_tx, scwbeat_rx) = chan::<W<32, 4>, DefaultClock>();
        let (screlease_tx, screlease_rx) = chan::<Grant<2>, DefaultClock>();
        let (scgrant_tx, scgrant_rx) = chan::<Grant<2>, DefaultClock>();
        let (scdone_tx, scdone_rx) = chan::<Done<2>, DefaultClock>();
        let (scrdata_tx, scrdata_rx) = chan::<R<32, 2>, DefaultClock>();
        // Razboj's host onto the arbiter's seventh port, its rasteriser,
        // and its doorbell on the bridge's tenth slot.
        let (raw_tx, raw_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (rar_tx, rar_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (rw_tx, rw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (rb_tx, rb_rx) = chan::<B<2>, DefaultClock>();
        let (rr_tx, rr_rx) = chan::<R<32, 2>, DefaultClock>();
        let (rissue_tx, rissue_rx) = chan::<Issue<32>, DefaultClock>();
        let (rwbeat_tx, rwbeat_rx) = chan::<W<32, 4>, DefaultClock>();
        let (rrelease_tx, rrelease_rx) = chan::<Grant<2>, DefaultClock>();
        let (rgrant_tx, rgrant_rx) = chan::<Grant<2>, DefaultClock>();
        let (rdone_tx, rdone_rx) = chan::<Done<2>, DefaultClock>();
        let (rrdata_tx, rrdata_rx) = chan::<R<32, 2>, DefaultClock>();
        let (ridle_o, ridle_i) = signal::<Bit, DefaultClock>();
        let (ring_o, ring_i) = signal::<Bit, DefaultClock>();
        let (paw_bell_tx, paw_bell_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_bell_tx, par_bell_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_bell_tx, pw_bell_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_bell_tx, pb_bell_rx) = chan::<LiteB, DefaultClock>();
        let (pr_bell_tx, pr_bell_rx) = chan::<LiteR<32>, DefaultClock>();
        let (scat_o, scat_i) = signal::<U<32>, DefaultClock>();
        let (sccount_o, sccount_i) = signal::<U<16>, DefaultClock>();
        let (scstart_o, scstart_i) = signal::<Bit, DefaultClock>();
        let (scrun_o, scrun_i) = signal::<Bit, DefaultClock>();
        let (faw_tx, faw_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (far_tx, far_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (fw_tx, fw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (fb_tx, fb_rx) = chan::<B<2>, DefaultClock>();
        let (fr_tx, fr_rx) = chan::<R<32, 2>, DefaultClock>();
        let (sissue_tx, sissue_rx) = chan::<Issue<32>, DefaultClock>();
        let (swbeat_tx, swbeat_rx) = chan::<W<32, 4>, DefaultClock>();
        let (srelease_tx, srelease_rx) = chan::<Grant<2>, DefaultClock>();
        let (sgrant_tx, sgrant_rx) = chan::<Grant<2>, DefaultClock>();
        let (sdone_tx, sdone_rx) = chan::<Done<2>, DefaultClock>();
        let (srdata_tx, srdata_rx) = chan::<R<32, 2>, DefaultClock>();
        let (saw_tx, saw_rx) = chan::<Aw<32, 2>, DefaultClock>();
        let (sar_tx, sar_rx) = chan::<Ar<32, 2>, DefaultClock>();
        let (sw_tx, sw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (sb_tx, sb_rx) = chan::<B<2>, DefaultClock>();
        let (sr_tx, sr_rx) = chan::<R<32, 2>, DefaultClock>();
        // The interrupt controller speaks AXI-Lite too, behind a
        // bridge of its own.
        let (paw_tx, paw_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (par_tx, par_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (pw_tx, pw_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (pb_tx, pb_rx) = chan::<LiteB, DefaultClock>();
        let (pr_tx, pr_rx) = chan::<LiteR<32>, DefaultClock>();
        // And the debug module, on the seventh port.
        let (aw6_tx, aw6_rx) = chan::<Aw<32, 5>, DefaultClock>();
        let (ar6_tx, ar6_rx) = chan::<Ar<32, 5>, DefaultClock>();
        let (w6_tx, w6_rx) = chan::<W<32, 4>, DefaultClock>();
        let (b6_tx, b6_rx) = chan::<B<5>, DefaultClock>();
        let (r6_tx, r6_rx) = chan::<R<32, 5>, DefaultClock>();
        let (daw_tx, daw_rx) = chan::<LiteAw<32>, DefaultClock>();
        let (dar_tx, dar_rx) = chan::<LiteAr<32>, DefaultClock>();
        let (dw_tx, dw_rx) = chan::<LiteW<32, 4>, DefaultClock>();
        let (db_tx, db_rx) = chan::<LiteB, DefaultClock>();
        let (dr_tx, dr_rx) = chan::<LiteR<32>, DefaultClock>();
        // The timer first, since the core reads its line in the same
        // step, and the memory controller before the bridge inside its
        // own unit, for the same reason. The serial port before the
        // interrupt controller, and the controller before the core,
        // for the same reason again.
        //
        // The Ethernet port's units are split around the register block,
        // for the same reason. The ones that drive what the block reads
        // come before it: the receive side's slot number, which the block
        // turns into the address the store engine writes to, and its
        // length. The ones that read what the block drives come after it:
        // the store engine, which takes that address, and the sending
        // side, which takes the start. With the receive side after the
        // block, the block read last step's slot, so the store engine
        // wrote the first frame to slot zero while the block told the
        // core slot one; the board test caught it as a frame of the right
        // length whose every byte read back zero.
        join2(
            join2(
                join2(
                    join2(
                        self.share.run(
                            (net_rx, fromlink_rx, frometh_rx),
                            (net_tx, tolink_tx, toeth_tx),
                        ),
                        self.flen.run(toeth_rx, (lenbyte_tx, flen_len_o)),
                    ),
                    self.fin.run(
                        (
                            lenbyte_rx,
                            flen_len_i,
                            eth_rx_busy_hold,
                            eth_rx_full_i,
                        ),
                        (
                            sword_tx,
                            eth_rx_len_o,
                            store_go_o,
                            eth_rx_which_o,
                            eth_rx_drops_o,
                        ),
                    ),
                ),
                join2(
                    join2(
                        join2(
                            join2(
                                // The debug module before the core, whose
                                // requests the core reads in the same step.
                                join2(
                                    self.dmod.run(
                                        LitePort {
                                            aw: daw_rx,
                                            ar: dar_rx,
                                            w: dw_rx,
                                            b: db_tx,
                                            r: dr_tx,
                                        },
                                        (
                                            debug_i,
                                            dbg_rdata_i,
                                            haltreq_o,
                                            resumereq_o,
                                            dbg_regno_o,
                                            dbg_wdata_o,
                                            dbg_we_o,
                                        ),
                                    ),
                                    self.pdm.run(
                                        (
                                            aw6_rx,
                                            ar6_rx,
                                            w6_rx,
                                            [db_rx],
                                            [dr_rx],
                                        ),
                                        (
                                            [daw_tx],
                                            [dar_tx],
                                            [dw_tx],
                                            b6_tx,
                                            r6_tx,
                                        ),
                                    ),
                                ),
                                self.timer.run(
                                    PerPort {
                                        req: req1_rx,
                                        w: wd1_rx,
                                        ans: ans1_tx,
                                        r: rb1_tx,
                                    },
                                    (rst_timer, tirq_o, sirq_o, time_o),
                                ),
                            ),
                            join2(
                                join2(
                                    self.uart.run(
                                        LitePort {
                                            aw: law_rx,
                                            ar: lar_rx,
                                            w: lw_rx,
                                            b: lb_tx,
                                            r: lr_tx,
                                        },
                                        (rst_uart, rx, tx, uirq_o),
                                    ),
                                    join2(
                                        self.pwm.run(
                                            LitePort {
                                                aw: paw_pwm_rx,
                                                ar: par_pwm_rx,
                                                w: pw_pwm_rx,
                                                b: pb_pwm_tx,
                                                r: pr_pwm_tx,
                                            },
                                            pwm_pins,
                                        ),
                                        self.entropy.run(
                                            LitePort {
                                                aw: paw_trng_rx,
                                                ar: par_trng_rx,
                                                w: pw_trng_rx,
                                                b: pb_trng_tx,
                                                r: pr_trng_tx,
                                            },
                                            (),
                                        ),
                                    ),
                                ),
                                join2(
                                    self.plic.run(
                                        LitePort {
                                            aw: paw_rx,
                                            ar: par_rx,
                                            w: pw_rx,
                                            b: pb_tx,
                                            r: pr_tx,
                                        },
                                        (
                                            rst_plic,
                                            [uirq_i, irq, eth_irq_i],
                                            eirq_o,
                                            seirq_o,
                                        ),
                                    ),
                                    self.eth.run(
                                        LitePort {
                                            aw: paw_eth_rx,
                                            ar: par_eth_rx,
                                            w: pw_eth_rx,
                                            b: pb_eth_tx,
                                            r: pr_eth_tx,
                                        },
                                        (
                                            eth_tx_busy_i,
                                            eth_rx_busy_i,
                                            eth_rx_len_i,
                                            eth_rx_which_i,
                                            eth_rx_drops_i,
                                            mac_drops,
                                            eth_tx_base_o,
                                            eth_tx_bytes_o,
                                            eth_tx_start_o,
                                            eth_rx_base_o,
                                            eth_irq_o,
                                            eth_rx_full_o,
                                        ),
                                    ),
                                ),
                            ),
                        ),
                        join2(
                                // The tracker runs before the memory and
                                // the router: the request and the read
                                // beats it hands them cross in the
                                // cycle (issue 1291).
                                self.pdmem.run(
                                    (
                                        aw0_rx, ar0_rx, w0_rx, ans0_rx,
                                        rb0_rx,
                                    ),
                                    (req0_tx, wd0_tx, b0_tx, r0_tx),
                                ),
                            self.cpu.run(
                                (
                                    rst,
                                    eirq_i,
                                    tirq_i,
                                    sirq_i,
                                    rdata_rx,
                                    done_rx,
                                    grant_rx,
                                    haltreq_i,
                                    resumereq_i,
                                    dbg_regno_i,
                                    dbg_wdata_i,
                                    dbg_we_i,
                                    time_i,
                                    seirq_i,
                                ),
                                (
                                    halt,
                                    instr_o,
                                    retire_o,
                                    issue_tx,
                                    wbeat_tx,
                                    release_tx,
                                    debug_o,
                                    dbg_rdata_o,
                                ),
                            ),
                        ),
                    ),
                    join2(
                        join2(
                            join2(
                                join2(
                                    self.host.run(
                                        (
                                            issue_rx, wbeat_rx, b_rx, r_rx,
                                            release_rx,
                                        ),
                                        (
                                            aw_tx, ar_tx, w_tx, grant_tx,
                                            done_tx, rdata_tx,
                                        ),
                                    ),
                                    self.arb.run(
                                        (
                                            [
                                                aw_rx, jaw_rx, faw_rx, saw_rx,
                                                scaw_rx, sdaw_rx, raw_rx,
                                            ],
                                            [
                                                ar_rx, jar_rx, far_rx, sar_rx,
                                                scar_rx, sdar_rx, rar_rx,
                                            ],
                                            [
                                                w_rx, jw_rx, fw_rx, sw_rx,
                                                scw_rx, sdw_rx, rw_rx,
                                            ],
                                            xb_rx,
                                            xr_rx,
                                        ),
                                        (
                                            xaw_tx,
                                            xar_tx,
                                            xw_tx,
                                            [
                                                b_tx, jb_tx, fb_tx, sb_tx,
                                                scb_tx, sdb_tx, rb_tx,
                                            ],
                                            [
                                                r_tx, jr_tx, fr_tx, sr_tx,
                                                scr_tx, sdr_tx, rr_tx,
                                            ],
                                        ),
                                    ),
                                ),
                                join2(
                                join2(
                                    join2(
                                        self.jarb.run(
                                            (
                                                [kaw_rx, taw_rx],
                                                [kar_rx, tar_rx],
                                                [kw_rx, tw_rx],
                                                jb_rx,
                                                jr_rx,
                                            ),
                                            (
                                                jaw_tx,
                                                jar_tx,
                                                jw_tx,
                                                [kb_tx, tb_tx],
                                                [kr_tx, tr_tx],
                                            ),
                                        ),
                                        self.dhost.run(
                                            (
                                                tissue_rx, twbeat_rx, tb_rx,
                                                tr_rx, trelease_rx,
                                            ),
                                            (
                                                taw_tx, tar_tx, tw_tx,
                                                tgrant_tx, tdone_tx, trdata_tx,
                                            ),
                                        ),
                                    ),
                                    join2(
                                        join2(
                                            self.dtm.run(
                                                (
                                                    bscan_sel,
                                                    bscan_shift,
                                                    bscan_capture,
                                                    bscan_update,
                                                    bscan_tdi,
                                                    bscan_reset,
                                                    dansx_rx,
                                                ),
                                                (bscan_tdo, dreq_tx),
                                            ),
                                            self.dbridge.run(
                                                (
                                                    dreqx_rx, tgrant_rx,
                                                    tdone_rx, trdata_rx,
                                                ),
                                                (
                                                    dans_tx, tissue_tx,
                                                    twbeat_tx, trelease_tx,
                                                ),
                                            ),
                                        ),
                                        join2(
                                            self.dreq.run(dreq_rx, dreqx_tx),
                                            self.dans.run(dans_rx, dansx_tx),
                                        ),
                                    ),
                                ),
                                self.jtag.run(
                                    AxiPinsIn {
                                        pins: jtag,
                                        b: kb_rx,
                                        r: kr_rx,
                                    },
                                    AxiPinsOut {
                                        aw: kaw_tx,
                                        ar: kar_tx,
                                        w: kw_tx,
                                        awready: jtag_awready,
                                        wready: jtag_wready,
                                        bid: jtag_bid,
                                        bresp: jtag_bresp,
                                        bvalid: jtag_bvalid,
                                        arready: jtag_arready,
                                        rid: jtag_rid,
                                        rdata: jtag_rdata,
                                        rresp: jtag_rresp,
                                        rlast: jtag_rlast,
                                        rvalid: jtag_rvalid,
                                    },
                                ),
                                ),
                            ),
                            self.router.run(
                                (
                                    xaw_rx,
                                    xar_rx,
                                    xw_rx,
                                    [
                                        b0_rx, b1_rx, b2_rx, b3_rx, b4_rx,
                                        b5_rx, b6_rx, b7_rx,
                                    ],
                                    [
                                        r0_rx, r1_rx, r2_rx, r3_rx, r4_rx,
                                        r5_rx, r6_rx, r7_rx,
                                    ],
                                ),
                                (
                                    [
                                        aw0_tx, aw1_tx, aw2_tx, aw3_tx, aw4_tx,
                                        aw5_tx, aw6_tx, aw7_tx,
                                    ],
                                    [
                                        ar0_tx, ar1_tx, ar2_tx, ar3_tx, ar4_tx,
                                        ar5_tx, ar6_tx, ar7_tx,
                                    ],
                                    [
                                        w0_tx, w1_tx, w2_tx, w3_tx, w4_tx,
                                        w5_tx, w6_tx, w7_tx,
                                    ],
                                    xb_tx,
                                    xr_tx,
                                ),
                            ),
                        ),
                        join2(
                            join2(
                                join2(
                                    self.dmem.run(
                                        PerPort {
                                            req: req0_rx,
                                            w: wd0_rx,
                                            ans: ans0_tx,
                                            r: rb0_tx,
                                        },
                                        (),
                                    ),
                                    self.ptimer.run(
                                        (
                                            aw1_rx, ar1_rx, w1_rx, ans1_rx,
                                            rb1_rx,
                                        ),
                                        (req1_tx, wd1_tx, b1_tx, r1_tx),
                                    ),
                                ),
                                join2(
                                    self.puart.run(
                                        (
                                            aw2_rx,
                                            ar2_rx,
                                            w2_rx,
                                            [
                                                lb_rx, pb_pwm_rx, vb,
                                                pb_rem_rx, pb_eth_rx,
                                                pb_trng_rx, pb_spi_rx, pb_mdio_rx,
                                                pb_sd_rx, pb_bell_rx,
                                            ],
                                            [
                                                lr_rx, pr_pwm_rx, vr,
                                                pr_rem_rx, pr_eth_rx,
                                                pr_trng_rx, pr_spi_rx, pr_mdio_rx,
                                                pr_sd_rx, pr_bell_rx,
                                            ],
                                        ),
                                        (
                                            [
                                                law_tx,
                                                paw_pwm_tx,
                                                vaw,
                                                paw_rem_tx,
                                                paw_eth_tx,
                                                paw_trng_tx,
                                                paw_spi_tx,
                                                paw_mdio_tx,
                                                paw_sd_tx,
                                                paw_bell_tx,
                                            ],
                                            [
                                                lar_tx,
                                                par_pwm_tx,
                                                var,
                                                par_rem_tx,
                                                par_eth_tx,
                                                par_trng_tx,
                                                par_spi_tx,
                                                par_mdio_tx,
                                                par_sd_tx,
                                                par_bell_tx,
                                            ],
                                            [
                                                lw_tx, pw_pwm_tx, vw,
                                                pw_rem_tx, pw_eth_tx,
                                                pw_trng_tx, pw_spi_tx, pw_mdio_tx,
                                                pw_sd_tx, pw_bell_tx,
                                            ],
                                            b2_tx,
                                            r2_tx,
                                        ),
                                    ),
                                    join2(
                                        join2(
                                            self.pplic.run(
                                                (
                                                    aw4_rx,
                                                    ar4_rx,
                                                    w4_rx,
                                                    [pb_rx],
                                                    [pr_rx],
                                                ),
                                                (
                                                    [paw_tx],
                                                    [par_tx],
                                                    [pw_tx],
                                                    b4_tx,
                                                    r4_tx,
                                                ),
                                            ),
join2(
                                            join2(
                                                self.prom.run(
                                                    (
                                                        aw5_rx, ar5_rx,
                                                        w5_rx, ans5_rx,
                                                        rb5_rx,
                                                    ),
                                                    (
                                                        req5_tx, wd5_tx,
                                                        b5_tx, r5_tx,
                                                    ),
                                                ),
                                                self.rom.run(
                                                    PerPort {
                                                        req: req5_rx,
                                                        w: wd5_rx,
                                                        ans: ans5_tx,
                                                        r: rb5_tx,
                                                    },
                                                    (),
                                                ),
                                            ),
                                            // The flash: the master and the window
                                            // before the pins, which read the lines
                                            // both drive this step (issue 312).
                                            join2(
                                                join2(
                                                    join2(
                                                    self.spi.run(
                                                        LitePort {
                                                            aw: paw_spi_rx,
                                                            ar: par_spi_rx,
                                                            w: pw_spi_rx,
                                                            b: pb_spi_tx,
                                                            r: pr_spi_tx,
                                                        },
                                                        SpiLines {
                                                            miso: fl_miso,
                                                            sclk: m_sclk_o,
                                                            mosi: m_mosi_o,
                                                            cs_n: m_cs_n_o,
                                                            irq: spi_irq_o,
                                                        },
                                                    ),
                                                    join2(
                                                    self.mdio.run(
                                                        LitePort {
                                                            aw: paw_mdio_rx,
                                                            ar: par_mdio_rx,
                                                            w: pw_mdio_rx,
                                                            b: pb_mdio_tx,
                                                            r: pr_mdio_tx,
                                                        },
                                                        MdioLines {
                                                            mdio_in: phy_mdio_in,
                                                            mdc: phy_mdc,
                                                            mdio_out: phy_mdio_out,
                                                            mdio_oe: phy_mdio_oe,
                                                        },
                                                    ),
                                                    self.sd.run(
                                                        LitePort {
                                                            aw: paw_sd_rx,
                                                            ar: par_sd_rx,
                                                            w: pw_sd_rx,
                                                            b: pb_sd_tx,
                                                            r: pr_sd_tx,
                                                        },
                                                        SdLines {
                                                            cmd_in: sd_cmd_in,
                                                            dat_in: sd_dat_in,
                                                            sclk: sd_clk,
                                                            cmd_out: sd_cmd_out,
                                                            cmd_oe: sd_cmd_oe,
                                                            dat_out: sd_dat_out,
                                                            dat_oe: sd_dat_oe,
                                                            irq: sd_irq_o,
                                                            dma_in: sdin_rx,
                                                            dma_out: sdout_tx,
                                                            dma_at: sdat_o,
                                                            dma_bytes: sdbytes_o,
                                                            dma_words: sdwords_o,
                                                            store_go: sdsgo_o,
                                                            fetch_go: sdfgo_o,
                                                            store_busy: sdsbusy_i,
                                                            fetch_busy: sdfbusy_i,
                                                        },
                                                    ),
                                                    ),
                                                    ),
                                                    join2(
                                                        self.pflash.run(
                                                            (aw7_rx, ar7_rx, w7_rx, ans7_rx, rb7_rx),
                                                            (req7_tx, wd7_tx, b7_tx, r7_tx),
                                                        ),
                                                        self.flashwin.run(
                                                            PerPort {
                                                                req: req7_rx,
                                                                w: wd7_rx,
                                                                ans: ans7_tx,
                                                                r: rb7_tx,
                                                            },
                                                            (w_rst_i, fl_miso_win, w_sclk_o, w_mosi_o, w_cs_n_o),
                                                        ),
                                                    ),
                                                ),
                                                self.cfgflash.run(
                                                    (m_sclk_i, m_mosi_i, m_cs_n_i, w_sclk_i, w_mosi_i, w_cs_n_i),
                                                    (fl_cclk, fl_mosi, fl_cs_n, w_rst_o, fl_refused),
                                                ),
                                            ),
),
                                        ),
                                        // The peripheral before the link, so
                                        // a transaction and the first byte of
                                        // its frame are one step apart rather
                                        // than two.
                                        join2(
                                            self.remote.run(
                                                LitePort {
                                                    aw: paw_rem_rx,
                                                    ar: par_rem_rx,
                                                    w: pw_rem_rx,
                                                    b: pb_rem_tx,
                                                    r: pr_rem_tx,
                                                },
                                                (ans_rx, ask_tx),
                                            ),
                                            self.link.run(
                                                (ask_rx, tolink_rx),
                                                (ans_tx, fromlink_tx),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                            self.ddr3.run(
                                (aw3_rx, ar3_rx, w3_rx, b3_tx, r3_tx),
                                (
                                    sys_clk, sys_rst, calib, ui_clk, ui_rst,
                                    ck_p, ck_n, mem_rst_n, cke, cs_n, ras_n,
                                    cas_n, we_n, row, bank, dm, odt, dq, dqs,
                                    dqs_n,
                                ),
                            ),
                        ),
                    ),
                ),
            ),
            join2(
                self.store.run(
                    (
                        sgrant_rx,
                        sdone_rx,
                        sword_rx,
                        eth_rx_base_i,
                        eth_rx_len_store,
                        store_go_i,
                    ),
                    (sissue_tx, swbeat_tx, srelease_tx, eth_rx_busy_o),
                ),
                join2(
                    join2(
                        self.fout.run(
                            (fword_rx, eth_tx_bytes_i, eth_tx_start_i),
                            (frometh_tx, eth_tx_busy_o, fnwords_o),
                        ),
                        self.fetch.run(
                            (
                                fgrant_rx,
                                fdone_rx,
                                frdata_rx,
                                eth_tx_base_i,
                                fnwords_i,
                                eth_tx_start_fetch,
                            ),
                            (fissue_tx, frelease_tx, fword_tx, fetch_run_o),
                        ),
                    ),
                    join2(
                        join2(
                            self.fhost.run(
                                (
                                    fissue_rx,
                                    fwbeat_rx,
                                    fb_rx,
                                    fr_rx,
                                    frelease_rx,
                                ),
                                (
                                    faw_tx, far_tx, fw_tx, fgrant_tx,
                                    fdone_tx, frdata_tx,
                                ),
                            ),
                            self.shost.run(
                                (
                                    sissue_rx,
                                    swbeat_rx,
                                    sb_rx,
                                    sr_rx,
                                    srelease_rx,
                                ),
                                (
                                    saw_tx, sar_tx, sw_tx, sgrant_tx, sdone_tx,
                                    srdata_tx,
                                ),
                            ),
                        ),
                        join2(
                            join2(
                                self.fnobeats.run((), fwbeat_tx),
                                self.snoreads.run(srdata_rx, ()),
                            ),
                            // The scanout: the fetch before `ScanFetch`,
                            // which reads whether it is running.
                            join2(
                                join2(
                                    self.vfetch.run(
                                        (
                                            scgrant_rx, scdone_rx, scrdata_rx,
                                            scat_i, sccount_i, scstart_i,
                                        ),
                                        (
                                            scissue_tx, screlease_tx,
                                            scan_words, scrun_o,
                                        ),
                                    ),
                                    self.scan.run(
                                        (scan_req, scrun_i),
                                        (scat_o, sccount_o, scstart_o),
                                    ),
                                ),
                                join2(
                                    self.vhost.run(
                                        (
                                            scissue_rx, scwbeat_rx,
                                            scb_rx, scr_rx, screlease_rx,
                                        ),
                                        (
                                            scaw_tx, scar_tx, scw_tx,
                                            scgrant_tx, scdone_tx,
                                            scrdata_tx,
                                        ),
                                    ),
                                    join2(
                                        join2(
                                            self.vnobeats.run((), scwbeat_tx),
                                            // Razboj: the rasteriser before
                                            // the doorbell, which reads
                                            // whether it is idle.
                                            join2(
                                                join2(
                                                    self.raster.run(
                                                        (rgrant_rx, rdone_rx, rrdata_rx, ring_i),
                                                        (rissue_tx, rwbeat_tx, rrelease_tx, ridle_o),
                                                    ),
                                                    self.doorbell.run(
                                                        LitePort {
                                                            aw: paw_bell_rx,
                                                            ar: par_bell_rx,
                                                            w: pw_bell_rx,
                                                            b: pb_bell_tx,
                                                            r: pr_bell_tx,
                                                        },
                                                        (rst_bell, ridle_i, ring_o),
                                                    ),
                                                ),
                                                self.rhost.run(
                                                    (
                                                        rissue_rx, rwbeat_rx, rb_rx, rr_rx,
                                                        rrelease_rx,
                                                    ),
                                                    (
                                                        raw_tx, rar_tx, rw_tx, rgrant_tx,
                                                        rdone_tx, rrdata_tx,
                                                    ),
                                                ),
                                            ),
                                        ),
                                        // The SD host's engines, after the host, whose starts and
                                        // lengths they read.
                                        join2(
                                            join2(
                                                join2(
                                                    self.sdstore.run(
                                                        (
                                                            dsgrant_rx, dsdone_rx, sdout_rx, sdat_i,
                                                            sdbytes_i, sdsgo_i,
                                                        ),
                                                        (dsissue_tx, dswbeat_tx, dsrelease_tx, sdsbusy_o),
                                                    ),
                                                    self.sdfetch.run(
                                                        (
                                                            dfgrant_rx, dfdone_rx, dfrdata_rx, sdat_f,
                                                            sdwords_i, sdfgo_i,
                                                        ),
                                                        (dfissue_tx, dfrelease_tx, sdin_tx, sdfbusy_o),
                                                    ),
                                                ),
                                                join2(
                                                    self.sdshost.run(
                                                        (
                                                            dsissue_rx, dswbeat_rx, dsb_rx, dsr_rx,
                                                            dsrelease_rx,
                                                        ),
                                                        (
                                                            dsaw_tx, dsar_tx, dsw_tx, dsgrant_tx,
                                                            dsdone_tx, dsrdata_tx,
                                                        ),
                                                    ),
                                                    self.sdfhost.run(
                                                        (
                                                            dfissue_rx, dfwbeat_rx, dfb_rx, dfr_rx,
                                                            dfrelease_rx,
                                                        ),
                                                        (
                                                            dfaw_tx, dfar_tx, dfw_tx, dfgrant_tx,
                                                            dfdone_tx, dfrdata_tx,
                                                        ),
                                                    ),
                                                ),
                                            ),
                                            join2(
                                                join2(
                                                    self.sdnoreads.run(dsrdata_rx, ()),
                                                    self.sdnobeats.run((), dfwbeat_tx),
                                                ),
                                                self.sdarb.run(
                                                    (
                                                        [dsaw_rx, dfaw_rx],
                                                        [dsar_rx, dfar_rx],
                                                        [dsw_rx, dfw_rx],
                                                        sdb_rx,
                                                        sdr_rx,
                                                    ),
                                                    (
                                                        sdaw_tx,
                                                        sdar_tx,
                                                        sdw_tx,
                                                        [dsb_tx, dfb_tx],
                                                        [dsr_tx, dfr_tx],
                                                    ),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        )
        .await;
    }
}
