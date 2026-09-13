<!-- SPDX-License-Identifier: Apache-2.0 -->
# **Some theses for a modern hardware HDL**

# **Some theses for a modern hardware definition language**

May 3, 2026 | [Filip Filmar](mailto:filmil@gmail.com)

# **Theses**

1. It would be nice if [the language](https://drive.google.com/file/d/1ODgHWazEVGgtS6dAJ5EH3pH4AqgXW-Pf/view?usp=drive_link) were simple to parse.  
2. It makes sense to create a new domain-specific language in 2026\.  
3. Reuse is important for modern hardware design.  
4. Ability to predeclare interfaces promotes reuse and should be present.  
5. Generics are important for reuse too.  
6. The distinction between combinatorial and sequential networks should not matter.  
7. Often used primitives, like pipelines and state machines, should be first class citizens.  
8. At the design language level, there should be no notion of combinatorial, vs. sequential logic.  
9. Possibly, there need to be multiple “facets” of a description language: a design language, an implementation language, a configuration language.  
10. It is more important for the language to support useful and often used primitives, than it is to be fully general.  
11. A realistic language requires interop with conventional languages.  
12. A realistic language requires interop with testing infrastructure.

# **Details**

## **It would be nice if the language were simple to parse**

Makes for a simple parser and tooling build. I had Gemini build an [example](https://gemini.google.com/share/98c4298a13c5).

Side note, better to have \`{}\` than \`begin .. end\`, avoids having \`begin\` and \`end\` as reserved words.

Contrast: VHDL is notoriously difficult to parse, leading to few good tools for handling it.

## **It makes sense to create a new domain-specific language in 2026**

One direction for the development of high level synthesis tools is to make them valid programs in an existing programming language. This makes sense as you can then compile and run them as regular programs, which is nice.

This avoids having to build a new toolchain for a new language. However, in 2026, with the availability of LLM powered tools, it is not that outrageous to think that working out a full toolchain with LLM assistance has become accessible to all.

## **Reuse is important for modern hardware design**

Modern hardware design means millions to billions to trillions of components. In such an environment, connecting individual wires is too low an abstraction level. You must be able to build ever larger blocks and compose them together, if you are to meet the demands of modern hardware.

One way we do this is by making reuse easy to happen. While HDLs make it easy to swap out implementations, more is needed. Here are some elements of reuse that I think are required:

* Processing Elements  
* Composable pipelines  
* Module interfaces  
* Procedures and functions  
* Composable state machines  
* Libraries and packages

## **Ability to predeclare interfaces promotes reuse and should be present**

Interfaces in HDL/HSL are much more important than in languages intended for describing software.

Every interface in HDL/HSL gets at least two implementations by default: one is the register-transfer-level implementation, which is useful for simulation and synthesis, and another is the behavioral level, which is useful for simulation. Depending on your needs, you may have more or less detailed functional simulation, as well as architectures tailor made to specific implementation fabrics.

This in turn means that having a way to communicate only an interface of a module instead of the module and its implementation, is immediately useful. Contrast that to software where you try not to abstract an implementation into an interface until you have enough implementation to justify the complexity. In software you wait until you get there. In hardware you are already there.

## **Generics are important for reuse too**

Again one area where hardware has an immediate need. The Go language famously did without generics for many years.

In hardware you may have a 8-bit, or a 16-bit,or a 32-bit, or a 64-bit Wishbone bus. There is no reason to require the interface authors to define separate modules for these. HDLs usually have these on the ready, but implementations are spotty.

## **Often used primitives, like pipelines and state machines, should be first class citizens**

We mostly only implement these anyways, so why not make making them easier.

## **At the design language level, there should be no notion of combinatorial, vs. sequential logic**

It seems to me that the combinatorial and sequential network distinction exists in design languages as an artifact of how we learned digital design. Since flip-flops are the cornerstone of stateful design, it makes sense that they exist as a concept when looking at a concrete digital design.

However, I've noticed that their importance diminishes in HDL/HLS. If you ever had to retime a design so that it makes timing, you probably noticed that save for a few lucky corner cases, you usually have to redesign it altogether. This leads me to believe that: (a) in many cases it is possible to convert a combinatorial network into an equivalent sequential network that can operate at a higher clock rate. And (b) if this is possible, then it’s also automatable.

With this in mind I think it should be possible to design a HLS language which does not use combinatorial and sequential logic as first class concepts. Instead, the statefulness of a computational network should be derived from how a HLS description is mapped to the underlying digital technology.

## **​Possibly, there need to be multiple “facets” of a description language: a design language, an implementation language, a configuration language**

## **​It is more important for the language to support useful and often used primitives, than it is to be fully general**

Instead, use interop with existing languages to supplant test only functionality.

## **​A realistic language requires interop with conventional languages**

To reuse already existing modules. Not sure how to make that happen.

## **​A realistic language requires interop with testing infrastructure**

I think it might be more interesting to provide interop with existing languages (e.g. for testing or for simulation, or verification), than to add a non-synthesizeable language subset which e.g. deals with file IO and such.

# **FLHDL: A data flow hardware definition language**

# **FLHDL: A Data Flow Hardware Definition Language**

May 7, 2026[Filip Filmar](mailto:filmil@gmail.com) Assisted by LLM.

This document is a specification for a data-flow oriented hardware definition language. It does not describe a necessarily stand-alone language, but rather one of a hierarchy of languages, each of which may have a specific focus.

The intention is to describe a language useful for defining modern, trillion-

# References

1. [Idea seed: communicating processing elements](https://docs.google.com/document/d/1kOZrQbu5O7M2gWmsZ-6DSBEYTo58eat7a9gSi-ISbGk/edit)  
2. [FLHDL proposal](https://docs.google.com/document/d/1tRt-pZpcFFNffbjPmE4Rxg0JIrNJpQ5HT_p9LvsA6LM/edit?tab=t.0)  
3. [Idea Seed: Some theses for a modern hardware definition language](https://docs.google.com/document/d/1ce-PA8tqCZH5zC162W8QeTA3u0KCgmMgopnvkKAMAL4/edit?tab=t.0#heading=h.17q7inz3kajo)

# Core Philosophy & The Three Facets

FLHDL separates the concerns of hardware development into three distinct facets to allow for architectural exploration without manual RTL rewriting.

* Design Facet: Defines the logic, dataflow, and interface contracts. At this level, there is no distinction between combinatorial and sequential logic.  
* Implementation Facet: Maps the design to specific technology. Handles retiming, clock-gating, and determines if a network is combinatorial or sequential based on timing constraints.  
* Configuration Facet: Handles the assembly of the system, binding specific implementations to interface instances and setting generic parameters.

# Syntax & Lexical Structure

FLHDL is defined by an LL(1) to ensure fast, deterministic parsing and simple tooling.

* Brace-Scoped: Uses {..} for blocks to improve scannability and reduce reserved keywords.  
* Prefix-Heavy: Every construct is announced by a unique keyword or symbol.  
* Flat Arrays: Multidimensional arrays are natively supported; the compiler handles flat bit-offset calculations.

# Interfaces: The Universal Contract

Interfaces are the primary unit of reuse. Every first-class citizen (Module, Pipeline, FSM) can implement an interface. We will see later how this is done.

| interface StreamingOp \<type T\> {     in\_data  : T;     out\_data : T;          modport Server { in in\_data; out out\_data; }     modport Client { out in\_data; in out\_data; } } |
| :---- |

### Examples

#### Wishbone interface definition

| package std.bus; // Parameterized interface for address and data widths pub interface Wishbone \<addr\_w: int, data\_w: int\> {     adr     : uint\<addr\_w\>;  // Address bus     dat\_m2s : uint\<data\_w\>;  // Data: Master to Slave     dat\_s2m : uint\<data\_w\>;  // Data: Slave to Master     we      : bool;          // Write Enable     stb     : bool;          // Strobe     cyc     : bool;          // Cycle     ack     : bool;          // Acknowledge     // Modport for the Master (initiator) role     modport Master {         out adr;         out dat\_m2s;         in  dat\_s2m;         out we;         out stb;         out cyc;         in  ack;     }     // Modport for the Slave (target) role     modport Slave {         in  adr;         in  dat\_m2s;         out dat\_s2m;         in  we;         in  stb;         in  cyc;         out ack;     } } |
| :---- |

# **Syntax & Grammar Foundations of FLHDL**

# **Syntax \&amp; Grammar Foundations of FLHDL**

The syntax and grammar of FLHDL are designed from the ground up to prioritize parsing efficiency, unambiguous code structures, and developer readability. By understanding these foundational rules, you can effectively harness the language\&apos;s core capabilities.

## **Core Grammar Design**

* **LL(1) Deterministic Grammar:** FLHDL relies on an LL(1) deterministic grammar, allowing for highly efficiency single-pass parsing that requires only one token of lookahead.  
* **Prefix-Heavy, Keyword-Driven Structure:** The language features a prefix-heavy design built around distinct keywords. Using unique identifiers such as module, struct, and fsm prevents parser backtracking and eliminates complex \&quot;First/First\&quot; conflicts.  
* **Brace-Scoped Blocks:** FLHDL uses standard curly braces ({}) for block scoping. This approach dramatically improves visual scannability and minimizes the bloat of reserved words typically needed to explicitly open and close logic blocks.

## **Unified Operator Set**

FLHDL streamlines hardware description by providing a unified, consistent set of operators tailored for both dataflow and structural composition:

| Operator | Hardware Function |
| :---: | ----- |
| `:` | Type Specification. |
| `= or :=` | Logic Assignment. |
| `=>` | Port Binding and FSM Transitions. |
| `@` | Synchronization Tags. |
| `< >` | Compile-Time Generics. |
| `|>` | Pipeline Chaining. |

## **Rustified Ranges**

To handle indexing, bit-slicing, and array traversals accurately, FLHDL implements \&quot;Rustified\&quot; ranges alongside explicit directional markers:

* **Exclusive Range (..):** Used to declare a range that excludes the upper bound.  
* **Inclusive Range (..=):** Used to declare a range that includes the upper bound.  
* **Directional Ranges (\<- and \-\>):** These operators dictate Descending (\&lt;-) and Ascending (-\&gt;) ranges, definitively resolving any bit-order ambiguity directly at the syntax level.

Here are code examples demonstrating the Syntax \&amp; Grammar Foundations of FLHDL:

### **1\. Prefix-Heavy, Keyword-Driven Design \&amp; Brace Scoping**

FLHDL avoids \&quot;First/First\&quot; parser conflicts by forcing every major structure or statement to start with a unique keyword (module, var, interface, if). It completely drops the verbose begin/end blocks in favor of clean, software-like curly braces.

**Code snippet:**

```
// 'module' and 'interface' immediately tell the 
// LL(1) parser what to expect
interface MemoryBus {
    in  addr: u32;
    out data: u32;
}

module AddressDecoder {
    // Standard brace scoping
    if (enable) {
        var local_addr := base_addr + offset;
    }
}
```

### **2\. Unified Operator Set**

FLHDL standardizes operators so they mean the same thing regardless of whether you are writing combinatorial math or wiring up high-level pipelines.

**Code snippet:**

```
// [:] Type Specification
var counter: u8;

// [:=] or [=] Logic Assignment
counter := counter + 1;

// [=>] Port Binding and FSM Transitions
// Used for mapping interfaces or jumping to a new state
bind system.cpu => FastRiscvCore;
=> State_Idle; 

// [@] Synchronization Tags
var delayed_val := @Sync(MathOp(data));

// [< >] Compile-Time Generics
interface AxiStream <DATA_WIDTH: int> { ... }

// [|>] Pipeline Chaining
// Automatically connects compatible output/input ports
out_pixels := in_pixels |> Grayscale() |> Blur();
```

### **3\. Rustified \&amp; Directional Ranges**

Handling bit-widths and array slices is a notorious source of bugs in hardware design. FLHDL uses explicit range syntax to eliminate \&quot;off-by-one\&quot; errors and bit-endianness ambiguity.

**Code snippet:**

```
var instruction: u32;

// Exclusive Range (..) - goes up to, but does not include, 8
var opcode := instruction[0..8]; 

// Inclusive Range (..=) - includes the 7th bit
var func3 := instruction[12..=14];

// Directional Ranges (<- and ->) 
// Explicitly forces the bit-ordering to avoid endianness mismatches during elaboration
var msb_first := instruction[31 <- 0]; // Descending (Hardware default)
var lsb_first := instruction[0 -> 31]; // Ascending (Software/Buffer default)
```

# **Type System & Structures**

## Type System & Structures

The FLHDL type system is engineered to provide precise hardware control while maintaining the rigorous safety guarantees found in modern software languages. It introduces a highly structured approach to data layout, memory organization, and system scaling.

### Core Data Types

To ensure consistency and clarity, FLHDL adopts modern syntax for defining both simple and complex types:

* **Rustified Scalar Nomenclature:** The language utilizes a clear, "Rustified" naming convention for scalars, specifically employing types such as u8, u32, and bool.  
* **Product & Sum Types:** FLHDL provides robust support for structs and tagged enums, which include explicit bit-mapping for precise hardware layout.  
* **Zero-Size Types (ZST):** The unit type () operates as a Zero-Size Type and is uniquely used for combinatorial path signaling rather than data storage.

### Multidimensional Arrays and Memory Management

Handling memory blocks correctly is critical for optimal synthesis. FLHDL distinguishes between different hardware memory topologies directly at the syntax level:

* **Jagged Arrays:** Modeled using the standard a\[i\]\[j\] syntax.  
* **Contiguous Arrays:** Modeled using the a\[i, j\] syntax, which explicitly dictates the inference of single physical RAM blocks.

To manage the underlying hardware addresses for these contiguous structures, FLHDL employs **Static Offset Flattening**. 

### Organizational Hierarchy

As hardware designs grow in complexity, managing the scope and visibility of modules and types becomes paramount.

* **Hierarchical Namespaces:** FLHDL organizes code using a nested, file-based package system that is explicitly built to support trillion-component scale architectures.

Here are code examples demonstrating the Type System & Structures features in FLHDL:

### 1\. Rustified Scalar Nomenclature & Zero-Size Types (ZST)

FLHDL uses clean, explicit bit-width declarations for scalars. The Zero-Size Type () is used when a signal carries no data but is needed for synchronization or control flow.

```
// Standard explicit bit-width scalars
var counter: u32 := 0;
var is_active: bool := true;
var byte_mask: u8 := 0xFF;

// Zero-Size Type: Takes up 0 physical wires in hardware.
// Used solely to trigger an event or synchronize a domain.
var start_trigger: () := button_a &amp; button_b;
```

### 2\. Product & Sum Types (Structs and Tagged Enums)

FLHDL provides software-like structs and sum types, but the compiler rigorously flattens them into bit-vectors for hardware synthesis.

```
// Product Type: A standard struct
struct Pixel {
    r: u8;
    g: u8;
    b: u8;
}

// Sum Type: A Tagged Enum
// The compiler automatically calculates the required bit-width to store 
// the tag + the largest payload (in this case, 32 bits for the Data payload).
enum NetworkMessage {
    case Heartbeat;                       // Empty variant
    case Data(u32);                       // Anonymous single payload
    case Error(code: u8, fatal: bool);    // Named struct-like payload
}

pipeline Decoder {
    stage Parse {
        var msg: NetworkMessage := fetch_msg();
        
        // Pattern matching on the hardware enum
        match (msg) {
            case Data(d): {
                out_val := d;
            }
            case Error(err): {
                // Accessing the struct-like payload fields
                err_code := err.code;
            }
        }
    }
}
```

### 3\. Multidimensional Arrays (Jagged vs. Contiguous)

The syntax explicitly dictates to the compiler how the underlying memory should be structured in physical hardware.

```
module MemoryBlocks {
    
    // Jagged Array: Synthesized as an array of independent arrays.
    // Maps to discrete, individual registers or fragmented logic.
    var lookup_table: u32[4][4];
    
    // Contiguous Array: Synthesized as a single, unified block of memory.
    // The compiler automatically infers a single RAM primitive and handles 
    // the Static Offset Flattening calculation for addresses.
    var video_ram: u32[4, 4];
    
    // Accessing the contiguous RAM (compiler calculates the flattened offset)
    var pixel := video_ram[row, col];
}
```

### 4\. Hierarchical Namespaces

Packages map to physical directories, and symbols are isolated. The dot-notation allows the LL(1) parser to traverse the "Tree of Maps" without confusion.

```
// File: vendor_a/dsp/filters.flhdl
package vendor_a.dsp.filters;

// Private by default, invisible outside this file
func add(x: u32, y: u32) -> u32 { return x + y; }

// 'pub' exposes this to other packages
pub proc MultiplyAccumulate(in a: u32, in b: u32, in c: u32, out res: u32) {
    res := add(a * b, c);
}

// ---------------------------------------------------------
// File: my_project/main.flhdl
package my_project;

// Explicit import prevents namespace pollution
import vendor_a.dsp.filters;

module Processor {
    // Calling the procedure via qualified access
    var mac_result := filters.MultiplyAccumulate(val1, val2, acc);
}
```

# **First-Class Hardware Primitives**

# **First-Class Hardware Primitives**

FLHDL accelerates the design process by elevating common hardware design patterns into native, first-class language constructs. By removing standard boilerplate, designers can focus on architectural intent rather than manual register management.

## **First-Class FSMs**

Finite State Machines (FSMs) are fundamental to digital design, and FLHDL treats them as native entities.

* **Named State Blocks:** FSMs are built using named state blocks that feature managed transitions.  
* **Automatic Encoding:** The compiler handles the automatic state encoding for these blocks, natively supporting encoding styles such as One-Hot and Gray.

## **Automated Pipelines**

Pipelining in FLHDL is heavily automated to reduce human error in signal propagation and register balancing. The language introduces specific constructs to manage data flow across clock cycles:

* **pipe Declarations:** Designers can use pipe declarations for signals, instructing the compiler to persist them across different pipeline stages.  
* **Live-Range Analysis:** The compiler performs live-range analysis to enable automated "bridge register" (passthrough) generation.  
* **Implementation by Context:** FLHDL infers the physical implementation of a signal (such as a flip-flop versus a continuous wire) based purely on its contextual scope, distinguishing between a Module and a Stage.

## **Procedures and Functions**

To promote logic reuse and clean structural composition, FLHDL implements distinct blocks for combinatorial and sequential logic operations:

* **Procedures (proc):** These serve as reusable logic blocks that are defined with explicit ports.  
* **Functions (fn):** Functions act as syntactic sugar for procedures by implicitly lowering return values to output ports.  
* **Procedure Binding:** The language allows for the binding of standalone proc logic to specific pipeline stages, which includes support for signal renaming during the bind.

In FLHDL, you can completely replace an inline stage block (using { }) with a direct Procedure Binding. This is a powerful feature that allows you to cleanly separate your combinatorial logic (the math) from your sequential structural flow (the pipeline).

Because FLHDL uses an LL(1) parser, the compiler simply looks for a colon (:) instead of an opening brace ({) after the stage name to instantly switch into "binding mode."

Here is how you directly implement the Compute stage using the MultiplyAccumulate procedure, mapping the pipeline signals to the procedure's ports via the \`=\>\` operator:

```
// 1. Define the reusable combinatorial procedure
proc MultiplyAccumulate(in a: u32, in b: u32, in c: u32, out res: u32) {
    res := (a * b) + c;
}

// 2. Define the structural pipeline
pipeline DSP {
    // Declaring the pipeline 'conveyor belt' signals
    pipe coeff: u32;
    pipe sample: u32;
    pipe acc: u32;
    pipe tap_out: u32;

    // ... previous stages (e.g., Fetching coefficients) ...

    // 3. Direct Procedure Binding
    // The ':' tells the compiler this stage is entirely fulfilled by 'MultiplyAccumulate'
    stage Compute : MultiplyAccumulate (
        a   => coeff,     // Map 'coeff' pipe to 'a' port
        b   => sample,    // Map 'sample' pipe to 'b' port
        c   => acc,       // Map 'acc' pipe to 'c' port
        res => tap_out    // Map 'res' port to 'tap_out' pipe
    );
    
    // ... subsequent stages ...
}
```

### **Why this is advantageous:**

1. **No Inline Clutter:** The stage definition acts purely as an interface map, keeping the pipeline's high-level architectural view clean.  
2. **Explicit Dataflow:** The \=\> syntax makes it visually obvious exactly how the pipeline's temporal state flows into and out of the "timeless" combinatorial procedure.  
3. **Zero Ambiguity:** The strict LL(1) parser never has to guess whether you are declaring a new local variable or calling a function; the port mapping is rigorously enforced.

# **Latency-Insensitive Dataflow (Tags)**

# Latency-Insensitive Dataflow (Tags)

Traditional hardware description languages force designers to manually manage cycle delays, insert pipeline registers, and wire up complex backpressure logic. FLHDL shifts this burden to the compiler. By leveraging an attribute system known as "Tags," designers can define logical groupings of signals and let the compiler synthesize the necessary physical synchronization.

## Synchronization Domains (@)

In FLHDL, the @ symbol is used to assign signals, procedures, or entire pipeline stages to specific **Synchronization Domains**.  
A Synchronization Domain acts as a boundary of temporal consistency. When multiple operations or signals share the same tag (e.g., @Sync), the FLHDL compiler performs Longest Path Analysis. If one path takes three clock cycles and another takes five, the compiler will automatically inject "bridge registers" into the shorter path to balance the pipeline depths. This guarantees that data arrives at the destination perfectly aligned, regardless of the underlying latency.

## Flow Policies & Elasticity

While simple delay matching is useful, real-world hardware often experiences unpredictable stalls (e.g., a cache miss or a full FIFO). FLHDL introduces Flow Policies to handle these scenarios dynamically.  
By attaching the with handshake attribute to a tag, the compiler automatically injects standard Ready/Valid flow control logic across the entire domain. To prevent combinatorial loops and maintain high throughput during stalls, the compiler replaces standard flip-flops with **Elastic Skid Buffers**. This provides the domain with elasticity—the ability to safely absorb in-flight data when a downstream module asserts a stall, ensuring no data is lost and backpressure is smoothly rippled upstream.

## Custom Handshake Protocols

While standard Ready/Valid logic is common, FLHDL treats handshakes as First-Class Protocols, allowing you to define exactly which signals participate in flow control without losing the benefits of automated latency-insensitivity.  
You can define a protocol block that separates signals into two directions:

* **Forward**: Signals that flow with the payload (e.g., req, valid). Injected into the forward pipeline path.  
* **Backward**: Signals that flow against the data to stall or acknowledge (e.g., ack, credit). Injected into the backpressure logic chain.

When a custom protocol (such as Credit-Based or Req/Ack) is bound to a tag, the compiler performs structural transformations: it substitutes the standard templates with your custom signals and injects the appropriate Credit Counters or modified buffers, automatically rippling the custom backpressure through every component in the domain.

## Resource Management

For complex domains that must track multiple in-flight transactions (such as out-of-order memory responses), FLHDL offers automated resource management.  
By applying a capacity=n attribute to a tag, you instruct the compiler to generate the necessary hardware tracking structures—such as hardware Scoreboards or Content Addressable Memories (CAMs)—to monitor up to n in-flight items. If the capacity is reached, the compiler automatically asserts backpressure to the source until a slot frees up.

## Zero-Size Types and Implicit Tagging

To keep code clean and readable, FLHDL implements features to minimize boilerplate around tagging:

* **Zero-Size Types (ZST):** Using the unit type () in conjunction with tags allows designers to explicitly define purely combinatorial control paths that carry synchronization metadata without generating physical data registers.  
* **Implicit Tagging:** FLHDL supports lexical scoping for tags. You can apply a default tag at the top of a module or block, and all un-tagged signals and stages within that lexical scope will implicitly inherit the domain.

## Code Examples

### 1\. Synchronization Domains (@)

By tagging operations with a domain like @Sync, the compiler automatically inserts pipeline registers (bridges) into shorter paths so that both a and b arrive at the final addition in the exact same clock cycle.

```
tag Sync; // Declare a basic synchronization domain

pipeline MathCore {
    stage Calculate {
        // heavy_math takes 5 cycles, fast_math takes 1 cycle.
        // The compiler automatically adds 4 cycles of delay to 'b'.
        var a := @Sync heavy_math(x, y);
        var b := @Sync fast_math(z);
        
        // Safely add them together; timing is guaranteed by the compiler
        res := @Sync a + b; 
    }
}
```

### 2\. Flow Policies & Elasticity (with handshake)

Adding the with handshake attribute transforms the basic delay-matching domain into an elastic domain. The compiler replaces standard flip-flops with Elastic Skid Buffers and automatically wires up the Ready/Valid backpressure logic.

```
// Defines an elastic domain
tag ElasticFlow with handshake;

pipeline VideoProcessor {
    // All stages in this pipeline will automatically stall safely 
    // without dropping pixels if the downstream interface asserts backpressure
    @ElasticFlow stage Grayscale {
        pipe p1 := convert_gray(pixel_in);
    }
    
    @ElasticFlow stage Blur {
        pixel_out := apply_blur(p1);
    }
}
```

### 3\. Custom Handshake Protocols

Instead of being locked into standard Ready/Valid, you can define exactly which signals flow forward (with data) and backward (against data for backpressure).

```
// 1. Define the custom physical protocol
protocol CreditBased {
    forward  req: bool;      // Flows downstream
    backward credit_out: u8; // Flows upstream to stall the source
}

// 2. Bind the protocol to a tag
tag @CreditDomain with CreditBased;

pipeline PCIeLink {
    // The compiler automatically injects Credit Counters instead of 
    // standard skid buffers for this stage.
    @CreditDomain stage Transmit {
        link_data := packet;
    }
}
```

### 4\. Resource Management (capacity=n)

When a domain needs to keep track of asynchronous or out-of-order events (like memory fetches), setting a capacity tells the compiler to generate hardware tracking structures.

```
// Instructs the compiler to generate a hardware scoreboard or CAM
// that can track up to 32 in-flight transactions.
tag @MemFetch with handshake, capacity=32;

pipeline LoadStoreUnit {
    @MemFetch stage IssueRead {
        mem_addr := calc_addr(base, offset);
    }
    
    // If 32 reads are issued but haven't returned, the IssueRead stage 
    // automatically asserts backpressure upstream until a slot frees up.
}
```

### 5\. Zero-Size Types and Implicit Tagging

Zero-Size Types (()) let you use the tag system to synchronize purely logical events without generating physical data wires. Implicit tagging allows you to apply a domain to an entire block at once.

```
tag @ControlSync;

module SystemController {
    
    // Implicit Tagging: Every operation inside this block 
    // inherits the @ControlSync domain.
    @ControlSync {
        
        // Zero-Size Type: 'trigger' takes up 0 physical bits, 
        // but the compiler will delay the start of 'FSM_Start' 
        // to perfectly align with the domain's longest path.
        var trigger: () := button_pressed & system_ready;
        
        if (trigger) {
            => State_Boot;
        }
    }
}
```

# **Architectural Facets & Composition**

# Architectural Facets & Composition

To address the complexities of modern, trillion-component hardware scaling, FLHDL radically departs from the traditional, monolithic module structure found in legacy HDLs. Instead, it enforces a strict separation of concerns through a paradigm known as the "Three Facets" and provides high-level operators for linking modular dataflows.

## The Three Facets

In FLHDL, hardware development is decoupled into three distinct layers. This allows architects to perform deep structural exploration without needing to manually rewrite RTL logic.

1. **The Design Facet (Architectural Intent):** This facet defines the core logic, dataflow, and interface contracts. At this level, the code is purely behavioral and "time-less." The designer specifies *what* the hardware should do, without worrying about whether a specific mathematical operation requires combinatorial wires or sequential flip-flops to meet timing.  
2. **The Implementation Facet (Physical Mapping):** The implementation facet contains the specific physical mappings for the design. It dictates how the abstract logic is mapped to physical fabric (e.g., FFs, block RAMs, DSP slices, or legacy IP). It handles retiming, clock-gating, and determines the physical synthesis of the logic.  
3. **The Configuration Facet (The Linker):** Acting much like a software build script or linker, the configuration facet sits entirely outside the logic design. It is used to bind specific implementations to architectural "sockets," set compile-time generics, and establish hierarchical pathing. If you need to change a nested module's internal implementation for a new FPGA target, you only modify the Configuration file—the parent Design Facet remains untouched and fully verified.

## Composable Interfaces

At the heart of FLHDL's architecture is the concept of the **Composable Interface**. In FLHDL, *any* first-class hardware citizen—whether it is a standard Module, a Pipeline, or a Finite State Machine—can satisfy a named interface "socket."  
Interfaces act as universal contracts. By defining standard interfaces (such as a parameterized Wishbone bus), designers can decouple the signal definitions from their specific implementation roles, ensuring perfectly matched ports and preventing electrical contention (e.g., two masters driving the same wire).

## High-Level Composition Operators

FLHDL provides specialized syntax to easily compose and route these interfaces together without the boilerplate of massive port-maps.

* **Pipeline Chaining (|\>):** When dealing with streaming data, FLHDL utilizes the |\> operator to chain compatible interfaces. This allows developers to link modular processing stages (e.g., source |\> Grayscale() |\> Blur() |\> display) instantly. The compiler automatically matches the outputs of one unit to the inputs of the next.  
* **Parallel Composition (fork and join):** For operations that must branch and execute concurrently, FLHDL offers fork and join blocks. When paired with synchronization tags, the compiler automatically manages the latency balancing. If branch A takes 10 cycles and branch B takes 2 cycles, the join block instructs the compiler to automatically inject the necessary delay registers into branch B, ensuring asymmetric parallel paths perfectly align when they reconverge.

## Late Binding

Because FLHDL utilizes the Configuration Facet to tie everything together, it inherently supports **Late Binding**.  
During development, the specific hardware "plug" for a "socket" does not need to be known. The exact physicalization is deferred until the final netlist generation. This allows for dynamic socket discovery, the usage of "Ghost Components" (unbound interfaces treated as transparent wires or null-ops), and highly isolated parallel testing via Co-Simulation before the full system is even synthesized.  
Here are code examples demonstrating the Architectural Facets and Composition features in FLHDL:

## 1\. Composable Interfaces

Interfaces act as standard "sockets." Any module, pipeline, or FSM can implement this contract.

```
// Defines a universal contract for streaming 32-bit data
interface StreamingOp {
    in  data_in: u32;
    out data_out: u32;
}
```

## 2\. The Three Facets & Late Binding

This demonstrates how architectural intent (Design) is decoupled from physical realization (Implementation), tied together only at compile-time (Configuration).

```
// --- 1. DESIGN FACET (Architectural Intent) ---
module ImageProcessor {
    // We instantiate an abstract socket, not a concrete module
    instance my_op: StreamingOp;
    
    // Use the abstract socket in our logic
    out_pixel := my_op(in_pixel);
}

// --- 2. IMPLEMENTATION FACET (Physical Mapping) ---
pipeline FastMultiplier implements StreamingOp {
    pipe temp: u32;
    stage One { temp := data_in * 2; }
    stage Two { data_out := temp + 5; }
}

pipeline DSPMultiplier implements StreamingOp {
    // A different implementation, perhaps mapping directly to a hard DSP slice
}

// --- 3. CONFIGURATION FACET (The Linker) ---
config SystemBuild {
    // Late Binding: We decide here which "plug" goes into the "socket"
    // We can swap to DSPMultiplier without touching the Design Facet
    bind ImageProcessor.my_op => FastMultiplier;
}
```

## 3\. Pipeline Chaining (|\>)

The pipe operator instantly links compatible interfaces, removing the need for tedious manual port-mapping of intermediate wires.

```
module CameraISP {
    // Assuming Grayscale, Blur, and Sharpen all satisfy the StreamingOp interface.
    // The compiler automatically wires data_out of one into data_in of the next.
    
    p_out := p_in |> Grayscale() |> Blur() |> Sharpen();
}
```

## 4\. Parallel Composition (fork and join)

This construct explicitly branches logic and forces the compiler to resolve asymmetric latencies when the data paths reconverge.

```
module ALU {
    // The compiler performs Longest Path Analysis. If 'heavy_op' takes 10 cycles 
    // and 'light_op' takes 2 cycles, the compiler automatically injects 8 cycles 
    // of delay registers into branch B.
    
    fork {
        branch A { 
            var res_a := @Sync heavy_op(p_in); 
        }
        branch B { 
            var res_b := @Sync light_op(p_in); 
        }
    } join @Sync (
        // The data is guaranteed to reconverge safely aligned in time
        result => res_a + res_b
    );
}
```

# **Testing & Integration (FFI)**

## **Testing & Integration (FFI)**

A fundamental philosophy of FLHDL is that a hardware description language should focus entirely on describing hardware. Rather than bloating the language with a massive, non-synthesizable subset for file I/O, string manipulation, and complex verification constraints, FLHDL delegates these tasks to modern software languages. It achieves this through a robust Foreign Function Interface (FFI) and a "Black-Box" integration model.

### **"Black-Box" Legacy Interoperability**

Integrating legacy IP is a critical reality of hardware design. FLHDL allows you to reuse existing Verilog or VHDL modules without polluting your pure LL(1) design files with instantiation wrappers.

Because of the strict separation of concerns, the integration happens entirely in the **Configuration Facet**:

1. **The Abstract Socket:** In the Design Facet, you simply declare a standard FLHDL interface. To the rest of your architecture, the component looks exactly like native code.  
2. **The Linkage:** In your Configuration file, you utilize a foreign or blackbox binding. This instructs the FLHDL compiler to halt elaboration at that boundary and map the abstract ports directly to the legacy Verilog/VHDL ports.

When the compiler encounters this boundary, it performs automatic **Type Marshalling**, verifying that the bit-widths of your FLHDL types (e.g., u32) perfectly match the vector widths (e.g., \[31:0\]) of the target module. Furthermore, if your design uses Latency-Insensitive Tags around this black-box, the compiler will treat the Verilog module as a fixed-latency block and automatically generate the necessary elastic buffers around its inputs and outputs to maintain total dataflow synchronization.

### **Co-Simulation via RPC**

To verify hardware behavior, FLHDL connects natively to modern programming languages like Go, Rust, and C++. Instead of relying on archaic simulation scripts, FLHDL implements **Co-Simulation via RPC**.

By binding an interface to a "Foreign" implementation in your test configuration, the FLHDL compiler automatically generates a communication bridge utilizing gRPC over Unix Domain Sockets. This allows a software-based testbench to run concurrently with the simulated hardware. The software can feed stimuli and evaluate responses in real-time, executing high-speed, parallel test suites using the native testing frameworks of your chosen software language.

### **Automated Marshalling**

Writing software drivers to communicate with simulated hardware often involves tedious, error-prone bit-shifting and masking. FLHDL eliminates this bottleneck through **Automated Marshalling**.

When a hardware interface is bound to a foreign software model, the FLHDL compiler automatically generates the corresponding, bit-exact software structs (in C, C++, or Rust). This guarantees that the software representation perfectly matches the hardware ports. A verification engineer can simply interact with standard software variables—the RPC bridge handles the packing, serialization, and cycle-accurate injection of those bits into the HDL simulation.

### **Behavioral Facets for Verification**

Because FLHDL relies heavily on Late Binding, verification engineers can utilize "Behavioral Facets." During early architectural exploration, you can write a high-level software implementation of a complex block (like a memory controller or DSP algorithm) in C++ or Rust.

Using the Configuration Facet, you can bind your FLHDL interface to this software model. As the project matures, you seamlessly swap the configuration binding from the software model to the physical RTL implementation. The parent logic remains entirely untouched and completely unaware that the underlying component transitioned from software to hardware.

Here are code examples demonstrating how Testing & Integration (FFI) features are implemented in FLHDL:

### **1\. "Black-Box" Legacy Interoperability**

This example shows how to import an existing Verilog module into an FLHDL design without wrapping it in messy instantiation logic. The integration happens cleanly in the Configuration Facet.

This example shows how to import an existing Verilog module into an FLHDL design without wrapping it in messy instantiation logic. The integration happens cleanly in the Configuration Facet.

```
// --- 1. DESIGN FACET (The abstract socket) ---
interface LegacyMultiplier {
    in  clk: bool;
    in  a: u32;
    in  b: u32;
    out res: u32;
}

module MathUnit {
    instance fast_mult: LegacyMultiplier;

    result := fast_mult(clock, val1, val2);
}

// --- 2. CONFIGURATION FACET (The Linker) ---
config SystemBuild {
    bind MathUnit.fast_mult => blackbox("verilog", "legacy_mult_top.v");
}
```

### **2\. Co-Simulation via RPC & Automated Marshalling**

Instead of writing a testbench in Verilog/VHDL, you bind your hardware interface directly to a software program. The compiler will automatically generate the RPC bridge and the bit-exact software structs.

Instead of writing a testbench in Verilog/VHDL, you bind your hardware interface directly to a software program. The compiler will automatically generate the RPC bridge and the bit-exact software structs.

```
// 1. Define the hardware interface
interface PCIeLink {
    in  rx_packet: u64;
    out tx_packet: u64;
    out link_up: bool;
}

// 2. Configure the Test Environment
config TestBench {
    bind system.pcie => foreign("cpp", "pcie_driver_model.cpp");
}
```

*Behind the scenes:* The FLHDL compiler automatically generates a matching pcie\_driver\_model.hpp file for the verification engineer, completely eliminating the need to write bit-shifting/masking drivers manually.

### **3\. Behavioral Facets (Late Binding)**

This illustrates how verification engineers can swap between high-level software algorithms and physical hardware implementations seamlessly as a project matures.

This illustrates how verification engineers can swap between high-level software algorithms and physical hardware implementations seamlessly as a project matures.

```rust
// --- EARLY DEVELOPMENT: Fast algorithmic testing ---
config EarlySim {
    bind CPU.mem_ctrl => foreign("rust", "mem_behavioral.rs");
}

// --- LATE DEVELOPMENT: Full RTL integration ---
config ProductionSynthesis {
    bind CPU.mem_ctrl => PhysicalDDR4Controller;
}
```

Here are examples demonstrating how FLHDL utilizes Co-Simulation via RPC to interface hardware designs directly with modern software languages (like C++ and Rust) for testing and behavioral modeling.

### **Example 1: Testing a PCIe Link with a C++ Driver**

Instead of writing a complex testbench in Verilog to generate PCIe packets, you can bind the hardware interface directly to a C++ program. FLHDL will automatically generate a gRPC bridge over Unix Domain Sockets to handle the communication.

**1\. The FLHDL Design & Configuration (Hardware Side)**

Define the universal hardware contract and the linker configuration.

```
// DESIGN FACET: Define the universal hardware contract
interface PCIeLink {
    in  rx_packet: u64;
    out tx_packet: u64;
    out link_up: bool;
}

module SystemTop {
    instance pcie: PCIeLink;
}

// CONFIGURATION FACET: The Linker
config TestBench {
    bind SystemTop.pcie => foreign("cpp", "pcie_driver_model.cpp");
}
```

**2\. The Auto-Generated C++ Header (Software Side)**

When you compile the FLHDL code, the compiler automatically generates a bit-exact C++ struct and the RPC bridge boilerplate (pcie\_driver\_model.hpp). You do not have to write this manually:

When you compile the FLHDL code, the compiler automatically generates a bit-exact C++ struct and the RPC bridge boilerplate.

```c#
// AUTO-GENERATED BY FLHDL COMPILER
#include <stdint.h>

struct PCIeLink_Ports {
    uint64_t rx_packet;  // Injected into hardware
    uint64_t tx_packet;  // Read from hardware
    bool     link_up;    // Read from hardware
};

void tick_hardware(PCIeLink_Ports* ports);
```

**3\. Your Custom C++ Testbench Logic**

You simply write standard C++ to interact with the auto-generated struct. The RPC bridge handles the cycle-accurate synchronization with the HDL simulator.

Standard C++ testbench logic interacting with the generated ports.

```c
#include "pcie_driver_model.hpp"
#include <iostream>

int main() {
    PCIeLink_Ports ports;
    ports.rx_packet = 0xDEADBEEF00001111;
    tick_hardware(&ports);

    if (ports.link_up) {
        std::cout << "Hardware responded with: " << ports.tx_packet << std::endl;
    }
    return 0;
}
```

### ---

**Example 2: Behavioral Modeling in Rust**

During early development, the RTL for a complex module (like a DDR4 controller) might not exist yet. You can use FFI via RPC to bind the interface to a high-level behavioral model written in Rust.

**1\. The FLHDL Design & Configuration**

**\`\`\`**  
interface MemController {  
    in  req\_valid: bool;  
    in  address: u32;  
    out data: u32;  
    out ready: bool;  
}

module CPU {  
    instance mem: MemController;  
    // ... CPU logic ...  
}

config EarlySim {  
    // Bind to a Rust software model instead of physical RTL  
    bind CPU.mem \=\> foreign("rust", "mem\_behavioral.rs");  
}  
\`\`\`

**2\. The Rust Behavioral Model**

The FLHDL compiler generates the Rust bridge, allowing you to quickly write the logic to fake the memory responses without touching hardware description.

```rust
// mem_behavioral.rs
use flhdl_rpc_bridge::MemControllerPorts;
use std::collections::HashMap;

pub fn eval_cycle(ports: &mut MemControllerPorts, ram: &mut HashMap<u32, u32>) {
    ports.ready = true;
    if ports.req_valid {
        let val = ram.get(&ports.address).unwrap_or(&0);
        ports.data = *val;
    }
}
```

### **Why this FFI / RPC approach is powerful:**

1. **Automated Marshalling:** The FLHDL compiler guarantees that the software structs (e.g., uint64\_t or bool) perfectly match the bit-widths of the hardware ports. No manual bit-shifting or bit-masking is required.  
2. **Cycle-Accurate:** The gRPC bridge automatically pauses and resumes the HDL simulator, ensuring that your C++/Rust loop ticks exactly 1-to-1 with the hardware clock domain.  
3. **No Messy Testbench Subsets:** You get to use the full power of modern software languages (File I/O, networking, threading, HashMaps) for verification without bloating the FLHDL language itself.

# **Handling of Clock and Reset**

# Handling of Clock and Reset

Because FLHDL treats time as an implementation detail rather than a design constraint, you do not manually route clock (`clk`) or reset (`rst`) signals through your logic in the Design Facet.

Instead, FLHDL handles clocks via **Synchronization Tags** and the **Configuration Facet**.

Here is exactly how you handle a clocked interface like an AXI bus at the top level:

### **1\. The Design Facet: Abstract the Clock**

In your design code, you define the AXI interface purely in terms of its dataflow and handshake semantics. You group the module under a synchronization domain (a Tag), such as `@AxiDomain`.

The compiler knows that everything sharing the @AxiDomain tag belongs to the same physical clock region, but at the design level, you just write the logic.

```
// 1. Design Facet (No clocks here!)
package amba.axi;

interface AxiStream <data_w: int> {
    in  data  : uint<data_w>;
    in  valid : bit;
    out ready : bit;
}

// We tag the module to a specific synchronization domain
module AxiProcessor @AxiDomain (
    bus : AxiStream<32>
) {
    // Pipeline or FSM logic here...
    // The compiler automatically infers registers based on the @AxiDomain
}
```

### **2\. The Configuration Facet: Linking to the Physical World**

When you are ready to synthesize this to an FPGA or connect it to an existing Verilog AXI IP, you use the Configuration Facet. This is where the abstract @AxiDomain is mapped to a physical clock.

If you are wrapping a legacy Verilog AXI module, you use a blackbox binding. The compiler automatically maps its internal concept of the domain's clock to the physical clock pin of your Verilog IP.

```
// 2. Configuration Facet (The Linker)
package system.config;

import amba.axi;

// Tell the compiler what @AxiDomain actually means physically
domain AxiDomain {
    clock: sys_clk_100mhz;
    reset: sys_rst_n; 
}


// Bind our FLHDL processor to be the top-level or link to external IP
bind AxiProcessor => TopLevel;
```

### **3\. What the Compiler Does (The Physical Emission)**

When the FLHDL compiler (written in Go/Rust) processes your design and emits the final Verilog/VHDL netlist, it looks at the Tags and automatically unrolls them into physical clock networks.

At the absolute top level, the compiler will synthesize the sys\_clk\_100mhz and sys\_rst\_n definitions from your Configuration Facet into actual, physical top-level ports.

The emitted Verilog will look something like this:

```

// 3. Emitted Verilog (Generated by FLHDL Compiler)
module AxiProcessor_Top (
    // The compiler injected these because of the @AxiDomain tag!
    input wire clk_AxiDomain,   
    input wire rst_n_AxiDomain, 

    // The actual AXI interface payload
    input wire [31:0] bus_data,
    input wire        bus_valid,
    output wire       bus_ready
);
    // ... generated state machines, elastic buffers, and registers 
    // all automatically wired to clk_AxiDomain ...
endmodule
```

### **Summary of the Flow:**

1. **You write:** module MyCore @DomainX (...)  
2. **You configure:** @DomainX is a 200MHz hardware clock.  
3. **The Compiler emits:** A Verilog module with an explicit input wire clk\_DomainX port and routes it to every register, pipeline, and elastic buffer inside that module.

If you have multiple clocks (e.g., AXI on 100MHz, and a DSP core on 250MHz), you just use two different tags (@AxiDomain and @DspDomain). If data crosses between them, the compiler sees the Tag transition and automatically injects an asynchronous FIFO/elastic buffer.

# **FLHDL Grammar**

# FLHDL Grammar

This is the complete, consolidated EBNF grammar for FLHDL (FLHDL) based on all our design sessions. As a latency-insensitive, strictly LL(1) hardware description language, this grammar ensures that a parser never has to guess or backtrack, utilizing unique keywords and single-token lookaheads for every decision.

### ---

**1\. Top-Level Structure and Namespaces**

An FLHDL source file is organized hierarchically into packages and imports, followed by top-level definitions that map to the Design, Implementation, or Configuration facets.

* **Packages and Imports:** Namespaces use a dot-delimited path, and all symbols are private by default unless marked with the pub keyword.  
* **Facets:** The language is divided into structural declarations (Interfaces), logical implementations (Modules, Pipelines, FSMs), and a separate config block that acts as a linker.

```
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

### ---

**2\. Interfaces, Elements, and Modports**

Interfaces act as the universal contract or "socket" that can be satisfied by any first-class citizen.

* **Elements:** You can separate the structural declaration of a sub-component (an element) from its implementation, allowing it to be compiled conditionally via the configuration facet.  
* **Modports:** Modports define the directionality of signals for specific roles, like a Master or Slave.  
* **Generics:** Interfaces and modules can accept compile-time parameters enclosed in \< \>.

```
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

### ---

**3\. Types, Structs, and Enums**

FLHDL uses a Rust-like type system for scalars while supporting complex, nested structures and tagged enums.

* **Structs:** A standard product type containing named fields.  
* **Enums:** Sum types (tagged unions) where variants can optionally hold data payloads or be explicitly mapped to physical binary/hex literals.

EBNF

```
struct_def ::= "struct" identifier "{" { identifier ":" type "," } "}"

enum_def ::= "enum" identifier "{" { enum_variant } "}"
enum_variant ::= "case" identifier [ "(" type ")" ] [ "=" literal ] ";"

type ::= identifier [ "<" literal { "," literal } ">" ] [ "[" slice_or_index "]" ]
```

### ---

**4\. First-Class Hardware Primitives**

Pipelines, FSMs, Procedures, and Functions are first-class hardware constructs that can optionally implement an interface.

* **Pipelines:** Can use implicit pipe declarations for automatic live-range analysis and bridging, or explicit in/out port boundaries. Stages can also bind directly to standalone procedures with renaming.  
* **FSMs:** Utilize explicit state blocks.  
* **Procs and Funcs:** proc handles explicit directional ports, while func is syntactic sugar that allows tuple-based multiple returns mapped to out ports.

```
module_def ::= "module" identifier [ generic_decl ] [ "implements" identifier ] "{" { statement | instance_decl } "}"

pipeline_def ::= "pipeline" identifier [ generic_decl ] [ "implements" identifier ] "{" { pipe_decl | stage_def } "}"
pipe_decl ::= "pipe" identifier ":" type ";"
stage_def ::= "stage" identifier [ "(" port_list ")" ] [ ":" identifier "(" bind_list ")" ] "{" { statement } "}"

fsm_def ::= "fsm" identifier [ generic_decl ] [ "implements" identifier ] "{" { state_def } "}"
state_def ::= "state" identifier "{" { statement } "}"

proc_def ::= [ "pub" ] "proc" identifier "(" [ port_list ] ")" "{" { statement } "}"
func_def ::= [ "pub" ] "func" identifier "(" [ port_list ] ")" "->" return_type "{" { statement } "}"

return_type ::= type | "(" type { "," type } ")"
port_list ::= port_decl { "," port_decl }
bind_list ::= identifier "=>" identifier { "," identifier "=>" identifier }
instance_decl ::= identifier ":" identifier "(" bind_list ")" ";"
```

### ---

**5\. Tagged Dataflow and Protocols**

Tags manage synchronization domains and backpressure across latency-insensitive pipelines.

* **Protocols:** Custom handshakes are defined by categorizing signals into forward and backward flow directions.  
* **Tags:** Link a data type to a synchronization domain, optionally attaching a custom protocol and tracking capacity (which generates a hardware scoreboard).

EBNF

```
protocol_def ::= "protocol" identifier "{" { protocol_signal } "}"
protocol_signal ::= ( "forward" | "backward" ) port_decl ";"

tag_def ::= "tag" identifier ":" type [ "with" identifier ] [ "capacity" "=" literal ] ";"
tag_prefix ::= "@" identifier | "~"
```

### ---

**6\. Statements and Control Flow**

Assignments use :=, while structural bindings and state transitions use \=\>.

* **Conditionals:** if and match represent combinational routing (multiplexers/decoders), with match allowing destructuring of Enums and Tags.  
* **Parallelism:** The fork and join block manages parallel execution branches, where the compiler re-aligns latency at the join node.  
* **State Transitions:** Handled via next \=\>.

EBNF

```
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

match_stmt ::= "match" [ tag_prefix ] expression "{" { match_arm } [ "default" ":" { statement } ] "}"
match_arm ::= "case" pattern ":" { statement }
pattern ::= literal | bit_mask | identifier

fork_stmt ::= "fork" "{" { "branch" identifier "{" { statement } "}" } "join" ";"
```

### ---

**7\. Expressions, Ranges, and LValues**

FLHDL supports standard expressions but extends left-values (LValues) and arrays with domain-specific slicing.

* **Multidimensional Indices:** Uses comma-separated expressions (e.g., a\[1, 2\]) for flat tensor offsets.  
* **Ranges:** Includes ascending half-open .., ascending inclusive ..=, directional descending \<-, and directional ascending \-\> to resolve hardware endianness explicitly.

EBNF

```
lvalue ::= identifier { "." identifier | "[" slice_or_index "]" }

slice_or_index ::= expression [ ( "," expression ) | ( range_op expression ) ]
range_op ::= ".." | "..=" | "<-" | "->"

expression ::= term { binary_op term }
term ::= literal | lvalue | "(" expression ")" | function_call
function_call ::= identifier "(" [ expression { "," expression } ] ")"

binary_op ::= "+" | "-" | "*" | "/" | "&" | "|" | "^" | "==" | "!=" | "<" | ">" | "<=" | ">=" | ">>" | "<<" | "|>"
```

