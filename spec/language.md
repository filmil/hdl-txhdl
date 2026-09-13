<!-- SPDX-License-Identifier: Apache-2.0 -->
# TxHDL - Transaction Hardware Description Language

## Complete Language Specification

A hardware description language based on transaction-level communication between modules over buses, with sequential operations within modules controlled by a single clock.

---

## Table of Contents

1. [Design Philosophy](#1-design-philosophy)
2. [Basic Types](#2-basic-types)
3. [Transactions](#3-transactions)
4. [Buses](#4-buses)
5. [Modules](#5-modules)
6. [Sequences](#6-sequences)
7. [Await Semantics](#7-await-semantics)
8. [Bus Operations](#8-bus-operations)
9. [Pipelines](#9-pipelines)
10. [Combinational Logic](#10-combinational-logic)
11. [Clocking and Reset](#11-clocking-and-reset)
12. [Generic Modules](#12-generic-modules)
13. [Module Instantiation](#13-module-instantiation)
14. [Structural Typing](#14-structural-typing)
15. [Control Flow](#15-control-flow)
16. [Bit Manipulation](#16-bit-manipulation)
17. [Tagged Channels and Callbacks](#17-tagged-channels-and-callbacks)
18. [Functions and Procedures](#18-functions-and-procedures)
19. [Memory and Arrays](#19-memory-and-arrays)
20. [Assertions and Verification](#20-assertions-and-verification)

---

## 1. Design Philosophy

### Core Principles

TxHDL is designed around these key ideas:

1. **Transaction-Centric**: Communication between modules happens through well-defined transactions over buses, not raw signals
2. **Sequential Within, Parallel Between**: Each module has a single sequential flow that's easy to reason about, while modules execute in parallel
3. **Implicit State**: Variables become registers automatically based on usage
4. **Blocking Semantics**: Operations can block for multiple cycles using `await`, making complex protocols readable
5. **Structural Typing**: Transactions are compatible if their fields match

### Design Decisions

| Aspect | Choice | Rationale |
|--------|--------|-----------|
| Target | Verilog/VHDL + Simulation | Practical deployment |
| Clock domains | Per-module, CDC at buses | Clean abstraction |
| State | Implicit registers | Reduces boilerplate |
| Syntax | C-like | Familiar to most engineers |
| Type system | Structural | Flexible composition |
| Parallelism | One sequence per module | Simplicity |

---

## 2. Basic Types

### Unsigned Integers

```c
// Fixed-width unsigned integers
var a: u1;      // 1-bit (boolean-compatible)
var b: u8;      // 8-bit
var c: u16;     // 16-bit
var d: u32;     // 32-bit
var e: u64;     // 64-bit
var f: u128;    // 128-bit

// Arbitrary width
var g: u<5>;    // 5-bit
var h: u<24>;   // 24-bit
var i: u<N>;    // parameterized width (N is const generic)
```

### Signed Integers

```c
// Fixed-width signed integers (two's complement)
var a: i8;      // 8-bit signed
var b: i16;     // 16-bit signed
var c: i32;     // 32-bit signed
var d: i64;     // 64-bit signed

// Arbitrary width
var e: i<12>;   // 12-bit signed
```

### Bit Vectors

```c
// Bit vectors - no arithmetic semantics, just raw bits
var data: bits<32>;     // 32-bit vector
var payload: bits<512>; // 512-bit vector

// Useful for opaque data that shouldn't be computed on
```

### Boolean

```c
var flag: bool;         // true or false
var valid: bool = true; // with initializer

// u1 and bool are interchangeable
```

### Arrays

```c
// Fixed-size arrays
var buffer: u8[256];           // 256 bytes
var matrix: u32[4][4];         // 4x4 matrix
var regfile: u64[32];          // 32 registers

// Parameterized arrays
var mem: u32[DEPTH];           // size from generic parameter
var fifo: T[N];                // generic type and size
```

### Enumerations

```c
// Simple enum (compiler assigns encoding)
enum State { Idle, Running, Paused, Done }

// Enum with explicit encoding
enum Opcode : u6 {
    ADD  = 0x00,
    SUB  = 0x01,
    AND  = 0x02,
    OR   = 0x03,
    XOR  = 0x04,
    SLT  = 0x05,
    ADDI = 0x08,
    LW   = 0x23,
    SW   = 0x2B,
    BEQ  = 0x04,
    J    = 0x02
}

// Enum with explicit width
enum AluOp : u3 {
    Add, Sub, And, Or, Xor, Slt, Sll, Srl
}

// Using enums
var state: State = Idle;
var op: Opcode = ADD;

match (state) {
    Idle => { /* ... */ }
    Running => { /* ... */ }
    _ => { /* default */ }
}
```

### Structs (Inline)

```c
// Anonymous struct types
var point: struct { x: i16; y: i16; };
point.x = 100;
point.y = 200;

// Struct with initializer
var config: struct { enable: bool; mode: u2; } = { .enable = true, .mode = 2 };
```

### Type Aliases

```c
// Create named type aliases
type Word = u32;
type Address = u32;
type RegIndex = u5;
type CacheLine = bits<512>;

var pc: Address = 0;
var rd: RegIndex = 5;
```

### Literals

```c
// Decimal
var a = 42;
var b = 1000000;

// Hexadecimal
var c = 0xFF;
var d = 0xDEAD_BEEF;    // underscores for readability

// Binary
var e = 0b1010_1100;
var f = 0b1111_0000_1111_0000;

// With explicit width
var g: u8 = 8'd255;     // Verilog-style
var h = u16(0x1234);    // Cast-style

// Boolean
var i = true;
var j = false;
```

---

## 3. Transactions

Transactions define the data structures that flow between modules over buses.

### Basic Transaction

```c
// Simple read request
transaction ReadRequest {
    addr: u32;
    len: u8;
}

// Simple read response
transaction ReadResponse {
    data: u64;
    error: bool;
}
```

### Transaction with All Field Types

```c
transaction MemoryCommand {
    // Address and data
    addr: u32;
    data: u64;

    // Control flags
    write: bool;
    byte_enable: u8;

    // Enum field
    burst_type: enum { Single, Incr, Wrap };

    // Sized field
    burst_len: u4;

    // Nested struct
    cache_hints: struct {
        cacheable: bool;
        allocate: bool;
        bufferable: bool;
    };
}
```

### Nested Transactions

```c
transaction BaseRequest {
    addr: u32;
    id: u8;
}

transaction ExtendedRequest {
    base: BaseRequest;      // nested transaction
    extra_data: u64;
    flags: u8;
}

// Usage
var req: ExtendedRequest;
req.base.addr = 0x1000;
req.base.id = 5;
req.extra_data = 0xDEADBEEF;
```

### Generic Transactions

```c
transaction Packet<T, const WIDTH: u32> {
    header: u32;
    payload: T;
    checksum: u<WIDTH>;
}

// Instantiation
var pkt: Packet<u64, 16>;
pkt.header = 0x12345678;
pkt.payload = 0xABCDABCDABCDABCD;
pkt.checksum = 0x1234;
```

---

## 4. Buses

Buses define communication channels between modules, wrapping transactions with handshaking.

### Simple Request-Response Bus

```c
// Define the bus with request and response
bus MemBus {
    request: MemRequest;
    response: MemResponse;
}

// Each direction has implicit valid/ready handshaking
// Master sends request when valid, slave accepts when ready
// Slave sends response when valid, master accepts when ready
```

### Multi-Channel Bus (AXI-style)

```c
bus AxiBus {
    // Write address channel
    channel aw {
        addr: u32;
        len: u8;
        size: u3;
        burst: u2;
        id: u4;
    }

    // Write data channel
    channel w {
        data: u64;
        strb: u8;
        last: bool;
    }

    // Write response channel
    channel b {
        id: u4;
        resp: u2;
    }

    // Read address channel
    channel ar {
        addr: u32;
        len: u8;
        size: u3;
        burst: u2;
        id: u4;
    }

    // Read data channel
    channel r {
        data: u64;
        resp: u2;
        last: bool;
        id: u4;
    }
}
```

### Tagged Channels (Out-of-Order)

```c
bus TaggedBus {
    channel request {
        id: u4;
        addr: u32;
        data: u64;
    }

    // Responses can arrive out of order, matched by id
    channel response tagged by id {
        id: u4;
        data: u64;
        error: bool;
    }
}
```

### Stream Bus (No Response)

```c
// Unidirectional streaming
bus DataStream {
    channel data {
        payload: u64;
        last: bool;
    }
}

// Master pushes data, slave consumes
// Just valid/ready handshaking, no response
```

### Bus with Sideband Signals

```c
bus MemBusWithSideband {
    request: MemRequest;
    response: MemResponse;

    // Sideband signals (directly connected, no handshaking)
    signal interrupt: bool;
    signal error_code: u4;
}
```

---

## 5. Modules

Modules are the primary building blocks containing state, ports, and behavior.

### Basic Module Structure

```c
module Counter {
    // Ports - external connections
    port ctrl: ControlBus.slave;
    port status: StatusBus.master;

    // Parameters (set at instantiation)
    const WIDTH: u32 = 32;
    const MAX_VALUE: u32 = 0xFFFFFFFF;

    // State - becomes registers
    var count: u32 = 0;
    var running: bool = false;
    var overflow: bool = false;

    // Combinational signals
    wire next_count = count + 1;
    wire at_max = count == MAX_VALUE;

    // Main sequence (behavior)
    sequence main {
        loop {
            var cmd = await ctrl.request;

            match (cmd.op) {
                Start => { running = true; }
                Stop => { running = false; }
                Reset => { count = 0; overflow = false; }
                Read => {
                    ctrl.response <- { .value = count, .overflow = overflow };
                }
            }
        }
    }

    // Secondary sequence for counting
    sequence counter_tick {
        loop {
            await cycle;
            if (running && !at_max) {
                count = next_count;
            } else if (running && at_max) {
                overflow = true;
            }
        }
    }
}
```

### Module with I/O Pins

```c
module UartTx {
    port host: UartBus.slave;

    // Direct I/O pins
    output tx: u1 = 1;      // output with default value
    input  cts: u1;         // input (directly sampled)

    // Active-low output
    output tx_n: u1 = 1;

    var shift_reg: u8;
    var bit_count: u4;

    sequence main {
        loop {
            var data = await host.command;

            // Wait for clear-to-send
            await cts == 1;

            // Send start bit
            tx = 0;
            await cycles(BAUD_TICKS);

            // Send data bits
            shift_reg = data.byte;
            for (bit_count in 0..8) {
                tx = shift_reg[0];
                shift_reg = shift_reg >> 1;
                await cycles(BAUD_TICKS);
            }

            // Send stop bit
            tx = 1;
            await cycles(BAUD_TICKS);

            host.response <- { .done = true };
        }
    }
}
```

### Module with Multiple Port Types

```c
module DmaController {
    // Slave port - receives commands
    port config: ConfigBus.slave;

    // Master port - issues memory transactions
    port mem: MemBus.master;

    // Stream ports
    port data_in: DataStream.slave;
    port data_out: DataStream.master;

    // Interrupt output (directly connected signal)
    output irq: bool = false;

    var src_addr: u32;
    var dst_addr: u32;
    var length: u32;
    var busy: bool = false;

    sequence main {
        loop {
            // Wait for configuration
            var cmd = await config.request;
            src_addr = cmd.src;
            dst_addr = cmd.dst;
            length = cmd.len;
            busy = true;

            // Perform transfer
            for (i in 0..length) {
                // Read from source
                mem.request <- { .addr = src_addr + i * 4, .write = false };
                var resp = await mem.response;

                // Write to destination
                mem.request <- { .addr = dst_addr + i * 4, .data = resp.data, .write = true };
                await mem.response;
            }

            busy = false;
            irq = true;

            config.response <- { .done = true };

            await cycle;
            irq = false;
        }
    }
}
```

---

## 6. Sequences

Sequences define the sequential behavior of a module over multiple clock cycles.

### Basic Sequence

```c
sequence main {
    // Initialization (runs once after reset)
    state = Idle;
    counter = 0;

    // Main loop
    loop {
        // Wait for something
        var req = await port.request;

        // Process
        result = compute(req.data);

        // Respond
        port.response <- { .data = result };
    }
}
```

### Sequence with State Machine

```c
sequence protocol_handler {
    loop {
        // State: Idle
        var cmd = await port.command;

        // State: Processing
        state = Processing;

        match (cmd.type) {
            Read => {
                // Multi-cycle read operation
                mem.request <- { .addr = cmd.addr, .write = false };
                var data = await mem.response;
                port.result <- { .data = data.value };
            }

            Write => {
                // Multi-cycle write operation
                mem.request <- { .addr = cmd.addr, .data = cmd.data, .write = true };
                await mem.response;
                port.result <- { .success = true };
            }

            Burst => {
                // Multi-beat burst
                for (i in 0..cmd.len) {
                    mem.request <- { .addr = cmd.addr + i * 4, .write = false };
                    var resp = await mem.response;
                    port.data <- { .value = resp.data, .last = (i == cmd.len - 1) };
                }
            }
        }

        // State: Idle
        state = Idle;
    }
}
```

### Sequence with Initialization

```c
sequence main {
    // One-time initialization
    for (i in 0..32) {
        regfile[i] = 0;
    }
    regfile[2] = STACK_POINTER;  // SP
    pc = RESET_VECTOR;

    // Main execution loop
    loop {
        // Fetch
        var instr = fetch(pc);

        // Decode & Execute
        execute(instr);

        // Update PC
        pc = next_pc;
    }
}
```

### Multiple Sequences (Single Module)

Note: TxHDL allows only one `sequence main` per module, but you can have helper sequences that are called.

```c
module Example {
    sequence main {
        loop {
            select {
                req = await port_a.request => {
                    handle_a(req);
                }
                req = await port_b.request => {
                    handle_b(req);
                }
            }
        }
    }

    // Helper procedure (not a parallel sequence)
    proc handle_a(req: RequestA) {
        // Process request A
        state = Processing;
        await cycles(10);
        port_a.response <- { .done = true };
        state = Idle;
    }

    proc handle_b(req: RequestB) {
        // Process request B
        result = compute(req.data);
        port_b.response <- { .result = result };
    }
}
```

---

## 7. Await Semantics

The `await` keyword blocks sequence execution until a condition is met.

### Await Transaction

```c
// Block until transaction arrives and handshake completes
var req = await port.request;

// Type is inferred from the channel
// Blocks until: port.request.valid && port.request.ready
```

### Await Condition

```c
// Block until boolean expression is true
await counter == 0;

// Block until flag is set
await ready;

// Complex condition
await (state == Idle) && (fifo_count > 0);
```

### Await Cycle

```c
// Wait exactly one clock cycle
await cycle;

// Wait N clock cycles
await cycles(10);

// Wait variable number of cycles
await cycles(delay_count);
```

### Await with Timeout

```c
// Wait for transaction with timeout
var resp = await port.response timeout 100 else {
    // This block executes if 100 cycles pass without response
    error_flag = true;
    // Must provide default value
    { .data = 0, .error = true }
};

// Timeout on condition
await data_valid timeout 50 else {
    handle_timeout();
};
```

### Select (Await Multiple)

```c
// Wait for first of multiple events
select {
    req = await port_a.request => {
        // Handle port A
        process_a(req);
    }
    req = await port_b.request => {
        // Handle port B
        process_b(req);
    }
    await timer_expired => {
        // Handle timeout
        handle_timeout();
    }
}

// With priority (first clause has highest priority)
select priority {
    await interrupt => { handle_interrupt(); }
    req = await normal_request => { handle_normal(req); }
}
```

### Await with Guard

```c
// Only await if guard condition is true
select {
    req = await port.request when enabled => {
        process(req);
    }
    await cycle when !enabled => {
        // Do nothing when disabled
    }
}
```

---

## 8. Bus Operations

### Send Transaction (Blocking)

```c
// Send and block until accepted (valid & ready handshake)
port.request <- { .addr = 0x1000, .data = value, .write = true };

// Execution continues only after handshake completes
```

### Send Transaction (Non-blocking)

```c
// Try to send, returns success/failure
if (port.request <-? { .addr = 0x1000 }) {
    // Transaction was accepted
} else {
    // Receiver was not ready, try again later
}
```

### Send with Callback (Tagged Channels)

```c
// Send and register completion handler
port.request <- { .id = tag, .addr = 0x1000 } then (resp) {
    // This executes when matching response arrives
    results[tag] = resp.data;
    done_count = done_count + 1;
};

// Execution continues immediately (non-blocking)
// Handler fires later when response arrives
```

### Check Channel Status

```c
// Check if transaction is pending
if (port.request.valid) {
    // Someone is trying to send us a request
}

// Check if ready to accept
if (port.response.ready) {
    // Receiver is ready for our response
}
```

### Peek Without Consuming

```c
// Look at pending transaction without accepting it
var req = port.request.peek;
if (req.write) {
    // It's a write, prepare for it
}

// Later, actually accept it
var actual_req = await port.request;
```

### Structured Field Assignment

```c
// Full field specification
port.request <- {
    .addr = base + offset,
    .data = value,
    .write = true,
    .byte_en = 0xFF,
    .burst = false
};

// Partial fields (others default to zero/false)
port.request <- { .addr = 0x100 };

// Copy and modify
var req = template_request;
req.addr = new_addr;
port.request <- req;
```

---

## 9. Pipelines

Pipelines define multi-stage operations with automatic register insertion.

### Basic Pipeline

```c
module PipelinedAdder {
    port input: DataBus.slave;
    port output: DataBus.master;

    pipeline add_pipeline(a: u32, b: u32) -> u32 {
        stage s1 {
            // Stage 1: Prepare operands
            var a_extended = zext<u64>(a);
            var b_extended = zext<u64>(b);
        }

        stage s2 {
            // Stage 2: Perform addition
            var sum = a_extended + b_extended;
        }

        stage s3 {
            // Stage 3: Truncate and return
            return sum[31:0];
        }
    }

    sequence main {
        loop {
            var req = await input.request;
            var result = add_pipeline(req.a, req.b);
            output.response <- { .sum = result };
        }
    }
}
```

### Pipeline with Data Hazards

```c
module AluPipeline {
    pipeline execute(op: AluOp, a: u32, b: u32) -> u32 {
        stage decode {
            var operation = op;
            var operand_a = a;
            var operand_b = b;
        }

        stage execute {
            var result = match (operation) {
                Add => operand_a + operand_b,
                Sub => operand_a - operand_b,
                And => operand_a & operand_b,
                Or  => operand_a | operand_b,
                Xor => operand_a ^ operand_b,
                Slt => (i32(operand_a) < i32(operand_b)) ? 1 : 0,
                Sll => operand_a << operand_b[4:0],
                Srl => operand_a >> operand_b[4:0]
            };
        }

        stage writeback {
            return result;
        }
    }
}
```

### Pipeline with Stall

```c
pipeline mem_access(addr: u32, write: bool, data: u32) -> u32 {
    stage address {
        var phys_addr = translate(addr);
        var is_write = write;
        var write_data = data;
    }

    stage cache_lookup {
        var hit = cache_check(phys_addr);
        var cache_data = cache_read(phys_addr);

        // Stall if cache miss
        if (!hit) {
            stall;  // Hold this stage until resolved
            await cache_fill_complete;
            cache_data = cache_read(phys_addr);
        }
    }

    stage complete {
        if (is_write) {
            cache_write(phys_addr, write_data);
            return 0;
        } else {
            return cache_data;
        }
    }
}
```

### Pipeline with Bypass/Forwarding

```c
module ExecuteUnit {
    // Forward declaration of bypass signals
    var ex_result: u32;
    var ex_rd: u5;
    var ex_valid: bool;

    var mem_result: u32;
    var mem_rd: u5;
    var mem_valid: bool;

    pipeline execute(instr: Instruction) -> Result {
        stage decode {
            var rs1 = instr.rs1;
            var rs2 = instr.rs2;
            var rd = instr.rd;

            // Read with forwarding
            var a = forward_or_read(rs1, ex_rd, ex_result, ex_valid,
                                         mem_rd, mem_result, mem_valid);
            var b = forward_or_read(rs2, ex_rd, ex_result, ex_valid,
                                         mem_rd, mem_result, mem_valid);
        }

        stage execute {
            var result = alu(instr.op, a, b);

            // Expose for forwarding
            ex_result = result;
            ex_rd = rd;
            ex_valid = instr.writes_reg;
        }

        stage memory {
            mem_result = ex_result;
            mem_rd = rd;
            mem_valid = instr.writes_reg;

            if (instr.is_load) {
                mem_result = load_memory(ex_result);
            }
        }

        stage writeback {
            return { .rd = rd, .value = mem_result, .valid = mem_valid };
        }
    }
}
```

---

## 10. Combinational Logic

### Wire Declarations

```c
module Example {
    var counter: u8;
    var enable: bool;

    // Simple combinational signals
    wire next_counter = counter + 1;
    wire is_max = counter == 255;
    wire should_wrap = is_max && enable;

    // Conditional
    wire output_value = enable ? counter : 0;

    // Complex expression
    wire computed = ((a & b) | (c ^ d)) + offset;
}
```

### Combinational Blocks

```c
module Decoder {
    var opcode: u6;

    // Multi-statement combinational logic
    comb control_signals: ControlBundle {
        var signals: ControlBundle;

        signals.alu_op = match (opcode) {
            0x00 => AluAdd,
            0x01 => AluSub,
            _ => AluNop
        };

        signals.reg_write = opcode != 0x2B;  // not SW
        signals.mem_read = opcode == 0x23;   // LW
        signals.mem_write = opcode == 0x2B;  // SW
        signals.branch = opcode == 0x04;     // BEQ

        return signals;
    }
}
```

### Always Block (Clocked Combinational)

```c
module Example {
    var counter: u8;
    var flag: bool;

    // Runs every clock, updates state
    always {
        if (enable) {
            if (counter < max_value) {
                counter = counter + 1;
            } else {
                counter = 0;
                flag = true;
            }
        }
    }
}
```

### Lookup Tables

```c
module SineLUT {
    // ROM-style lookup
    const SINE_TABLE: u8[256] = [
        0x80, 0x83, 0x86, 0x89, // ... full table
    ];

    wire sine_out = SINE_TABLE[phase];
}
```

---

## 11. Clocking and Reset

### Implicit Clock and Reset

```c
module Basic {
    // Clock and reset are implicit
    // - clk: rising-edge triggered
    // - rst: active-high synchronous reset

    var counter: u8 = 0;  // Resets to 0

    sequence main {
        // After reset, sequence starts here
        loop {
            await cycle;
            counter = counter + 1;
        }
    }
}
```

### Explicit Clock Reference

```c
module WithClock {
    // Reference the implicit clock
    clock clk;
    reset rst;

    // Can be used in expressions
    wire clock_active = clk;

    // Reset value comes from initializer
    var state: State = Idle;  // Resets to Idle
}
```

### Reset Behavior

```c
module ResetExample {
    // All var declarations reset to their initializers
    var counter: u8 = 0;
    var state: State = Idle;
    var buffer: u32[4] = [0, 0, 0, 0];  // All zeros

    // Sequences restart from beginning on reset
    sequence main {
        // This is the reset entry point
        initialization_done = false;

        // Perform initialization
        for (i in 0..4) {
            buffer[i] = default_values[i];
        }
        initialization_done = true;

        // Main loop
        loop {
            // ...
        }
    }
}
```

### Clock Domain Specification (in Top)

```c
module Top {
    instance fast_unit: FastModule;
    instance slow_unit: SlowModule;

    // Assign clock domains
    domain main_clock(100MHz) {
        fast_unit
    }

    domain slow_clock(10MHz) {
        slow_unit
    }

    // CDC handled automatically at bus boundaries
    connect fast_unit.output -> slow_unit.input;
}
```

---

## 12. Generic Modules

### Type Parameters

```c
module Fifo<T> {
    port input: bus { data: T }.slave;
    port output: bus { data: T }.master;

    var buffer: T[16];
    // ...
}

// Instantiation
instance byte_fifo: Fifo<u8>;
instance word_fifo: Fifo<u32>;
instance packet_fifo: Fifo<Packet>;
```

### Const Parameters

```c
module Fifo<T, const DEPTH: u32> {
    port input: bus { data: T }.slave;
    port output: bus { data: T }.master;

    var buffer: T[DEPTH];
    var head: u<$clog2(DEPTH)> = 0;
    var tail: u<$clog2(DEPTH)> = 0;
    var count: u<$clog2(DEPTH+1)> = 0;

    wire full = count == DEPTH;
    wire empty = count == 0;

    // ...
}

// Instantiation
instance small_fifo: Fifo<u32, 8>;
instance large_fifo: Fifo<u64, 1024>;
```

### Default Parameters

```c
module Memory<
    T = u32,
    const DEPTH: u32 = 1024,
    const LATENCY: u32 = 1
> {
    port bus: MemBus<T>.slave;

    var mem: T[DEPTH];

    sequence main {
        loop {
            var req = await bus.request;

            // Configurable latency
            if (LATENCY > 0) {
                await cycles(LATENCY);
            }

            if (req.write) {
                mem[req.addr] = req.data;
            }
            bus.response <- { .data = mem[req.addr] };
        }
    }
}

// Use defaults
instance mem1: Memory;  // Memory<u32, 1024, 1>

// Override some
instance mem2: Memory<u64>;  // Memory<u64, 1024, 1>
instance mem3: Memory<u32, 4096, 2>;
```

### Computed Constants

```c
module Example<const WIDTH: u32> {
    // Derived constants
    const HALF_WIDTH: u32 = WIDTH / 2;
    const ADDR_BITS: u32 = $clog2(WIDTH);
    const MASK: u<WIDTH> = (1 << WIDTH) - 1;

    var data: u<WIDTH>;
    var addr: u<ADDR_BITS>;
}
```

---

## 13. Module Instantiation

### Basic Instantiation

```c
module Top {
    // Simple instances
    instance cpu: CPU;
    instance mem: Memory;
    instance uart: UartController;
}
```

### Instantiation with Parameters

```c
module Top {
    // With generic parameters
    instance fifo: Fifo<u64, 32>;
    instance mem: Memory<DEPTH = 4096, LATENCY = 2>;

    // Named parameters
    instance cache: Cache<
        LINE_SIZE = 64,
        NUM_WAYS = 4,
        NUM_SETS = 256
    >;
}
```

### Port Connections

```c
module Top {
    instance producer: DataProducer;
    instance consumer: DataConsumer;

    // Direct connection
    connect producer.output -> consumer.input;

    // Multiple connections
    instance arbiter: Arbiter;
    connect producer.request -> arbiter.port_a;
    connect consumer.request -> arbiter.port_b;
}
```

### Bus with Address Decoding

```c
module Top {
    instance cpu: CPU;
    instance ram: Memory<DEPTH = 16384>;
    instance rom: ROM<DEPTH = 4096>;
    instance uart: UartController;
    instance gpio: GpioController;

    // Bus with address-based routing
    bus main_bus: MemBus {
        masters: [cpu.mem_port],
        slaves: [
            0x0000_0000..0x0000_3FFF => ram.port,    // 16KB RAM
            0x0001_0000..0x0001_0FFF => rom.port,    // 4KB ROM
            0x4000_0000..0x4000_000F => uart.port,   // UART
            0x4000_0100..0x4000_010F => gpio.port    // GPIO
        ]
    }
}
```

### Array of Instances

```c
module Top {
    // Array of identical modules
    instance pe[16]: ProcessingElement;

    // Connect in a ring
    for (i in 0..16) {
        connect pe[i].output -> pe[(i + 1) % 16].input;
    }
}
```

---

## 14. Structural Typing

TxHDL uses structural typing for transactions - compatibility is based on field matching, not type names.

### Compatible Transactions

```c
// These have identical structure
transaction CpuRequest {
    addr: u32;
    data: u64;
    write: bool;
}

transaction DmaRequest {
    addr: u32;
    data: u64;
    write: bool;
}

// This bus accepts any structurally compatible transaction
bus GenericBus {
    request: { addr: u32; data: u64; write: bool; };
    response: { data: u64; };
}

module Arbiter {
    port cpu: GenericBus.slave;
    port dma: GenericBus.slave;
    port mem: GenericBus.master;

    sequence main {
        loop {
            select {
                // CpuRequest is compatible
                req = await cpu.request => {
                    mem.request <- req;  // Forward directly
                    var resp = await mem.response;
                    cpu.response <- resp;
                }
                // DmaRequest is also compatible
                req = await dma.request => {
                    mem.request <- req;
                    var resp = await mem.response;
                    dma.response <- resp;
                }
            }
        }
    }
}
```

### Subtype Compatibility

```c
// Extended transaction (superset of fields)
transaction ExtendedRequest {
    addr: u32;
    data: u64;
    write: bool;
    priority: u2;    // extra field
    cache_hint: u4;  // extra field
}

// Can be sent to port expecting base fields
// Extra fields are ignored by receiver
port.request <- extended_req;  // OK if port expects { addr, data, write }
```

### Anonymous Struct Types

```c
// Inline struct definitions
bus SimpleBus {
    request: { addr: u32; data: u32; };
    response: { data: u32; error: bool; };
}

// Module can use any compatible transaction type
module Handler {
    port bus: SimpleBus.slave;

    sequence main {
        loop {
            var req = await bus.request;
            // req has .addr and .data fields
            bus.response <- { .data = process(req.data), .error = false };
        }
    }
}
```

---

## 15. Control Flow

### If-Else

```c
if (condition) {
    // then branch
} else if (other_condition) {
    // else-if branch
} else {
    // else branch
}

// Single-line
if (flag) do_something();

// Conditional expression
var result = condition ? value_if_true : value_if_false;
```

### Match (Exhaustive Switch)

```c
// Match on enum
match (state) {
    Idle => {
        // handle idle
    }
    Running => {
        // handle running
    }
    Paused => {
        // handle paused
    }
    Done => {
        // handle done
    }
}

// Match with default
match (opcode) {
    ADD => result = a + b,
    SUB => result = a - b,
    _ => result = 0  // default case
}

// Match expression (returns value)
var alu_result = match (op) {
    Add => a + b,
    Sub => a - b,
    And => a & b,
    Or => a | b,
    _ => 0
};
```

### Loops

```c
// Infinite loop
loop {
    // runs forever (or until break)
}

// While loop
while (condition) {
    // body
}

// Counted for loop
for (i in 0..10) {
    // i goes 0, 1, 2, ..., 9
}

// Inclusive range
for (i in 0..=10) {
    // i goes 0, 1, 2, ..., 10
}

// Step
for (i in 0..100 step 4) {
    // i goes 0, 4, 8, 12, ...
}

// Reverse
for (i in 10..0 step -1) {
    // i goes 10, 9, 8, ..., 1
}

// Iterate array
for (item in array) {
    process(item);
}

// Iterate with index
for (i, item in array) {
    buffer[i] = transform(item);
}
```

### Loop Control

```c
loop {
    if (done) break;        // exit loop
    if (skip) continue;     // next iteration

    // process
}

// Labeled loops (for nested break/continue)
outer: for (i in 0..10) {
    for (j in 0..10) {
        if (condition) break outer;  // break outer loop
    }
}
```

### Return

```c
sequence main {
    // Early return from sequence (unusual, restarts from beginning)
    if (error) return;

    loop {
        // normal processing
    }
}

// Return from function
func compute(x: u32) -> u32 {
    if (x == 0) return 0;
    return x * 2;
}
```

---

## 16. Bit Manipulation

### Bit Slicing

```c
var word: u32;

// Extract bits [high:low] (inclusive, descending)
var byte0: u8 = word[7:0];    // bits 7 down to 0
var byte1: u8 = word[15:8];   // bits 15 down to 8
var nibble: u4 = word[31:28]; // top nibble

// Single bit
var lsb: u1 = word[0];
var msb: u1 = word[31];
var flag: bool = word[7];  // bool-compatible
```

### Bit Assignment

```c
var word: u32 = 0;

// Assign to bit slice
word[7:0] = 0xFF;
word[15:8] = byte_value;
word[31] = 1;  // set MSB
```

### Concatenation

```c
var a: u8 = 0xAB;
var b: u8 = 0xCD;

// Concatenate (left is MSB)
var combined: u16 = { a, b };  // 0xABCD

// Multiple parts
var word: u32 = { a, b, a, b };  // 0xABCDABCD

// With literals
var extended: u16 = { 8'b0, a };  // zero-extend
```

### Replication

```c
var byte_val: u8 = 0xAA;

// Replicate N times
var replicated: u32 = {4{byte_val}};  // 0xAAAAAAAA

// Useful for sign extension
var sign_bit: u1 = value[15];
var extended: u32 = { {16{sign_bit}}, value };
```

### Extension

```c
var byte_val: u8 = 0xFF;  // -1 if signed

// Zero extension
var zero_ext: u32 = zext(byte_val);  // 0x000000FF

// Sign extension
var sign_ext: i32 = sext(byte_val);  // 0xFFFFFFFF (-1)

// Explicit width
var extended: u64 = zext<u64>(byte_val);
```

### Bitwise Operations

```c
var a: u32 = 0xF0F0_F0F0;
var b: u32 = 0x0F0F_0F0F;

var and_result = a & b;   // 0x00000000
var or_result = a | b;    // 0xFFFFFFFF
var xor_result = a ^ b;   // 0xFFFFFFFF
var not_result = ~a;      // 0x0F0F0F0F

// Shifts
var shl = a << 4;         // logical shift left
var shr = a >> 4;         // logical shift right
var sar = i32(a) >> 4;    // arithmetic shift right (sign-extend)

// Rotate
var rol = (a << 4) | (a >> 28);  // rotate left
var ror = (a >> 4) | (a << 28);  // rotate right
```

### Reduction Operations

```c
var word: u8 = 0b10110100;

var any_set = |word;      // OR reduction: 1 (any bit set)
var all_set = &word;      // AND reduction: 0 (not all bits set)
var parity = ^word;       // XOR reduction: 0 (even parity)

// Count ones (popcount)
var ones = popcount(word);  // 4

// Find first set
var first = ffs(word);      // 2 (bit index of lowest set bit)

// Count leading/trailing zeros
var clz_val = clz(word);    // count leading zeros
var ctz_val = ctz(word);    // count trailing zeros
```

---

## 17. Tagged Channels and Callbacks

### Tagged Channel Declaration

```c
transaction Request {
    id: u4;
    addr: u32;
    data: u64;
}

transaction Response {
    id: u4;
    data: u64;
    error: bool;
}

bus TaggedBus {
    channel request: Request;
    channel response: Response tagged by id;  // out-of-order matching
}
```

### The `then` Keyword

```c
module Master {
    port bus: TaggedBus.master;

    sequence main {
        var results: u64[4];
        var done: u4 = 0;

        // Issue requests with completion callbacks
        bus.request <- { .id = 0, .addr = 0x1000 } then (resp) {
            results[0] = resp.data;
            done = done + 1;
        };

        bus.request <- { .id = 1, .addr = 0x2000 } then (resp) {
            results[1] = resp.data;
            done = done + 1;
        };

        bus.request <- { .id = 2, .addr = 0x3000 } then (resp) {
            results[2] = resp.data;
            done = done + 1;
        };

        bus.request <- { .id = 3, .addr = 0x4000 } then (resp) {
            results[3] = resp.data;
            done = done + 1;
        };

        // Execution continues immediately
        // Callbacks fire as responses arrive (any order)

        // Sync point
        await done == 4;

        var sum = results[0] + results[1] + results[2] + results[3];
    }
}
```

### await_all Barrier

```c
sequence main {
    // Issue multiple requests
    for (i in 0..8) {
        bus.request <- { .id = i, .addr = base + i * 8 } then (r) {
            buffer[r.id] = r.data;
        };
    }

    // Wait for all callbacks to complete
    await_all;

    // All data now in buffer
}
```

### Named Handlers

```c
module Master {
    var cache: u64[16];

    handler fill_cache(resp: Response) {
        cache[resp.id] = resp.data;
    }

    sequence main {
        for (i in 0..16) {
            bus.request <- { .id = i, .addr = base + i * 8 } then fill_cache;
        }

        await_all;
    }
}
```

---

## 18. Functions and Procedures

### Pure Functions

```c
// Pure function - combinational logic, no side effects
func add(a: u32, b: u32) -> u32 {
    return a + b;
}

func max(a: u32, b: u32) -> u32 {
    return (a > b) ? a : b;
}

func decode_opcode(instr: u32) -> Opcode {
    return Opcode(instr[31:26]);
}

// Usage in expressions
wire sum = add(x, y);
var op = decode_opcode(instruction);
```

### Functions with Complex Logic

```c
func priority_encode(bits: u8) -> u3 {
    for (i in 7..=0 step -1) {
        if (bits[i]) {
            return i;
        }
    }
    return 0;
}

func count_ones(val: u32) -> u6 {
    var count: u6 = 0;
    for (i in 0..32) {
        if (val[i]) {
            count = count + 1;
        }
    }
    return count;
}
```

### Procedures (Can Have Side Effects)

```c
module Example {
    var state: State;
    var counter: u32;

    // Procedure - can modify module state
    proc reset_state() {
        state = Idle;
        counter = 0;
    }

    proc increment(amount: u32) {
        counter = counter + amount;
        if (counter > MAX) {
            counter = MAX;
            state = Overflow;
        }
    }

    sequence main {
        reset_state();

        loop {
            var cmd = await port.command;
            increment(cmd.value);
            port.response <- { .counter = counter };
        }
    }
}
```

### Procedures with Await (Async)

```c
module Example {
    port mem: MemBus.master;

    // Async procedure - can await
    proc fetch_data(addr: u32) -> u64 {
        mem.request <- { .addr = addr, .write = false };
        var resp = await mem.response;
        return resp.data;
    }

    proc store_data(addr: u32, data: u64) {
        mem.request <- { .addr = addr, .data = data, .write = true };
        await mem.response;
    }

    sequence main {
        loop {
            var a = fetch_data(0x1000);
            var b = fetch_data(0x2000);
            var sum = a + b;
            store_data(0x3000, sum);
        }
    }
}
```

---

## 19. Memory and Arrays

### Array Declaration

```c
// Fixed-size arrays
var buffer: u8[256];
var registers: u32[32];
var cache_lines: u64[8][64];  // 2D: 8 lines of 64 words

// With initialization
var lookup: u8[4] = [0, 1, 2, 3];
var zeros: u32[16] = [0; 16];  // all zeros
```

### Array Access

```c
// Read
var value = buffer[index];
var reg = registers[rs1];

// Write
buffer[index] = new_value;
registers[rd] = result;

// 2D access
var word = cache_lines[line][offset];
```

### Memory Module Pattern

```c
module Memory<T, const DEPTH: u32> {
    port bus: MemBus<T>.slave;

    var mem: T[DEPTH];

    sequence main {
        loop {
            var req = await bus.request;

            if (req.write) {
                mem[req.addr] = req.data;
                bus.response <- { .data = mem[req.addr] };
            } else {
                bus.response <- { .data = mem[req.addr] };
            }
        }
    }
}
```

### Dual-Port Memory

```c
module DualPortMemory<const DEPTH: u32> {
    port port_a: MemBus.slave;
    port port_b: MemBus.slave;

    var mem: u32[DEPTH];

    // Port A handler
    sequence handle_a {
        loop {
            var req = await port_a.request;
            if (req.write) {
                mem[req.addr] = req.data;
            }
            port_a.response <- { .data = mem[req.addr] };
        }
    }

    // Port B handler (read-only)
    sequence handle_b {
        loop {
            var req = await port_b.request;
            port_b.response <- { .data = mem[req.addr] };
        }
    }
}
```

### Register File

```c
module RegisterFile {
    port read1: bus { addr: u5; data: u32; }.slave;
    port read2: bus { addr: u5; data: u32; }.slave;
    port write: bus { addr: u5; data: u32; enable: bool; }.slave;

    var regs: u32[32];

    // R0 is hardwired to zero
    wire r0_zero = regs[0] == 0;

    // Combinational read (no cycle delay)
    comb read1_data: u32 {
        return (read1.addr == 0) ? 0 : regs[read1.addr];
    }

    comb read2_data: u32 {
        return (read2.addr == 0) ? 0 : regs[read2.addr];
    }

    // Sequential write
    always {
        if (write.enable && write.addr != 0) {
            regs[write.addr] = write.data;
        }
    }
}
```

---

## 20. Assertions and Verification

### Assertions

```c
module Example {
    var counter: u8;

    // Assertion - checked every cycle
    assert counter <= 100 : "Counter exceeded maximum";

    // Conditional assertion
    assert (state != Error) || error_handled : "Unhandled error state";
}
```

### Assumptions (For Formal Verification)

```c
module Example {
    input valid: bool;
    input data: u32;

    // Assume input constraints
    assume valid -> (data != 0) : "Valid data must be non-zero";
    assume data < 1000 : "Data within expected range";
}
```

### Cover Points

```c
module Example {
    var state: State;

    // Cover - check that this condition can be reached
    cover state == RareState : "Rare state reachable";
    cover counter == 255 : "Counter reaches maximum";
}
```

### Inline Assertions in Sequences

```c
sequence main {
    loop {
        var req = await port.request;

        // Runtime assertion
        assert req.addr < MEM_SIZE : "Address out of bounds";

        process(req);
    }
}
```

### Property Specifications

```c
module FifoVerification {
    var count: u8;
    const DEPTH: u8 = 16;

    // Safety property: count never exceeds depth
    property count_bounded {
        always (count <= DEPTH)
    }

    // Liveness property: requests eventually get responses
    property request_response {
        always (request.valid -> eventually response.valid)
    }

    // Sequence property: specific behavior pattern
    property handshake {
        always (valid && ready -> next (!valid || new_transaction))
    }
}
```

---

## Implementation Notes

### Compilation Flow

1. **Parse**: TxHDL source → AST
2. **Elaborate**: Resolve generics, instantiate modules
3. **Type Check**: Verify structural compatibility
4. **Lower**:
   - Sequences → FSMs
   - Pipelines → Registered stages
   - Await → State transitions
5. **Optimize**: Dead code elimination, FSM minimization
6. **Generate**: Output Verilog/VHDL/SystemC

### Await Implementation

```
await condition;

Becomes:
    STATE_N:
        if (condition) begin
            state <= STATE_N_PLUS_1;
        end
        // else stay in STATE_N
```

### Then Callback Implementation

```
port <- req then (resp) { handler; };

Becomes:
    - Register callback in pending table
    - Response dispatcher matches ID
    - Invokes handler when response arrives
    - Completion tracking via bitmask
```

---

## Quick Reference

### Keywords

```
module, instance, port, bus, channel, transaction
sequence, pipeline, stage, always, comb, wire
var, const, type, enum, struct
func, proc, handler
await, select, then, await_all
if, else, match, loop, while, for, in, break, continue, return
true, false
input, output, connect
assert, assume, cover, property
clock, reset, domain
tagged, by
```

### Operators

```
Arithmetic: + - * / %
Bitwise:    & | ^ ~ << >> >>>
Comparison: == != < <= > >=
Logical:    && || !
Assignment: =
Bus send:   <- <-?
Range:      .. ..=
Field:      .
Index:      []
Slice:      [high:low]
Concat:     { , }
Replicate:  {N{expr}}
Ternary:    ? :
```

### Built-in Functions

```
zext(val)       - Zero extend
sext(val)       - Sign extend
$clog2(val)     - Ceiling log base 2
popcount(val)   - Count set bits
clz(val)        - Count leading zeros
ctz(val)        - Count trailing zeros
ffs(val)        - Find first set
```
