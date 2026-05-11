# **FLHDL: A Data Flow Hardware Definition Language**

**Date:** May 11, 2026  
**Author:** Filip Filmar  
**Status:** Specification

---

## **1. Introduction**

FLHDL is a modern hardware definition language designed for the era of trillion-component systems. It moves away from the low-level wire-and-gate abstractions of traditional HDLs, focusing instead on dataflow, architectural intent, and robust reuse.

### **1.1. Design Goals (Theses)**
1.  **Parser Simplicity:** Uses an LL(1) deterministic grammar for fast, unambiguous tooling.
2.  **Explicit Reuse:** Promotes reuse through predeclared interfaces and generics.
3.  **High-Level Primitives:** Elevates pipelines and state machines to first-class citizens.
4.  **Temporal Abstraction:** Removes the manual distinction between combinatorial and sequential logic at the design level.
5.  **Multi-Facet Description:** Separates design intent from physical implementation and system configuration.
6.  **Interoperability:** Requires robust interop with conventional software languages for testing and legacy IP integration.

---

## **2. Top-Level Structure & The Three Facets**

FLHDL separates hardware development into three distinct concerns:
*   **Design Facet:** Defines logic, dataflow, and interface contracts. It is "time-less".
*   **Implementation Facet:** Maps design logic to specific technology (FPGA fabric, ASICs).
*   **Configuration Facet:** Binds specific implementations to architectural "sockets," sets generics, and manages system assembly.

An FLHDL source file is organized hierarchically into packages and imports, followed by top-level definitions that map to these facets.

### **Grammar: Top-Level Structure and Namespaces**
```ebnf
source_file ::= { package_decl | import_decl | top_level_def }

package_decl ::= "package" identifier { "." identifier } ";"
import_decl  ::= "import" identifier { "." identifier } [ "as" identifier ] ";"

top_level_def ::= interface_def
                | module_def
                | fsm_def
                | pipeline_def
                | config_def
                | proc_def
                | func_def
                | struct_def
                | enum_def
                | protocol_def
                | tag_def

config_def ::= "config" identifier "{" { bind_stmt } "}"
```

### **Example**
```flhdl
package system.core;
import vendor.alu as alu;

// --- 1. DESIGN FACET ---
module Processor {
    instance my_alu: alu.MathOps;
}

// --- 2. IMPLEMENTATION FACET ---
pipeline FastALU implements MathOps {
    // Pipeline implementation here...
}

// --- 3. CONFIGURATION FACET ---
config Build {
    bind Processor.my_alu => FastALU;
}
```

---

## **3. Interfaces, Elements, and Modports**

Interfaces act as the universal contract or "socket" that can be satisfied by any first-class citizen. Modports define the directionality of signals for specific roles, like a Master or Slave.

### **Grammar: Interfaces**
```ebnf
interface_def ::= "interface" identifier [ generic_decl ] "{" { interface_item } "}"

interface_item ::= port_decl ";"
                 | element_decl
                 | modport_def

element_decl ::= "element" identifier ":" identifier ";"

modport_def ::= "modport" identifier "{" { port_decl ";" } "}"

port_decl ::= direction identifier ":" type
direction ::= "in" | "out"

generic_decl ::= "<" identifier ":" type { "," identifier ":" type } ">"
```

### **Example**
```flhdl
interface StreamingOp <type T> {
    data_in  : T;
    data_out : T;

    modport Producer { out data_in;  in data_out; }
    modport Consumer { in  data_in; out data_out; }
}
```

---

## **4. Type System & Structures**

FLHDL uses a Rust-like type system for scalars (`u8`, `u32`, `bool`) and the unit type `()` as a Zero-Size Type for synchronization. It also supports complex, nested structures and tagged enums.

*   **Structs:** A standard product type containing named fields.
*   **Enums:** Sum types (tagged unions) where variants can optionally hold data payloads.

### **Grammar: Types**
```ebnf
struct_def ::= "struct" identifier "{" { identifier ":" type "," } "}"

enum_def ::= "enum" identifier "{" { enum_variant } "}"
enum_variant ::= "case" identifier [ "(" type ")" ] [ "=" literal ] ";"

type ::= identifier [ "<" literal { "," literal } ">" ] [ "[" slice_or_index "]" ]
```

### **Example**
```flhdl
struct Pixel {
    r: u8,
    g: u8,
    b: u8,
}

enum NetworkMessage {
    case Heartbeat;                       // Empty variant
    case Data(u32);                       // Data payload
    case Error(code: u8, fatal: bool);    // Struct payload
}

module MemoryBlocks {
    // Memory block definitions
    var regs: u32[16][16]; // Jagged Array: Independent logic
    var ram:  u32[16, 16]; // Contiguous Array: Physical RAM block
}
```

---

## **5. First-Class Hardware Primitives**

Pipelines, FSMs, Procedures, and Functions are first-class constructs that can optionally implement an interface.

*   **Pipelines:** Can use implicit `pipe` declarations for automatic live-range analysis and bridging. Stages can also bind directly to standalone procedures.
*   **FSMs:** Utilize explicit state blocks and automatic state encoding.
*   **Procs and Funcs:** `proc` handles explicit directional ports, while `func` allows multiple returns mapped to `out` ports.

### **Grammar: Primitives**
```ebnf
module_def ::= "module" identifier [ generic_decl ] [ "implements" identifier ]
               "{" { statement | instance_decl } "}"

pipeline_def ::= "pipeline" identifier [ generic_decl ]
                 [ "implements" identifier ] "{" { pipe_decl | stage_def } "}"
pipe_decl ::= "pipe" identifier ":" type ";"
stage_def ::= "stage" identifier [ "(" port_list ")" ]
              [ ":" identifier "(" bind_list ")" ] "{" { statement } "}"

fsm_def ::= "fsm" identifier [ generic_decl ] [ "implements" identifier ]
            "{" { state_def } "}"
state_def ::= "state" identifier "{" { statement } "}"

proc_def ::= [ "pub" ] "proc" identifier "(" [ port_list ] ")"
             "{" { statement } "}"
func_def ::= [ "pub" ] "func" identifier "(" [ port_list ] ")" "->" return_type
             "{" { statement } "}"

return_type ::= type | "(" type { "," type } ")"
port_list ::= port_decl { "," port_decl }
bind_list ::= identifier "=>" identifier { "," identifier "=>" identifier }
instance_decl ::= identifier ":" identifier "(" bind_list ")" ";"
```

### **Example**
```flhdl
proc Multiplier(in a: u32, in b: u32, out res: u64) {
    res := a * b;
}

pipeline Arithmetic {
    pipe temp: u32; // Persists across stages
    
    stage Compute : Multiplier(a => in_a, b => in_b, res => temp);
    
    stage Output { 
        out_res := temp + offset; 
    }
}

fsm Controller {
    state Idle {
        if (start) { next => Run; }
    }
    state Run {
        if (done) { next => Idle; }
    }
}
```

---

## **6. Tagged Dataflow and Protocols**

FLHDL uses **Tags** to manage synchronization without manual cycle-counting. Tags manage synchronization domains and backpressure across latency-insensitive pipelines.

### **Grammar: Tags and Protocols**
```ebnf
protocol_def ::= "protocol" identifier "{" { protocol_signal } "}"
protocol_signal ::= ( "forward" | "backward" ) port_decl ";"

tag_def ::= "tag" identifier ":" type [ "with" identifier ]
            [ "capacity" "=" literal ] ";"
tag_prefix ::= "@" identifier | "~"
```

### **Example**
```flhdl
// Defines a custom handshake protocol
protocol CreditBased {
    forward  data: u32;
    backward credit: u8;
}

// Binds the protocol to an elastic synchronization domain
tag @CreditDomain with CreditBased;

pipeline PCIeLink {
    @CreditDomain stage Transmit {
        // Compiler automatically injects credit counters and elastic buffers
        link_data := packet;
    }
}
```

---

## **7. Statements and Control Flow**

Assignments use `:=`, while structural bindings and state transitions use `=>`.
*   **Conditionals:** `if` and `match` represent combinational routing (multiplexers/decoders).
*   **Parallelism:** The `fork`/`join` block manages parallel execution branches, where the compiler re-aligns latency at the join node.

### **Grammar: Statements**
```ebnf
statement ::= var_decl
            | assign_stmt
            | bind_stmt
            | next_stmt
            | if_stmt
            | match_stmt
            | fork_stmt
            | return_stmt

var_decl ::= ( "var" | "wire" ) identifier ":" type ";"
assign_stmt ::= lvalue ":=" [ tag_prefix ] expression ";"
bind_stmt ::= identifier "=>" expression ";"
next_stmt ::= "next" "=>" identifier ";"
return_stmt ::= "return" expression ";"

if_stmt ::= "if" expression "{" { statement } "}" [ "else" "{" { statement } "}" ]

match_stmt ::= "match" [ tag_prefix ] expression
               "{" { match_arm } [ "default" ":" { statement } ] "}"
match_arm ::= "case" pattern ":" { statement }
pattern ::= literal | bit_mask | identifier

fork_stmt ::= "fork" "{" { "branch" identifier "{" { statement } "}" } "join"
              [ tag_prefix ] [ "(" bind_list ")" ] ";"
```

### **Example**
```flhdl
fork {
    branch A { var r1 := @Sync path_a(data); }
    branch B { var r2 := @Sync path_b(data); }
} join @Sync ( result => r1 + r2 );
```

---

## **8. Expressions, Ranges, and LValues**

FLHDL supports standard expressions but extends left-values (LValues) and arrays with domain-specific slicing and bit-ordering.

*   **Multidimensional Indices:** Uses comma-separated expressions (e.g., `a[1, 2]`).
*   **Ranges:** Includes ascending half-open `..`, ascending inclusive `..=`, directional descending `<-` (hardware default), and directional ascending `->`.

### **Grammar: Expressions**
```ebnf
lvalue ::= identifier { "." identifier | "[" slice_or_index "]" }

slice_or_index ::= expression [ ( "," expression ) | ( range_op expression ) ]
range_op ::= ".." | "..=" | "<-" | "->"

expression ::= term { binary_op term }
term ::= literal | lvalue | "(" expression ")" | function_call
function_call ::= identifier "(" [ expression { "," expression } ] ")"

binary_op ::= "+" | "-" | "*" | "/" | "&" | "|" | "^" | "==" | "!="
            | "<" | ">" | "<=" | ">=" | ">>" | "<<" | "|>"
```

### **Example**
```flhdl
var instruction: u32;

var opcode     := instruction[0..8];    // Exclusive (0 to 7)
var func3      := instruction[12..=14]; // Inclusive (12 to 14)
var msb_first  := instruction[31 <- 0]; // Descending
```

---

## **9. Testing & Integration (FFI)**

FLHDL focuses on hardware; complex I/O and verification are delegated to software via a Foreign Function Interface.

*   **Black-Box Interop:** Legacy Verilog/VHDL modules are bound in the Configuration Facet.
*   **Co-Simulation via RPC:** Interfaces can be bound to `foreign` implementations (Go, Rust, C++). The compiler generates a **gRPC bridge** for cycle-accurate co-simulation.

### **Example**
```flhdl
config System {
    // Bind to Verilog Legacy IP
    bind Top.mult => blackbox("verilog", "mult.v");
    
    // Bind to Software C++ test driver via gRPC
    bind Top.test_driver => foreign("cpp", "driver.cpp");
}
```

---

## **10. Handling of Clock and Reset**

Clocks and resets are implementation details managed via Tags and Configuration. You do not route them manually in the Design Facet.

1.  **Design:** Modules are tagged (e.g., `@AxiDomain`). No clock ports are manually declared.
2.  **Configuration:** The tag is mapped to a physical clock/reset.

### **Example**
```flhdl
// 1. Design Facet
module AxiProcessor @AxiDomain (
    bus : AxiStream<32>
) {
    // Logic goes here
}

// 2. Configuration Facet
domain AxiDomain {
    clock: sys_clk_100mhz;
    reset: sys_rst_n; 
}

config TopLevelConf {
    bind System.processor => AxiProcessor;
}
```

---

## **11. Full EBNF Grammar Reference**

This section consolidates all EBNF grammar rules for FLHDL into a single reference block.

```ebnf
source_file ::= { package_decl | import_decl | top_level_def }

package_decl ::= "package" identifier { "." identifier } ";"
import_decl  ::= "import" identifier { "." identifier } [ "as" identifier ] ";"

top_level_def ::= interface_def
                | module_def
                | fsm_def
                | pipeline_def
                | config_def
                | proc_def
                | func_def
                | struct_def
                | enum_def
                | protocol_def
                | tag_def

config_def ::= "config" identifier "{" { bind_stmt } "}"

interface_def ::= "interface" identifier [ generic_decl ] "{" { interface_item } "}"
interface_item ::= port_decl ";"
                 | element_decl
                 | modport_def

element_decl ::= "element" identifier ":" identifier ";"

modport_def ::= "modport" identifier "{" { port_decl ";" } "}"

port_decl ::= direction identifier ":" type
direction ::= "in" | "out"

generic_decl ::= "<" identifier ":" type { "," identifier ":" type } ">"

struct_def ::= "struct" identifier "{" { identifier ":" type "," } "}"

enum_def ::= "enum" identifier "{" { enum_variant } "}"
enum_variant ::= "case" identifier [ "(" type ")" ] [ "=" literal ] ";"

type ::= identifier [ "<" literal { "," literal } ">" ] [ "[" slice_or_index "]" ]

module_def ::= "module" identifier [ generic_decl ] [ "implements" identifier ]
               "{" { statement | instance_decl } "}"

pipeline_def ::= "pipeline" identifier [ generic_decl ]
                 [ "implements" identifier ] "{" { pipe_decl | stage_def } "}"
pipe_decl ::= "pipe" identifier ":" type ";"
stage_def ::= "stage" identifier [ "(" port_list ")" ]
              [ ":" identifier "(" bind_list ")" ] "{" { statement } "}"

fsm_def ::= "fsm" identifier [ generic_decl ] [ "implements" identifier ]
            "{" { state_def } "}"
state_def ::= "state" identifier "{" { statement } "}"

proc_def ::= [ "pub" ] "proc" identifier "(" [ port_list ] ")"
             "{" { statement } "}"
func_def ::= [ "pub" ] "func" identifier "(" [ port_list ] ")" "->" return_type
             "{" { statement } "}"

return_type ::= type | "(" type { "," type } ")"
port_list ::= port_decl { "," port_decl }
bind_list ::= identifier "=>" identifier { "," identifier "=>" identifier }
instance_decl ::= identifier ":" identifier "(" bind_list ")" ";"

protocol_def ::= "protocol" identifier "{" { protocol_signal } "}"
protocol_signal ::= ( "forward" | "backward" ) port_decl ";"

tag_def ::= "tag" identifier ":" type [ "with" identifier ]
            [ "capacity" "=" literal ] ";"
tag_prefix ::= "@" identifier | "~"

statement ::= var_decl
            | assign_stmt
            | bind_stmt
            | next_stmt
            | if_stmt
            | match_stmt
            | fork_stmt
            | return_stmt

var_decl ::= ( "var" | "wire" ) identifier ":" type ";"
assign_stmt ::= lvalue ":=" [ tag_prefix ] expression ";"
bind_stmt ::= identifier "=>" expression ";"
next_stmt ::= "next" "=>" identifier ";"
return_stmt ::= "return" expression ";"

if_stmt ::= "if" expression "{" { statement } "}" [ "else" "{" { statement } "}" ]

match_stmt ::= "match" [ tag_prefix ] expression
               "{" { match_arm } [ "default" ":" { statement } ] "}"
match_arm ::= "case" pattern ":" { statement }
pattern ::= literal | bit_mask | identifier

fork_stmt ::= "fork" "{" { "branch" identifier "{" { statement } "}" } "join"
              [ tag_prefix ] [ "(" bind_list ")" ] ";"

lvalue ::= identifier { "." identifier | "[" slice_or_index "]" }

slice_or_index ::= expression [ ( "," expression ) | ( range_op expression ) ]
range_op ::= ".." | "..=" | "<-" | "->"

expression ::= term { binary_op term }
term ::= literal | lvalue | "(" expression ")" | function_call
function_call ::= identifier "(" [ expression { "," expression } ] ")"

binary_op ::= "+" | "-" | "*" | "/" | "&" | "|" | "^" | "==" | "!="
            | "<" | ">" | "<=" | ">=" | ">>" | "<<" | "|>"
```
