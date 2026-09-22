// SPDX-License-Identifier: Apache-2.0
//! Programs for the core, as words: a small assembler with labels,
//! the demonstration program, and a generator of random straight-line
//! programs for the lockstep test.
use crate::isa::*;
use crate::model::DATA_BASE;

/// A branch or a jump whose label is not placed yet: a thirty-two bit
/// one or a compressed one, as a function of the byte offset.
enum Fixup {
    Wide(Box<dyn Fn(i32) -> u32>),
    Short(Box<dyn Fn(i32) -> u16>),
}

/// Instructions with labels, as halfwords: a thirty-two bit instruction
/// is two of them, the low half first, and a compressed one is one.
/// Addresses and offsets are bytes, two to a halfword. A forward branch
/// is emitted with a placeholder and patched when its label is placed.
#[derive(Default)]
pub struct Asm {
    pub halves: Vec<u16>,
    /// Whether `emit` writes an instruction in its compressed spelling
    /// when it has one. Branches and jumps to labels, and addresses,
    /// stay thirty-two bits, since their size is fixed before the
    /// label is placed.
    pub compress: bool,
    fixups: Vec<(usize, usize, Fixup)>,
    abs_fixups: Vec<(usize, usize, Box<dyn Fn(u32) -> u32>)>,
    labels: Vec<Option<usize>>,
}

impl Asm {
    /// An instruction: compressed if `compress` is set and it can be,
    /// else thirty-two bits.
    pub fn emit(&mut self, w: u32) {
        match compress(w) {
            Some(h) if self.compress => self.emit_c(h),
            _ => self.wide(w),
        }
    }
    /// A thirty-two bit instruction, whatever `compress` says.
    pub fn wide(&mut self, w: u32) {
        self.halves.push(w as u16);
        self.halves.push((w >> 16) as u16);
    }
    /// A compressed instruction.
    pub fn emit_c(&mut self, h: u16) {
        self.halves.push(h);
    }
    /// Where the next instruction goes, in bytes.
    pub fn here(&self) -> usize {
        self.halves.len() * 2
    }
    /// A thirty-two bit instruction in place of the two halfwords at
    /// `at`.
    fn put(&mut self, at: usize, w: u32) {
        self.halves[at] = w as u16;
        self.halves[at + 1] = (w >> 16) as u16;
    }
    /// Pad to a whole word with a c.nop, if the next instruction would
    /// start in the upper half of one. A trap handler needs it: mtvec
    /// holds an address whose low two bits are zero.
    pub fn align(&mut self) {
        if self.halves.len() % 2 == 1 {
            self.emit_c(c_nop());
        }
    }
    /// A fresh label, not yet placed.
    pub fn label(&mut self) -> usize {
        self.labels.push(None);
        self.labels.len() - 1
    }
    /// Place a label here.
    pub fn place(&mut self, l: usize) {
        let here = self.halves.len();
        self.labels[l] = Some(here);
        let fixups = std::mem::take(&mut self.fixups);
        for (at, lbl, enc) in &fixups {
            if *lbl == l {
                let off = (here as i32 - *at as i32) * 2;
                match enc {
                    Fixup::Wide(e) => self.put(*at, e(off)),
                    Fixup::Short(e) => self.halves[*at] = e(off),
                }
            }
        }
        self.fixups = fixups;
        let abs_fixups = std::mem::take(&mut self.abs_fixups);
        for (at, lbl, enc) in &abs_fixups {
            if *lbl == l {
                self.put(*at, enc(here as u32 * 2));
            }
        }
        self.abs_fixups = abs_fixups;
    }
    /// A label's absolute address, `enc` taking it: what a handler's
    /// address in mtvec needs. The program is at zero.
    pub fn abs(&mut self, l: usize, enc: impl Fn(u32) -> u32 + 'static) {
        let at = self.halves.len();
        match self.labels[l] {
            Some(t) => self.wide(enc(t as u32 * 2)),
            None => {
                self.abs_fixups.push((at, l, Box::new(enc)));
                self.wide(0);
            }
        }
    }
    /// A branch or jump to a label, `enc` taking the byte offset.
    pub fn to(&mut self, l: usize, enc: impl Fn(i32) -> u32 + 'static) {
        let at = self.halves.len();
        match self.labels[l] {
            Some(t) => self.wide(enc((t as i32 - at as i32) * 2)),
            None => {
                self.fixups.push((at, l, Fixup::Wide(Box::new(enc))));
                self.wide(0);
            }
        }
    }
    /// A compressed branch or jump to a label.
    pub fn to_c(&mut self, l: usize, enc: impl Fn(i32) -> u16 + 'static) {
        let at = self.halves.len();
        match self.labels[l] {
            Some(t) => self.emit_c(enc((t as i32 - at as i32) * 2)),
            None => {
                self.fixups.push((at, l, Fixup::Short(Box::new(enc))));
                self.emit_c(0);
            }
        }
    }
    /// The program as the instruction memory holds it: little-endian
    /// words, two halfwords each, the last padded with a zero halfword
    /// if it needs one.
    pub fn words(&self) -> Vec<u32> {
        self.halves
            .chunks(2)
            .map(|c| c[0] as u32 | (*c.get(1).unwrap_or(&0) as u32) << 16)
            .collect()
    }
}

/// The demonstration: a loop that sums one to ten, a call that
/// doubles the sum, stores and loads of every width with the sign
/// extension they imply, the upper immediates, a trap handler that
/// an ecall and an illegal word reach and return from, the CSRs,
/// a use of a word the instruction before it loaded, which stalls a
/// cycle, the multiplies and divides, a line said on the serial port
/// and three bytes echoed from it, then the halt. Interrupts are
/// enabled from the start, the timer's at once and set to 150, the
/// line's after the echo, since the port's byte raises it too; the
/// handler counts interrupts in x8, two by the end. It leaves 110 in
/// x10 and at the first data word, the second trap's cause in x23, 5
/// in x24, 0xfe01 in x25, -220 in x26, -55 in x29 and -2 in x30, and
/// has written OK, a newline and the three bytes to the serial port.
pub fn demo() -> Vec<u32> {
    // Every instruction that has a compressed spelling is written in
    // it, as a compiler would, so the fetch sees both lengths.
    let mut a = Asm {
        compress: true,
        ..Asm::default()
    };
    let (top, done, double) = (a.label(), a.label(), a.label());
    let (handler, sync, count) = (a.label(), a.label(), a.label());
    a.emit(lui(2, DATA_BASE >> 12)); // x2 = data base

    // Interrupts: the handler's address into mtvec, the external
    // interrupt enabled in mie, interrupts enabled in mstatus; the
    // handler counts them in x8.
    a.abs(handler, |h| addi(21, 0, h as i32)); // x21 = the handler
    a.emit(csrrw(0, CSR_MTVEC, 21)); // mtvec = x21
    a.emit(lui(9, 1)); // x9 = 0x1000
    a.emit(srli(9, 9, 1)); // x9 = 0x800, the external interrupt's bit
                           // x4 = the compare's page in the interrupt controller, which is
                           // where the two halves of `mtimecmp` are.
    a.emit(lui(4, (CLINT_BASE + MTIMECMP_OFF) >> 12));
    a.emit(addi(3, 0, 150)); // x3 = 150
    a.emit(sw(3, 4, 0)); // mtimecmp = 150: a timer interrupt then
    a.emit(sw(0, 4, 4)); // and its high half zero: a reset leaves the
                         // compare all ones (issue 419)
    a.emit(ori(9, 9, 0x80)); // x9 = MEXT | MTIMER, the handler's mask
    a.emit(addi(22, 0, 0x80)); // x22 = MTIMER
    a.emit(csrrw(0, CSR_MIE, 22)); // mie = the timer's, for now
    a.emit(csrrsi(0, CSR_MSTATUS, 8)); // mstatus.MIE = 1
    a.emit(addi(5, 0, 10)); // x5 = 10, the count
    a.emit(addi(10, 0, 0)); // x10 = 0, the sum
    a.emit(addi(6, 0, 0)); // x6 = 0, i
    a.place(top);
    a.emit(addi(6, 6, 1)); // i += 1
    a.emit(add(10, 10, 6)); // sum += i
    a.to(top, |o| bne(6, 5, o)); // until i == 10
    a.to(double, |o| jal(1, o)); // x10 = double(x10)
    a.emit(sw(10, 2, 0)); // mem[0] = 110
    a.emit(addi(7, 0, -2)); // x7 = -2
    a.emit(sb(7, 2, 5)); // a byte of it at 5
    a.emit(sh(7, 2, 10)); // a half of it at 10
    a.emit(lb(11, 2, 5)); // x11 = -2, sign extended
    a.emit(lbu(12, 2, 5)); // x12 = 254
    a.emit(lh(13, 2, 10)); // x13 = -2
    a.emit(lhu(14, 2, 10)); // x14 = 65534
    a.emit(lw(15, 2, 4)); // x15 = the byte in its word
    a.emit(addi(25, 15, 1)); // x25 = x15 + 1, a use right after the load
    a.emit(auipc(16, 1)); // x16 = pc + 4096
    a.emit(srai(17, 7, 1)); // x17 = -1
    a.emit(slt(18, 7, 0)); // x18 = 1: -2 < 0
    a.emit(sltu(19, 7, 0)); // x19 = 0: big unsigned
    a.emit(xori(20, 7, -1)); // x20 = 1

    // The M extension: a multiply takes three cycles in the core, a
    // divide thirty-three.
    a.emit(mul(26, 10, 7)); // x26 = 110 * -2 = -220
    a.emit(mulh(27, 7, 7)); // x27 = high word of 4 = 0
    a.emit(mulhu(28, 7, 7)); // x28 = high word of 0xfffffffe^2
    a.emit(div(29, 10, 7)); // x29 = 110 / -2 = -55
    a.emit(rem(30, 7, 5)); // x30 = -2 rem 10 = -2

    // The serial port: three bytes, each written once the port is not
    // busy, which a load of the status says.
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the port; the call is done
    for byte in b"OK\n" {
        let wait = a.label();
        a.place(wait);
        a.emit(lw(21, 1, 4)); // x21 = status
        a.emit(andi(21, 21, 1)); // busy?
        a.to(wait, |o| bne(21, 0, o)); // then ask again
        a.emit(addi(21, 0, *byte as i32)); // the byte
        a.emit(sw(21, 1, 0)); // out it goes
    }
    // The port's other side: three bytes typed at the terminal, each
    // taken once the status says one has come, and echoed once the
    // port is free. Then the line's interrupt is enabled: the bytes
    // raised it too, and it is pending by now.
    for _ in 0..3 {
        let (came, free) = (a.label(), a.label());
        a.place(came);
        a.emit(lw(21, 1, 4)); // x21 = status
        a.emit(andi(21, 21, 2)); // a byte received?
        a.to(came, |o| beq(21, 0, o)); // else ask again
        a.emit(lw(20, 1, 8)); // x20 = the byte, which clears the bit
        a.place(free);
        a.emit(lw(21, 1, 4)); // x21 = status
        a.emit(andi(21, 21, 1)); // busy?
        a.to(free, |o| bne(21, 0, o)); // then ask again
        a.emit(sw(20, 1, 0)); // the byte, back out
    }
    a.emit(csrrw(0, CSR_MIE, 9)); // mie = both

    // Traps: an ecall, an illegal word, each returning to the word
    // after it; then the CSRs.
    a.emit(ecall()); // trap, cause 11
    a.emit(0xffff_ffff); // an illegal word: trap, cause 2, mtval = it
    a.emit(csrrwi(0, CSR_MSCRATCH, 5)); // mscratch = 5
    a.emit(csrrs(24, CSR_MSCRATCH, 0)); // x24 = mscratch
    a.to(done, |o| jal(0, o));
    a.place(double);
    a.emit(add(10, 10, 10));
    a.emit(jalr(0, 1, 0)); // return
    a.align(); // mtvec holds a whole word's address
    a.place(handler);
    a.emit(csrrs(23, CSR_MCAUSE, 0)); // x23 = mcause
    a.to(sync, |o| bge(23, 0, o)); // an exception: cause positive
    a.emit(csrrc(0, CSR_MIP, 9)); // an interrupt: clear the line's bit,
    a.emit(andi(22, 23, 0xff)); // x22 = which interrupt
    a.emit(addi(3, 0, 7)); // x3 = the timer's
    a.to(count, |o| bne(22, 3, o)); // the line's: nothing more
                                    // The timer's: the count sits far from the compare in the
                                    // controller's window, too far for one base register, so it is
                                    // read through x22 and the compare is written through x4.
    a.emit(lui(22, (CLINT_BASE + MTIME_OFF + 8) >> 12));
    a.emit(lw(22, 22, -8)); // x22 = the count's low half
    a.emit(addi(22, 22, 1000)); // the next one well past the end
    a.emit(sw(22, 4, 0)); // mtimecmp = x22
    a.emit(lw(22, 4, 0)); // and read back: the store is posted, and the
                          // line drops only when it lands, which is before
                          // this load returns; a return before that took
                          // the interrupt twice (issue 420)
    a.place(count);
    a.emit(addi(8, 8, 1)); // count it,
    a.emit(mret()); // and return to the interrupted word
    a.place(sync);
    a.emit(csrrs(22, CSR_MEPC, 0)); // x22 = mepc
    a.emit(addi(22, 22, 4)); // past the trapping word
    a.emit(csrrw(0, CSR_MEPC, 22)); // mepc = x22
    a.emit(mret());
    a.place(done);
    a.emit(halt());
    a.words()
}

/// A random straight-line program: register operations, the M
/// extension among them, aligned stores and loads within the first
/// data words, a forward branch or jump now and then, CSR operations
/// on mscratch, an ecall or an illegal instruction now and then, which
/// a handler after the end returns from, and the halt at the end.
/// About half the instructions that have a compressed spelling are
/// written in it, and some branches and jumps are compressed ones, so
/// the two lengths are mixed and a thirty-two bit instruction often
/// starts in the upper half of a word. `seed` is the whole of it.
/// A program that interrupts itself: it raises the software interrupt
/// by writing `msip` in the interrupt controller, takes the trap,
/// clears it in the handler and counts it, three times, and halts.
///
/// It is the shape an operating system's scheduler has, which is why
/// the register is there: a port enters its scheduler by writing this
/// one bit.
pub fn soft() -> Vec<u32> {
    let mut a = Asm {
        compress: true,
        ..Asm::default()
    };
    let (top, handler, done) = (a.label(), a.label(), a.label());
    a.emit(lui(2, DATA_BASE >> 12)); // x2 = the data base
    a.abs(handler, |h| addi(21, 0, h as i32)); // x21 = the handler
    a.emit(csrrw(0, CSR_MTVEC, 21)); // mtvec = x21
    a.emit(lui(4, CLINT_BASE >> 12)); // x4 = msip's page, which is
    a.emit(addi(22, 0, 8)); // x22 = MSOFT, the software interrupt's bit
    a.emit(csrrw(0, CSR_MIE, 22)); // mie = MSOFT
    a.emit(csrrsi(0, CSR_MSTATUS, 8)); // mstatus.MIE = 1
    a.emit(addi(8, 0, 0)); // x8 = 0, how many were taken
    a.emit(addi(5, 0, 3)); // x5 = 3, how many to take
    a.place(top);
    a.emit(addi(3, 0, 1));
    a.emit(sw(3, 4, 0)); // msip = 1: the interrupt is raised here
    a.emit(addi(0, 0, 0)); // and taken in one of the cycles after it
    a.emit(addi(0, 0, 0));
    a.emit(addi(0, 0, 0));
    a.emit(addi(0, 0, 0));
    a.to(top, |o| blt(8, 5, o)); // until three have been taken
    a.emit(sw(8, 2, 0)); // mem[0] = how many were taken
    a.place(done);
    a.emit(halt());
    // The handler: clear `msip`, which is what makes the line fall,
    // count the interrupt, and return.
    a.align();
    a.place(handler);
    a.emit(sw(0, 4, 0)); // msip = 0
    a.emit(addi(8, 8, 1)); // one more taken
    a.emit(mret());
    a.words()
}

/// What the machine says it is, read by a program: `mhartid`, `misa`
/// and a write to a read-only register, which traps.
///
/// It is the first thing a stock kernel does. Zephyr's
/// `arch/riscv/core/reset.S` begins `csrr a0, mhartid`, and before
/// issue 274 that instruction was an illegal one, so the kernel took a
/// trap into a handler it had not installed yet.
///
/// The program writes three words where the test can read them: the
/// hart's index, what `misa` says the machine is, and how many illegal
/// instructions the handler counted, which is one.
pub fn machine_info() -> Vec<u32> {
    let mut a = Asm::default();
    let handler = a.label();
    a.abs(handler, |h| addi(21, 0, h as i32)); // x21 = the handler
    a.emit(csrrw(0, CSR_MTVEC, 21)); // mtvec = x21
    a.emit(lui(2, DATA_BASE >> 12)); // x2 = the data base
                                     // A read is `csrrs` with `x0` as its source, which the
                                     // specification says is not a write, so a read-only register
                                     // answers it.
    a.emit(csrrs(8, CSR_MHARTID, 0)); // x8 = mhartid
    a.emit(sw(8, 2, 0)); // mem[0] = mhartid
    a.emit(csrrs(9, CSR_MISA, 0)); // x9 = misa
    a.emit(sw(9, 2, 4)); // mem[1] = misa
                         // A write to one of them is an illegal instruction, and the
                         // handler counts it and steps over it.
    a.emit(addi(5, 0, 1)); // x5 = 1, a source that is not x0
    a.emit(csrrw(0, CSR_MHARTID, 5)); // illegal
    a.emit(sw(6, 2, 8)); // mem[2] = how many were counted
    a.emit(halt());
    // The handler: count the trap, step over the instruction that
    // caused it, and return.
    a.align();
    a.place(handler);
    a.emit(addi(6, 6, 1)); // one more illegal instruction
    a.emit(csrrs(7, CSR_MEPC, 0)); // x7 = mepc
    a.emit(addi(7, 7, 4)); // past the instruction
    a.emit(csrrw(0, CSR_MEPC, 7)); // mepc = x7
    a.emit(mret());
    a.words()
}

/// A program that measures itself with the two machine counters.
///
/// It reads `mcycle` and `minstret`, does a division, reads both
/// again, and writes the two differences where a test can see them.
/// A division is the point: the sequencer takes about thirty-three
/// cycles and retires one instruction, so the cycles between the two
/// reads are many more than the instructions, which is the whole
/// reason a profiler wants both counters rather than either.
pub fn measure() -> Vec<u32> {
    let mut a = Asm::default();
    a.emit(lui(2, DATA_BASE >> 12)); // x2 = the data base
    a.emit(csrrs(10, CSR_MCYCLE, 0)); // x10 = cycles before
    a.emit(csrrs(11, CSR_MINSTRET, 0)); // x11 = retired before
                                        // The work: a division, which stalls, and a few plain
                                        // instructions around it.
    a.emit(addi(5, 0, 1000));
    a.emit(addi(6, 0, 7));
    a.emit(div(7, 5, 6)); // x7 = 1000 / 7, thirty-odd cycles
    a.emit(addi(8, 7, 0));
    a.emit(csrrs(12, CSR_MCYCLE, 0)); // x12 = cycles after
    a.emit(csrrs(13, CSR_MINSTRET, 0)); // x13 = retired after
    a.emit(sub(14, 12, 10)); // x14 = the cycles between
    a.emit(sub(15, 13, 11)); // x15 = the instructions between
    a.emit(sw(14, 2, 0)); // mem[0] = cycles
    a.emit(sw(15, 2, 4)); // mem[1] = instructions
    a.emit(sw(7, 2, 8)); // mem[2] = what the division came to
    a.emit(halt());
    a.words()
}

/// A program that idles: it arms the timer, waits for it with `wfi`,
/// and counts the interrupts that woke it.
///
/// This is the shape of an operating system with nothing to run. The
/// core stops fetching at the `wfi` and starts again when the timer's
/// line comes up, so the cycles between are cycles in which nothing
/// retires, which is what the lockstep test measures.
///
/// The second half is the part worth having: the `wfi` after the
/// handler returns runs with interrupts disabled, and the core wakes
/// from it anyway, because the specification says a wait ends when an
/// interrupt is pending and enabled whether or not `mstatus` lets it
/// be taken. A kernel idles inside its own lock, so a core that waited
/// for the global bit would never wake.
pub fn idle() -> Vec<u32> {
    let mut a = Asm::default();
    let (top, handler) = (a.label(), a.label());
    a.abs(handler, |h| addi(21, 0, h as i32)); // x21 = the handler
    a.emit(csrrw(0, CSR_MTVEC, 21)); // mtvec = x21
    a.emit(lui(2, DATA_BASE >> 12)); // x2 = the data base
                                     // The compare and the count are too far apart in the controller's
                                     // window for one base register to reach both with a twelve-bit
                                     // offset, so each gets its own.
    a.emit(lui(4, (CLINT_BASE + MTIMECMP_OFF) >> 12)); // x4 = compare
    a.emit(lui(7, (CLINT_BASE + MTIME_OFF + 8) >> 12)); // x7 = count + 8
    a.emit(addi(22, 0, 128)); // x22 = MTIMER, the timer's bit
    a.emit(csrrw(0, CSR_MIE, 22)); // mie = MTIMER
    a.emit(csrrsi(0, CSR_MSTATUS, 8)); // mstatus.MIE = 1
    a.emit(addi(8, 0, 0)); // x8 = 0, how many woke it
    a.emit(addi(5, 0, 2)); // x5 = 2, how many to wait for
    a.place(top);
    // Arm the timer a little ahead of the count, then wait for it.
    a.emit(lw(3, 7, -8)); // x3 = the count's low half
    a.emit(addi(3, 3, 300)); // x3 += 300, well past the stores
    a.emit(sw(0, 4, 4)); // the compare's high half is zero
    a.emit(sw(3, 4, 0)); // and its low half is the count plus 40
    a.emit(wfi()); // stop here until the line comes up
    a.to(top, |o| blt(8, 5, o)); // until two have woken it
    a.emit(sw(8, 2, 0)); // mem[0] = how many woke it
    a.emit(halt());
    // The handler: push the compare out of reach so the line falls,
    // count the wake, and return.
    a.align();
    a.place(handler);
    a.emit(addi(6, 0, -1)); // x6 = the largest compare there is
    a.emit(sw(6, 4, 4)); // the compare's high half, so the line falls
    a.emit(addi(8, 8, 1)); // one more wake
    a.emit(mret());
    a.words()
}

/// The two halves of a program that lives above the boot memory: what
/// the boot memory holds, which is a jump into the data memory, and
/// what the data memory holds, which is the program itself.
///
/// It is what a bootloader leaves behind: the core starts in the boot
/// memory, and everything after the jump is fetched over the bus
/// (issue 134). The program adds the first ten numbers, writes the sum
/// where the test can read it, and halts.
pub fn in_memory() -> (Vec<u32>, Vec<u32>) {
    let mut boot = Asm::default();
    boot.emit(lui(1, DATA_BASE >> 12)); // x1 = the data memory
    boot.emit(jalr(0, 1, 0)); // and away
                              // Its branches are relative and its addresses are written out, so
                              // the program runs where it is loaded without being told where.
    let mut a = Asm::default();
    let top = a.label();
    a.emit(lui(2, DATA_BASE >> 12)); // x2 = the data memory
    a.emit(addi(5, 0, 10)); // x5 = 10
    a.emit(addi(6, 0, 0)); // x6 = i
    a.emit(addi(10, 0, 0)); // x10 = the sum
    a.place(top);
    a.emit(addi(6, 6, 1));
    a.emit(add(10, 10, 6));
    a.to(top, |o| bne(6, 5, o));
    a.emit(sw(10, 2, 64)); // the sum, well past the program
    a.emit(halt());
    (boot.words(), a.words())
}

pub fn random(seed: u64, len: usize) -> Vec<u32> {
    let mut s = seed.wrapping_mul(0x9e3779b97f4a7c15) | 1;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut a = Asm::default();
    let (handler, sync) = (a.label(), a.label());
    a.emit(lui(2, DATA_BASE >> 12));
    a.abs(handler, |h| addi(31, 0, h as i32));
    a.emit(csrrw(0, CSR_MTVEC, 31));
    // Interrupts enabled from the start; the line is the test's.
    a.emit(lui(31, 1));
    a.emit(srli(31, 31, 1));
    a.emit(ori(31, 31, 0x80));
    a.emit(csrrw(0, CSR_MIE, 31)); // the line and the timer
                                   // x30 = the compare's page, kept, since the handler writes it.
    a.emit(lui(30, (CLINT_BASE + MTIMECMP_OFF) >> 12));
    a.emit(csrrsi(0, CSR_MSTATUS, 8));
    while a.halves.len() < 2 * len {
        let r = next();
        // Half the instructions are shaped to have a compressed
        // spelling and are written in it: the destination is the first
        // operand, the registers are x8 to x15, and the immediate is
        // six bits.
        let squeeze = r >> 56 & 1 == 1;
        let rd = (r >> 8 & 31) as u32;
        let rs1 = (r >> 16 & 31) as u32;
        let rs2 = (r >> 24 & 31) as u32;
        let imm = ((r >> 32) as i32) >> 20;
        let (rd, rs1, rs2, imm) = if squeeze {
            let rd = 8 + (rd & 7);
            (rd, rd, 8 + (rs2 & 7), imm >> 6)
        } else {
            (rd, rs1, rs2, imm)
        };
        let amt = (r >> 40 & 31) as u32;
        let off = ((r >> 48) & 0x3f) as i32 * 4; // an aligned data offset
        let w = match r & 31 {
            0 => addi(rd, rs1, imm),
            1 => slti(rd, rs1, imm),
            2 => sltiu(rd, rs1, imm),
            3 => xori(rd, rs1, imm),
            4 => ori(rd, rs1, imm),
            5 => andi(rd, rs1, imm),
            6 => slli(rd, rs1, amt),
            7 => srli(rd, rs1, amt),
            8 => srai(rd, rs1, amt),
            9 => add(rd, rs1, rs2),
            10 => sub(rd, rs1, rs2),
            11 => sll(rd, rs1, rs2),
            12 => slt(rd, rs1, rs2),
            13 => sltu(rd, rs1, rs2),
            14 => xor(rd, rs1, rs2),
            15 => srl(rd, rs1, rs2),
            16 => sra(rd, rs1, rs2),
            // Half of the time an M instruction, by the function code.
            17 => match r >> 60 & 15 {
                0 => mul(rd, rs1, rs2),
                1 => mulh(rd, rs1, rs2),
                2 => mulhsu(rd, rs1, rs2),
                3 => mulhu(rd, rs1, rs2),
                4 => div(rd, rs1, rs2),
                5 => divu(rd, rs1, rs2),
                6 => rem(rd, rs1, rs2),
                7 => remu(rd, rs1, rs2),
                _ => or(rd, rs1, rs2),
            },
            18 => and(rd, rs1, rs2),
            19 => lui(rd, (r >> 12) as u32 & 0xfffff),
            20 => auipc(rd, (r >> 12) as u32 & 0xfffff),
            // Half of the stores and loads of a word go to the timer.
            21 => match r >> 60 & 1 {
                0 => sw(rs2, 2, off),
                _ => sw(rs2, 30, (amt as i32 & 3) * 4),
            },
            22 => sh(rs2, 2, off + (amt as i32 & 2)),
            23 => sb(rs2, 2, off + (amt as i32 & 3)),
            24 => match r >> 60 & 1 {
                0 => lw(rd, 2, off),
                _ => lw(rd, 30, (amt as i32 & 3) * 4),
            },
            25 => lh(rd, 2, off + (amt as i32 & 2)),
            26 => lb(rd, 2, off + (amt as i32 & 3)),
            27 => lhu(rd, 2, off + (amt as i32 & 2)),
            28 => lbu(rd, 2, off + (amt as i32 & 3)),
            // The CSR instructions, on mscratch.
            29 => match r >> 60 & 7 {
                0 => csrrw(rd, CSR_MSCRATCH, rs1),
                1 => csrrs(rd, CSR_MSCRATCH, rs1),
                2 => csrrc(rd, CSR_MSCRATCH, rs1),
                3 => csrrwi(rd, CSR_MSCRATCH, amt),
                4 => csrrsi(rd, CSR_MSCRATCH, amt),
                5 => csrrci(rd, CSR_MSCRATCH, amt),
                6 => csrrs(rd, CSR_MCAUSE, 0),
                _ => match r >> 63 & 1 {
                    0 => csrrs(rd, CSR_MEPC, 0),
                    _ => csrrs(rd, CSR_MTVAL, 0),
                },
            },
            // A trap: an ecall, or an illegal instruction: a word of
            // zeros, which is two illegal halfwords, a reserved
            // compressed one, or a word of ones.
            30 => match r >> 60 & 3 {
                0 => ecall(),
                1 => {
                    a.emit_c(0x8002); // c.jr x0, which is reserved
                    continue;
                }
                2 => 0,
                _ => 0xffff_ffff,
            },
            // A forward branch over the next one or two words.
            _ => {
                let l = a.label();
                let f3 = (r >> 5 & 7) as u32;
                let skip = 1 + (r >> 60 & 1) as usize;
                let enc: fn(u32, u32, i32) -> u32 = match f3 {
                    0 => beq,
                    1 => bne,
                    2 | 3 => blt,
                    4 => bge,
                    5 => bltu,
                    _ => bgeu,
                };
                // A compressed one now and then: a branch on a register
                // of x8 to x15 against zero, or a jump.
                let rs1s = 8 + (rs1 & 7);
                match r >> 54 & 3 {
                    0 => a.to_c(l, move |o| c_beqz(rs1s, o)),
                    1 => a.to_c(l, move |o| c_bnez(rs1s, o)),
                    2 => a.to_c(l, c_j),
                    _ => a.to(l, move |o| enc(rs1, rs2, o)),
                }
                a.compress = squeeze;
                for _ in 0..skip {
                    let r = next();
                    let rd = (r >> 8 & 31) as u32;
                    let rd = if rd == 2 || rd == 30 || rd == 31 {
                        3
                    } else {
                        rd
                    };
                    a.emit(addi(rd, (r >> 16 & 31) as u32, 1));
                }
                a.place(l);
                continue;
            }
        };
        // x2 stays the data base and x30 the timer's, so the loads and
        // stores stay in range, and x31 is the handler's own.
        if (rd == 2 || rd == 30 || rd == 31) && !matches!(r & 31, 21..=23 | 30)
        {
            continue;
        }
        a.compress = squeeze;
        a.emit(w);
    }
    a.emit(halt());
    // The handler: an interrupt is cleared and returned from; an
    // exception returns past the instruction that trapped.
    a.align();
    a.place(handler);
    a.emit(csrrs(31, CSR_MCAUSE, 0));
    a.to(sync, |o| bge(31, 0, o));
    a.emit(lui(31, 1));
    a.emit(srli(31, 31, 1)); // 0x800, the external interrupt's bit
    a.emit(csrrc(0, CSR_MIP, 31));
    // And the timer's next: the count plus 64. The count sits far
    // from the compare in the controller's window, too far for one
    // base register, so it is reached through x31 and the compare's
    // high half is written zero, which it is in a run this short.
    a.emit(lui(31, (CLINT_BASE + MTIME_OFF + 8) >> 12));
    a.emit(lw(31, 31, -8)); // x31 = the count's low half
    a.emit(addi(31, 31, 64));
    a.emit(sw(0, 30, 4)); // the compare's high half is zero
    a.emit(sw(31, 30, 0)); // and its low half is the count plus 64
    a.emit(lw(31, 30, 0)); // read back, so the store has landed and the
                           // line has dropped before the return (issue 420)
    a.emit(mret());
    // An exception returns past the instruction that trapped, which is
    // four bytes long for an ecall and for an illegal instruction whose
    // low two bits are both set, and two for the rest. mcause is 11 or
    // 2, so its bit 3 says ecall; x29 is the handler's too.
    a.place(sync);
    a.emit(csrrs(29, CSR_MTVAL, 0));
    a.emit(andi(29, 29, 3)); // the trap value's low two bits
    a.emit(andi(31, 31, 8)); // 8 for an ecall
    a.emit(or(29, 29, 31)); // 3 or 8 for four bytes
    a.emit(sltiu(29, 29, 3)); // 1 for two bytes
    a.emit(slli(29, 29, 1));
    a.emit(csrrs(31, CSR_MEPC, 0));
    a.emit(addi(31, 31, 4));
    a.emit(sub(31, 31, 29)); // mepc + 4 - 2, or + 4
    a.emit(csrrw(0, CSR_MEPC, 31));
    a.emit(mret());
    a.words()
}
