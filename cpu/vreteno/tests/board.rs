// SPDX-License-Identifier: Apache-2.0
//! The board's design, as one lowered unit, run on the programs
//! compiled for the core: the greeting, which uses the data memory and
//! the serial port; the DDR3 test, which uses the memory's region
//! through the bridge and the controller's model; and input by
//! interrupt, which takes the serial port's receive interrupt through
//! the interrupt controller; and the remote peripheral, which the core
//! reaches as frames on the Ethernet port with a program in this file
//! answering them. And its netlist, which holds the memory controller
//! as a foreign module.
use std::collections::HashMap;
use txhdl::comp::{
    chan, pad, signal, DefaultClock, In, Out, Running, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};
use txhdl_parts::eth::EthByte;
use txhdl_parts::remote::eth::{FRAME_LEN, KIND_ANSWER, KIND_ASK};
use vreteno32::board::{Board, BoardIn, BoardOut, REMOTE_DEV};
use vreteno32::core::Vreteno;
use vreteno32::dmem::Dmem;
use vreteno32::rom::Rom;
use vreteno32::term::Terminal;

/// The serial port's divider in these runs: four cycles a bit, as the
/// demonstration has it.
type TestBoard = Board<4>;

/// What a run came to: what the serial line said, and the cycle the
/// core halted on.
struct Ran {
    said: String,
    /// How many bytes of the stream the terminal managed to type, the
    /// cycle the last one went out on, and how long the run was.
    typed: usize,
    typed_at: u64,
    ran_for: u64,
    halted_at: Option<u64>,
    /// Every frame that left the Ethernet port, whether or not a
    /// program was there to answer it.
    sent: Vec<Vec<u8>>,
    /// What the debugger on the JTAG cable read, in order, and how
    /// many steps of its plan it got through.
    got: Vec<u32>,
    steps: usize,
}

/// Run `text` with `data` in the data memory, on the board's design,
/// for at most `limit` cycles, with a terminal on the serial line that
/// types `reply` once the core has said a line.
fn run(text: &[u32], data: &[u8], reply: &[u8], limit: u64) -> Ran {
    run_paced(text, data, reply, &[], limit)
}

/// The same, with a program on the other side of the Ethernet port
/// answering the remote peripheral's frames.
fn run_served(text: &[u32], data: &[u8], limit: u64) -> Ran {
    let net = Net {
        serve: true,
        ..Default::default()
    };
    run_all(text, data, b"", &[], limit, net, &[])
}

/// The same, with a debugger on the JTAG cable following `plan`.
fn run_debugged(text: &[u32], data: &[u8], limit: u64, plan: &[Op]) -> Ran {
    run_all(text, data, b"", &[], limit, Net::default(), plan)
}

/// The program at the other end of the wire, for one cycle: it reads
/// the frames the peripheral sends, keeps what it is told to keep,
/// refuses a read of an address nothing has written, and answers with
/// the frame it was sent, four bytes of it changed. `//tools/remote`
/// does the same thing in Go on another machine; this is the same
/// protocol with the wire left out.
fn device(
    from: &Rx<EthByte>,
    to: &Tx<EthByte>,
    frame: &mut Vec<u8>,
    reply: &mut Vec<EthByte>,
    words: &mut HashMap<u32, u32>,
    sent: &mut Vec<Vec<u8>>,
    serve: bool,
) {
    if !reply.is_empty() && to.ready().to_bool() {
        to.send(reply.remove(0));
    }
    let Some(byte) = from.recv() else {
        return;
    };
    frame.push(byte.data.raw() as u8);
    if !byte.last.to_bool() {
        return;
    }
    sent.push(frame.clone());
    if serve && frame.len() >= FRAME_LEN as usize {
        let at = u32::from_be_bytes(frame[18..22].try_into().unwrap());
        let data = u32::from_be_bytes(frame[22..26].try_into().unwrap());
        let (answer, err) = if frame[17] & 1 == 1 {
            words.insert(at, data);
            (0, false)
        } else {
            match words.get(&at) {
                Some(v) => (*v, false),
                None => (0, true),
            }
        };
        let mut out = frame.clone();
        out.truncate(FRAME_LEN as usize);
        out[14] = KIND_ANSWER as u8;
        out[17] = err as u8;
        out[22..26].copy_from_slice(&answer.to_be_bytes());
        let n = out.len();
        for (i, b) in out.into_iter().enumerate() {
            reply.push(EthByte {
                data: U::from(b),
                last: Bit::from_bool(i + 1 == n),
            });
        }
    }
    frame.clear();
}

/// What a debugger on the JTAG cable does during a run, one step at a
/// time, as single-beat transactions on the master's pins: a write of a
/// word, a read of one, whose answer is kept, or a wait. The master
/// model in the run drives the pins as the JTAG-to-AXI core does, one
/// transaction at a time (issue 154).
#[derive(Clone, Copy, Debug)]
enum Op {
    Write(u32, u32),
    Read(u32),
    Wait(u64),
}

/// The pins the model drives and reads, kept out of the board's port
/// struct so the run can move them.
struct Jtag {
    awaddr: Out<U<32>>,
    awvalid: Out<Bit>,
    wdata: Out<U<32>>,
    wvalid: Out<Bit>,
    bready: Out<Bit>,
    araddr: Out<U<32>>,
    arvalid: Out<Bit>,
    rready: Out<Bit>,
    awready: In<Bit>,
    wready: In<Bit>,
    bvalid: In<Bit>,
    arready: In<Bit>,
    rdata: In<U<32>>,
    rvalid: In<Bit>,
}

/// The master model's state between cycles: which op, and which of its
/// handshakes are done.
#[derive(Default)]
struct Master {
    at: usize,
    aw_done: bool,
    w_done: bool,
    ar_done: bool,
    /// What was driven valid this cycle, since a ready seen without a
    /// valid is not a handshake.
    awv: bool,
    wv: bool,
    arv: bool,
    waited: u64,
    got: Vec<u32>,
}

impl Master {
    /// Before a cycle: drive the pins for the op in hand.
    fn drive(&mut self, plan: &[Op], j: &Jtag) {
        let (mut awv, mut wv, mut arv) = (false, false, false);
        if let Some(op) = plan.get(self.at) {
            match *op {
                Op::Write(addr, data) => {
                    j.awaddr.set(U::from(addr));
                    j.wdata.set(U::from(data));
                    // The data beat after the address has gone, as
                    // the JTAG-to-AXI master sends them.
                    awv = !self.aw_done;
                    wv = self.aw_done && !self.w_done;
                }
                Op::Read(addr) => {
                    j.araddr.set(U::from(addr));
                    arv = !self.ar_done;
                }
                Op::Wait(_) => {}
            }
        }
        j.awvalid.set(Bit::from_bool(awv));
        j.wvalid.set(Bit::from_bool(wv));
        j.arvalid.set(Bit::from_bool(arv));
        self.awv = awv;
        self.wv = wv;
        self.arv = arv;
        j.bready.set(Bit::One);
        j.rready.set(Bit::One);
    }

    /// After the cycle: what the pins say happened at its edge. A
    /// valid the model held meets a ready the pins computed in the
    /// same step, so the beat went; a response present in the step was
    /// taken, since ready is always high on the model's side.
    fn observe(&mut self, plan: &[Op], j: &Jtag) {
        let Some(op) = plan.get(self.at) else {
            return;
        };
        match *op {
            Op::Write(..) => {
                if self.awv && j.awready.get().to_bool() {
                    self.aw_done = true;
                }
                if self.wv && j.wready.get().to_bool() {
                    self.w_done = true;
                }
                if self.aw_done && self.w_done && j.bvalid.get().to_bool() {
                    self.next();
                }
            }
            Op::Read(_) => {
                if self.arv && j.arready.get().to_bool() {
                    self.ar_done = true;
                }
                if self.ar_done && j.rvalid.get().to_bool() {
                    self.got.push(j.rdata.get().raw() as u32);
                    self.next();
                }
            }
            Op::Wait(n) => {
                self.waited += 1;
                if self.waited >= n {
                    self.next();
                }
            }
        }
    }

    fn next(&mut self) {
        self.at += 1;
        self.aw_done = false;
        self.w_done = false;
        self.ar_done = false;
        self.waited = 0;
    }
}

/// The same, with the terminal typing in blocks of `block` bytes and
/// waiting for a byte back between them, which is how a sender talks
/// to the loader.
fn run_paced(
    text: &[u32],
    data: &[u8],
    reply: &[u8],
    blocks: &[usize],
    limit: u64,
) -> Ran {
    run_all(text, data, reply, blocks, limit, Net::default(), &[])
}

/// The run itself. `serve` says whether a program answers the frames
/// the remote peripheral sends; without one its port is a wire with
/// nothing at the other end, which is what every other run here wants.
/// What is at the other end of the Ethernet port during a run.
#[derive(Default)]
struct Net<'a> {
    /// A program answering the remote peripheral's frames.
    serve: bool,
    /// Frames put on the wire from the start, unasked.
    inject: &'a [Vec<u8>],
}

fn run_all(
    text: &[u32],
    data: &[u8],
    reply: &[u8],
    blocks: &[usize],
    limit: u64,
    net: Net,
    plan: &[Op],
) -> Ran {
    let Net { serve, inject } = net;
    let mut board = TestBoard {
        cpu: Vreteno::with(text),
        rom: Rom::with(text),
        dmem: Dmem::with(data),
        ..Default::default()
    };
    let (rst_o, rst) = signal::<Bit, DefaultClock>();
    let (irq_o, irq) = signal::<Bit, DefaultClock>();
    let (rx_o, rx) = signal::<Bit, DefaultClock>();
    let quiet = || signal::<Bit, DefaultClock>().1;
    let bit = || signal::<Bit, DefaultClock>().0;
    let (halt_o, halt) = signal::<Bit, DefaultClock>();
    let (tx_o, tx) = signal::<Bit, DefaultClock>();
    // The Ethernet port, as the board sees it: the frames the remote
    // peripheral sends and the frames that answer them. There is no
    // MAC in these runs, so a byte goes out a cycle and the program
    // below reads them as they arrive.
    let (net_out_tx, net_out_rx) = chan::<EthByte, DefaultClock>();
    let (net_in_tx, net_in_rx) = chan::<EthByte, DefaultClock>();
    // The third slot of the page at `0x3000` is tied off: nothing in
    // these runs writes to `0x3200`, and a run that did would wait on
    // an answer that never comes, which is what the board top's own
    // tie-off does as well.
    // The JTAG master's pins, with no master on them: every valid low
    // and nothing else read.
    let lo = || signal::<Bit, DefaultClock>().1;
    let (awaddr_o, awaddr) = signal::<U<32>, DefaultClock>();
    let (awvalid_o, awvalid) = signal::<Bit, DefaultClock>();
    let (wdata_o, wdata) = signal::<U<32>, DefaultClock>();
    let (wvalid_o, wvalid) = signal::<Bit, DefaultClock>();
    let (bready_o, bready) = signal::<Bit, DefaultClock>();
    let (araddr_o, araddr) = signal::<U<32>, DefaultClock>();
    let (arvalid_o, arvalid) = signal::<Bit, DefaultClock>();
    let (rready_o, rready) = signal::<Bit, DefaultClock>();
    let (awready_o, awready) = signal::<Bit, DefaultClock>();
    let (wready_o, wready) = signal::<Bit, DefaultClock>();
    let (bvalid_o, bvalid) = signal::<Bit, DefaultClock>();
    let (arready_o, arready) = signal::<Bit, DefaultClock>();
    let (rdata_o, rdata) = signal::<U<32>, DefaultClock>();
    let (rvalid_o, rvalid) = signal::<Bit, DefaultClock>();
    let jtag = Jtag {
        awaddr: awaddr_o,
        awvalid: awvalid_o,
        wdata: wdata_o,
        wvalid: wvalid_o,
        bready: bready_o,
        araddr: araddr_o,
        arvalid: arvalid_o,
        rready: rready_o,
        awready,
        wready,
        bvalid,
        arready,
        rdata,
        rvalid,
    };
    // A single beat, a word wide, incrementing: what the master sends.
    let (awlen_o, awlen) = signal::<U<8>, DefaultClock>();
    let (awsize_o, awsize) = signal::<U<3>, DefaultClock>();
    let (awburst_o, awburst) = signal::<U<2>, DefaultClock>();
    let (wstrb_o, wstrb) = signal::<U<4>, DefaultClock>();
    let (wlast_o, wlast) = signal::<Bit, DefaultClock>();
    let (arlen_o, arlen) = signal::<U<8>, DefaultClock>();
    let (arsize_o, arsize) = signal::<U<3>, DefaultClock>();
    let (arburst_o, arburst) = signal::<U<2>, DefaultClock>();
    awlen_o.set(U::from(0u8));
    awsize_o.set(U::from(2u8));
    awburst_o.set(U::from(1u8));
    wstrb_o.set(U::from(0xfu8));
    wlast_o.set(Bit::One);
    arlen_o.set(U::from(0u8));
    arsize_o.set(U::from(2u8));
    arburst_o.set(U::from(1u8));
    let mut sim = Running::new(board.run(
        BoardIn {
            rst,
            irq,
            rx,
            sys_clk: quiet(),
            sys_rst: quiet(),
            vb: chan::<LiteB, DefaultClock>().1,
            vr: chan::<LiteR<32>, DefaultClock>().1,
            net_rx: net_in_rx,
            jtag_awid: signal::<U<2>, DefaultClock>().1,
            jtag_awaddr: awaddr,
            jtag_awlen: awlen,
            jtag_awsize: awsize,
            jtag_awburst: awburst,
            jtag_awlock: lo(),
            jtag_awcache: signal::<U<4>, DefaultClock>().1,
            jtag_awprot: signal::<U<3>, DefaultClock>().1,
            jtag_awvalid: awvalid,
            jtag_wdata: wdata,
            jtag_wstrb: wstrb,
            jtag_wlast: wlast,
            jtag_wvalid: wvalid,
            jtag_bready: bready,
            jtag_arid: signal::<U<2>, DefaultClock>().1,
            jtag_araddr: araddr,
            jtag_arlen: arlen,
            jtag_arsize: arsize,
            jtag_arburst: arburst,
            jtag_arlock: lo(),
            jtag_arcache: signal::<U<4>, DefaultClock>().1,
            jtag_arprot: signal::<U<3>, DefaultClock>().1,
            jtag_arvalid: arvalid,
            jtag_rready: rready,
        },
        BoardOut {
            halt: halt_o,
            tx: tx_o,
            pwm_pins: signal::<U<4>, DefaultClock>().0,
            calib: bit(),
            ui_clk: bit(),
            ui_rst: bit(),
            ck_p: bit(),
            ck_n: bit(),
            mem_rst_n: bit(),
            cke: bit(),
            cs_n: bit(),
            ras_n: bit(),
            cas_n: bit(),
            we_n: bit(),
            row: signal::<U<15>, DefaultClock>().0,
            bank: signal::<U<3>, DefaultClock>().0,
            dm: signal::<U<4>, DefaultClock>().0,
            odt: bit(),
            dq: pad::<U<32>, DefaultClock>(),
            dqs: pad::<U<4>, DefaultClock>(),
            dqs_n: pad::<U<4>, DefaultClock>(),
            vaw: chan::<LiteAw<32>, DefaultClock>().0,
            var: chan::<LiteAr<32>, DefaultClock>().0,
            vw: chan::<LiteW<32, 4>, DefaultClock>().0,
            net_tx: net_out_tx,
            jtag_awready: awready_o,
            jtag_wready: wready_o,
            jtag_bid: signal::<U<2>, DefaultClock>().0,
            jtag_bresp: signal::<U<2>, DefaultClock>().0,
            jtag_bvalid: bvalid_o,
            jtag_arready: arready_o,
            jtag_rid: signal::<U<2>, DefaultClock>().0,
            jtag_rdata: rdata_o,
            jtag_rresp: signal::<U<2>, DefaultClock>().0,
            jtag_rlast: bit(),
            jtag_rvalid: rvalid_o,
        },
    ));
    rst_o.set(Bit::One);
    sim.cycle();
    rst_o.set(Bit::Zero);
    irq_o.set(Bit::Zero);
    rx_o.set(Bit::One);
    let mut term = if blocks.is_empty() {
        Terminal::new(reply)
    } else {
        Terminal::paced(reply, blocks)
    };
    let mut halted_at = None;
    let mut typed_was = 0;
    let mut typed_at = 0;
    let mut ran_for = 0;
    // The program at the other end of the Ethernet port, and what it
    // has been told to remember.
    let mut frame: Vec<u8> = Vec::new();
    let mut reply: Vec<EthByte> = Vec::new();
    // Frames put on the wire from the start, before anything is asked
    // of the program at the other end. They leave a byte a cycle, as
    // the answers do.
    for f in inject {
        for (i, b) in f.iter().enumerate() {
            reply.push(EthByte {
                data: U::from(*b),
                last: Bit::from_bool(i + 1 == f.len()),
            });
        }
    }
    let mut words: HashMap<u32, u32> = HashMap::new();
    let mut sent: Vec<Vec<u8>> = Vec::new();
    let mut master = Master::default();
    for cycle in 0..limit {
        master.drive(plan, &jtag);
        device(
            &net_out_rx,
            &net_in_tx,
            &mut frame,
            &mut reply,
            &mut words,
            &mut sent,
            serve,
        );
        sim.cycle();
        master.observe(plan, &jtag);
        term.see(tx.get().to_bool());
        rx_o.set(Bit::from_bool(term.level()));
        ran_for = cycle;
        if term.typed() != typed_was {
            typed_was = term.typed();
            typed_at = cycle;
        }
        if halt.get().to_bool() {
            halted_at = Some(cycle);
            break;
        }
    }
    // The last byte is still going out when the core halts.
    for _ in 0..64 {
        sim.cycle();
        term.see(tx.get().to_bool());
    }
    Ran {
        said: term.said.clone(),
        typed: term.typed(),
        typed_at,
        ran_for,
        halted_at,
        sent,
        got: master.got,
        steps: master.at,
    }
}

#[test]
fn the_greeting_runs_on_the_board() {
    let ran = run(hello_program::TEXT, hello_program::DATA, b"", 8000);
    assert_eq!(ran.said, "hello from rust\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

#[test]
fn the_memory_test_runs_on_the_board() {
    let ran = run(ddr3_program::TEXT, ddr3_program::DATA, b"", 40000);
    assert_eq!(ran.said, "ddr3 ok\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

/// A frame arrives on the wire, and the Ethernet port's engines store
/// it in DDR3 without the core touching a byte of it. The core then
/// reads it back out of memory as the Zephyr driver will, and says what
/// it read (issue 151).
///
/// The bytes the core says are the only evidence this test takes, and
/// they come out of the real memory through the real bus, so a pass
/// covers the whole receive path: the split by EtherType, the length
/// found again after the crossing, the packing into words, the store
/// engine's bursts through the arbiter, the router and the bridge, and
/// the register block's arrival. That is what the engines' own
/// examples could not show, since each ran against a memory it defined
/// itself.
#[test]
fn a_frame_received_lands_in_memory_and_the_core_reads_it_back() {
    // Not the remote peripheral's type, so the sharing unit sends it to
    // the Ethernet port rather than to the remote peripheral. Twenty one
    // bytes, so the last word holds one real byte and the store engine
    // strobes away the other three, which is a write the memory must
    // honour lane by lane.
    let mut frame: Vec<u8> = (0..21u8).map(|i| 0x40 + i).collect();
    frame[12] = 0x08;
    frame[13] = 0x00;
    let bytes: String = frame.iter().map(|b| format!("{b:02x}")).collect();
    let want = format!("rx {:04x} {bytes}\n", frame.len());
    let frames = [frame];
    let net = Net {
        inject: &frames,
        ..Default::default()
    };
    let ran = run_all(
        ethrx_program::TEXT,
        ethrx_program::DATA,
        b"",
        &[],
        40000,
        net,
        &[],
    );
    assert_eq!(ran.said, want, "what the core read back out of DDR3");
    assert!(ran.halted_at.is_some(), "the core acknowledged and halted");
}

/// The core writes a frame into a transmit slot in DDR3 and asks the
/// Ethernet port to send it; the port's engines fetch it back out of
/// memory and put it on the wire without the core (issue 151).
///
/// What left the port is the only evidence taken, and it is compared
/// byte for byte with what the core wrote, so a pass covers the whole
/// sending path: the fetch engine's bursts through the arbiter, the
/// router and the bridge, the word count `FrameOut` works out, its bytes
/// with the frame's last one marked, and the sharing unit's merge onto
/// the wire.
///
/// Twenty three bytes, so the last word holds three real bytes over one
/// that is not the frame's, and a byte side that sent the whole word
/// would put a twenty fourth byte on the wire that this would see.
#[test]
fn a_frame_written_to_memory_leaves_the_port_as_written() {
    let len = 23u32;
    let frame: Vec<u8> = (0..len)
        .map(|i| match i {
            12 => 0x08,
            13 => 0x00,
            _ => (0x60 + i) as u8,
        })
        .collect();
    let ran = run(ethtx_program::TEXT, ethtx_program::DATA, b"", 40000);
    assert!(ran.halted_at.is_some(), "the frame went and the core halted");
    assert_eq!(ran.sent.len(), 1, "exactly one frame left the port");
    assert_eq!(ran.sent[0], frame, "and it is the frame the core wrote");
}

/// The terminal types four bytes, and the program takes each through
/// the interrupt controller, one interrupt a byte, never polling the
/// serial port for input.
#[test]
fn input_comes_by_interrupt_one_byte_each() {
    let ran = run(irq_program::TEXT, irq_program::DATA, b"ping", 20000);
    assert_eq!(ran.said, "ready\ngot ping in 4 interrupts\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

/// A byte at a time onto the serial port, waiting while it is busy.
/// `x1` holds the page the port is on.
fn say(a: &mut vreteno32::program::Asm, text: &[u8]) {
    use vreteno32::isa::{addi, andi, bne, lw, sw};
    for byte in text {
        let wait = a.label();
        a.place(wait);
        a.emit(lw(2, 1, 4)); // x2 = the status
        a.emit(andi(2, 2, 1)); // busy?
        a.to(wait, |off| bne(2, 0, off));
        a.emit(addi(3, 0, *byte as i32));
        a.emit(sw(3, 1, 0)); // the byte goes out
    }
}

/// A program that uses the remote peripheral: it writes a word to
/// `0x3300`, reads it back, and says on the serial port whether what
/// came back is what went out. Everything between the store and the
/// load is a frame leaving the Ethernet port, a program reading it,
/// and a frame coming back.
fn remote_program() -> Vec<u32> {
    use vreteno32::isa::{addi, beq, halt, jal, lui, lw, sw, UART_BASE};
    let mut a = vreteno32::program::Asm::default();
    // The serial port and the peripheral are on one page, so one
    // register addresses both.
    a.emit(lui(1, UART_BASE >> 12));
    a.emit(lui(4, 0xdead0));
    a.emit(addi(4, 4, 0x123)); // x4 = 0xdead0123
    a.emit(sw(4, 1, 0x300)); // the peripheral, at 0x3300
    a.emit(lw(5, 1, 0x300)); // and back from it
    let same = a.label();
    let done = a.label();
    a.to(same, |off| beq(5, 4, off));
    say(&mut a, b"remote bad\n");
    a.to(done, |off| jal(0, off));
    a.place(same);
    say(&mut a, b"remote ok\n");
    a.place(done);
    a.emit(halt());
    a.words()
}

/// A program that reads its own first word out of the boot memory over
/// the bus, tries to overwrite it, reads it again, and says whether the
/// memory kept it. The memory refuses the store, and the core raises
/// the store access fault for it (issue 417), so the program has a
/// handler that says so and returns to where the fault was taken.
fn rom_program() -> Vec<u32> {
    use vreteno32::isa::{
        addi, beq, csrrw, halt, jal, lui, lw, mret, sw, CSR_MTVEC, UART_BASE,
    };
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the serial port, for say
    let handler = a.label();
    a.abs(handler, |h| addi(7, 0, h as i32)); // x7 = the handler
    a.emit(csrrw(0, CSR_MTVEC, 7));
    a.emit(lui(6, 0)); // x6 = 0, the boot memory
    a.emit(lw(9, 6, 0)); // x9 = the program's first word, in a
                         // register the handler's say leaves alone
    a.emit(lui(4, 0xdead0));
    a.emit(addi(4, 4, 0x123)); // x4 = 0xdead0123
    a.emit(sw(4, 6, 0)); // refused, and answered so
    a.emit(lw(5, 6, 0)); // x5 = the word again
    let kept = a.label();
    let done = a.label();
    a.to(kept, |off| beq(5, 9, off));
    say(&mut a, b"rom changed\n");
    a.to(done, |off| jal(0, off));
    a.place(kept);
    say(&mut a, b"rom kept\n");
    a.place(done);
    a.emit(halt());
    // The store's fault: taken before whichever instruction was next
    // when the answer came back, so the handler returns to it as is.
    a.place(handler);
    say(&mut a, b"refused\n");
    a.emit(mret());
    a.words()
}

/// The boot memory is on the bus at zero: a load reads the program
/// that is there, and a store does not change it. The memory answers
/// the store with a refusal, which the core raises as a store access
/// fault (issue 417), so the program says `refused` from its handler
/// and then that the word is what it was; that the first load returned
/// the program and not zero is checked too, since a hole answers zero
/// as readily.
#[test]
fn the_boot_memory_is_readable_and_not_writable() {
    let text = rom_program();
    let ran = run(&text, b"", b"", 4000);
    assert_eq!(ran.said, "refused\nrom kept\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
    assert_ne!(text[0], 0, "the word the program reads is not zero");
}

/// The core writes a word to a device that is a program on the other
/// side of the Ethernet port, reads it back, and gets what it wrote.
/// Nothing on the bus knows the device is software: the transaction
/// leaves the board as a frame and the answer arrives as one.
#[test]
fn the_core_reaches_a_program_across_the_ethernet_port() {
    let ran = run_served(&remote_program(), b"", 8000);
    assert_eq!(ran.said, "remote ok\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
    // Two transactions, two frames: the store and the load.
    assert_eq!(ran.sent.len(), 2, "a frame each");
}

/// What leaves the board is the protocol's frame, read here off the
/// port rather than out of the peripheral: the store the program above
/// makes, addressed to the peripheral's own address, carrying the word
/// and all four lanes, from this board's device number.
#[test]
fn a_transaction_leaves_the_board_as_a_frame() {
    let ran = run(&remote_program(), b"", b"", 2000);
    let frame = &ran.sent[0];
    assert_eq!(frame.len(), FRAME_LEN as usize, "one frame, whole");
    assert_eq!(frame[12..14], [0x88, 0xb5], "the type");
    assert_eq!(frame[14], KIND_ASK as u8, "an ask");
    assert_eq!(frame[15], REMOTE_DEV as u8, "this board");
    assert_eq!(frame[17] & 1, 1, "a write");
    assert_eq!(&frame[18..22], &0x3300u32.to_be_bytes(), "the address");
    assert_eq!(&frame[22..26], &0xdead_0123u32.to_be_bytes(), "the word");
    assert_eq!(frame[26], 0xf, "every lane");
    // With nothing answering, the core waits on the peripheral, which
    // waits `REMOTE_WAIT` cycles before it answers the bus itself.
    // That is a second on the board, longer than this run.
    assert_eq!(ran.said, "", "the core is still waiting");
    assert!(ran.halted_at.is_none(), "and has not halted");
}

/// One module holds the rest, and the controller is an instance of its
/// wrapper that the netlist does not write, with the memory's pads
/// running out to the board's own ports.
#[test]
fn the_netlist_holds_the_controller() {
    let v = TestBoard::verilog("board");
    assert!(v.contains("module board("), "the top");
    assert!(v.contains("module board_cpu("), "the core");
    assert!(v.contains("module board_ddr3_bridge("), "the bridge");
    assert!(v.contains("ddr3_wb32 "), "the controller");
    assert!(!v.contains("module ddr3_wb32"), "not written");
    assert!(v.contains("inout [31:0] dq"), "the data pads");
}

/// A program for the loader to load: it says `hi` on the serial port
/// and halts. It is assembled here rather than compiled, because a
/// compiled image is linked for the boot memory and this one runs
/// wherever the stream says, which is what a loader is for.
fn payload() -> Vec<u32> {
    use vreteno32::isa::{addi, andi, bne, halt, lui, lw, sw, UART_BASE};
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the serial port
    for byte in b"hi\n" {
        // Wait while the port is busy, then write the byte.
        let wait = a.label();
        a.place(wait);
        a.emit(lw(2, 1, 4)); // x2 = the status
        a.emit(andi(2, 2, 1)); // busy?
        a.to(wait, |off| bne(2, 0, off));
        a.emit(addi(3, 0, *byte as i32));
        a.emit(sw(3, 1, 0)); // the byte goes out
    }
    a.emit(halt());
    a.words()
}

/// Where the sender pauses for the loader's acknowledgement: the
/// header, then every two words, then the checksum on its own. Two is
/// `BLOCK_WORDS` in the loader, and the two have to agree.
fn blocks(words: usize) -> Vec<usize> {
    let mut out = vec![12];
    let mut left = words;
    while left > 0 {
        let take = if left > 1 { 1 } else { left };
        out.push(take * 4);
        left -= take;
    }
    out.push(4);
    out
}

/// The stream the loader takes: the magic word, the address, the
/// length, the words and their sum, every number least significant
/// byte first.
fn stream(addr: u32, words: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut word = |w: u32| out.extend_from_slice(&w.to_le_bytes());
    word(0x444c_5854);
    word(addr);
    word(words.len() as u32 * 4);
    let mut sum: u32 = 0;
    for w in words {
        word(*w);
        sum = sum.wrapping_add(*w);
    }
    word(sum);
    out
}

/// The loader takes a program off the serial port, writes it into the
/// memory's region and jumps to it, and the program runs.
#[test]
fn the_loader_loads_a_program_and_runs_it() {
    let addr = 0x4000_0000;
    let ran = run_paced(
        boot_program::TEXT,
        boot_program::DATA,
        &stream(addr, &payload()),
        &blocks(payload().len()),
        400_000,
    );
    // The loader says `boot`, then `load` when the header arrives, then
    // one `K` for the header and one for each word, then `ok` with the
    // address it jumps to; the program it loaded says `hi`.
    let acks = "K".repeat(payload().len() + 1);
    assert_eq!(
        ran.said,
        format!("boot\nload\n{acks}ok 40000000\nhi\n"),
        "typed {} of {} bytes, the last at cycle {} of {}",
        ran.typed,
        stream(addr, &payload()).len(),
        ran.typed_at,
        ran.ran_for
    );
    assert!(ran.halted_at.is_some(), "the loaded program halted");
}

/// A stream whose checksum does not match is refused, and the loader
/// waits for another rather than jumping into whatever arrived.
#[test]
fn the_loader_refuses_a_stream_whose_sum_is_wrong() {
    let addr = 0x4000_0000;
    let mut bytes = stream(addr, &payload());
    let last = bytes.len() - 4;
    bytes[last] ^= 0xff;
    let ran = run_paced(
        boot_program::TEXT,
        boot_program::DATA,
        &bytes,
        &blocks(payload().len()),
        400_000,
    );
    assert!(ran.said.contains("bad sum "), "{}", ran.said);
    // The second pass of the loop says so, which is what tells a
    // return from a loaded program apart from a reset (issue 413).
    assert!(
        ran.said.ends_with("boot again\n"),
        "it waits for another: {}",
        ran.said
    );
    assert!(ran.halted_at.is_none(), "nothing was jumped into");
}

/// An address outside the memory's region is refused, so a stream
/// cannot write over the peripherals.
#[test]
fn the_loader_refuses_an_address_outside_the_memory() {
    let ran = run_paced(
        boot_program::TEXT,
        boot_program::DATA,
        &stream(0x3000, &payload()),
        &blocks(payload().len()),
        200_000,
    );
    assert!(ran.said.starts_with("boot\nload\nbad len "), "{}", ran.said);
    assert!(ran.halted_at.is_none(), "nothing was jumped into");
}

/// A program that counts in `x5` to three thousand, says `done` and
/// halts: long enough to be caught in the middle by a debugger.
fn count_program() -> Vec<u32> {
    use vreteno32::isa::{addi, blt, halt, lui, UART_BASE};
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the serial port, for say
    a.emit(addi(5, 0, 0));
    a.emit(addi(6, 0, 1500));
    a.emit(addi(6, 6, 1500)); // x6 = 3000
    let again = a.label();
    a.place(again);
    a.emit(addi(5, 5, 1));
    a.to(again, |off| blt(5, 6, off));
    say(&mut a, b"done\n");
    a.emit(halt());
    a.words()
}

/// The debug module from the cable (issue 154): a debugger on the
/// JTAG pins halts the counting program, sees it halted, reads the
/// count and `dpc` by the abstract command, writes the count to its
/// end, resumes, and sees the core running with the resume
/// acknowledged. The program then finishes at once, which is the write
/// having landed in the register file: three thousand iterations take
/// nine thousand cycles, and the run ends well before that.
#[test]
fn the_debug_module_halts_reads_writes_and_resumes_the_core() {
    use vreteno32::debug::{
        access, at, ABSTRACTCS, ALLHALTED, ALLRESUMEACK, ALLRUNNING, COMMAND,
        DATA0, DMACTIVE, DMCONTROL, DMSTATUS, HALTREQ, HALTSUM0, REGNO_GPR,
        RESUMEREQ,
    };
    let plan = [
        Op::Wait(300),
        Op::Write(at(DMCONTROL), DMACTIVE),
        Op::Write(at(DMCONTROL), HALTREQ | DMACTIVE),
        Op::Wait(20),
        Op::Read(at(DMSTATUS)), // 0
        Op::Read(at(HALTSUM0)), // 1
        Op::Write(at(COMMAND), access(REGNO_GPR + 5, false)),
        Op::Wait(6),
        Op::Read(at(DATA0)), // 2: the count
        Op::Write(at(COMMAND), access(0x7b1, false)),
        Op::Wait(6),
        Op::Read(at(DATA0)),      // 3: dpc
        Op::Read(at(ABSTRACTCS)), // 4
        Op::Write(at(DATA0), 2999),
        Op::Write(at(COMMAND), access(REGNO_GPR + 5, true)),
        Op::Wait(6),
        Op::Read(at(ABSTRACTCS)), // 5
        Op::Write(at(DMCONTROL), DMACTIVE),
        Op::Write(at(DMCONTROL), RESUMEREQ | DMACTIVE),
        Op::Wait(20),
        Op::Read(at(DMSTATUS)), // 6
    ];
    let ran = run_debugged(&count_program(), &[], 12000, &plan);
    assert_eq!(
        ran.steps,
        plan.len(),
        "the plan ran through: {:x?}",
        ran.got
    );
    let got = &ran.got;
    assert_eq!(got[0] & ALLHALTED, ALLHALTED, "halted: {:#x}", got[0]);
    assert_eq!(got[0] & ALLRUNNING, 0);
    assert_eq!(got[1], 1, "haltsum0");
    assert!(got[2] > 0 && got[2] < 3000, "the count so far: {}", got[2]);
    assert!((16..24).contains(&got[3]), "dpc in the loop: {:#x}", got[3]);
    assert_eq!(got[4] >> 8 & 7, 0, "no command error");
    assert_eq!(got[5] >> 8 & 7, 0, "no command error on the write");
    assert_eq!(got[6] & ALLRUNNING, ALLRUNNING, "running: {:#x}", got[6]);
    assert_eq!(got[6] & ALLRESUMEACK, ALLRESUMEACK, "acknowledged");
    assert_eq!(ran.said, "done\n");
    let halted_at = ran.halted_at.expect("the program halted itself");
    assert!(
        halted_at < 3000,
        "the write to x5 ended the loop: {halted_at}"
    );
}
