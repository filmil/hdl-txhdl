---
draft: true

---

# LHdl: Language Specification (2026 Standard)

LHdl is a modern Hardware Definition Language built on the principle that architectural intent should be decoupled from physical implementation. It prioritizes reuse, deterministic parsing, and the abstraction of time.

## Core Philosophy & The Three Facets

LHdl separates the concerns of hardware development into three distinct facets to allow for architectural exploration without manual RTL rewriting.

-   Design Facet: Defines the logic, dataflow, and interface contracts. At this level, there is no distinction between combinatorial and sequential logic.
    
-   Implementation Facet: Maps the design to specific technology. Handles retiming, clock-gating, and determines if a network is combinatorial or sequential based on timing constraints.
    
-   Configuration Facet: Handles the assembly of the system, binding specific implementations to interface instances and setting generic parameters.
    
## Syntax & Lexical Structure

LHdl is LL(1) to ensure fast, deterministic parsing and simple tooling. This is not a hard requirement, but it works for small and straightforward languages.

-   Brace-Scoped: Uses `{...}` for blocks to improve scannability and reduce reserved keywords (you don't need to make `begin`, `end`, `else` etc reserved.
    
-   Prefix-Heavy: Every construct is announced by a unique keyword or symbol. This is a consequence of LL(1).
    
-   Flat Arrays: Multidimensional arrays are natively supported; the compiler handles flat bit-offset calculations:  

    $$\text{Offset} = \left( \sum_{k=1}^{n} \left( i_k \prod_{j=k+1}^{n} D_j \right) \right) \times \text{sizeof}(Type)$$
    

## Interfaces: The Universal Contract

Interfaces are the primary unit of reuse. Every first-class citizen (Module, Pipeline, FSM) can implement an interface.
  
```
interface StreamingOp <type T> {  
   in_data : T;  
   out_data : T;
  
  modport Server { in in_data; out out_data; }  
    modport Client { out in_data; in out_data; }  
}  
```  

### Examples

1.  Standard Bus Interface: Defining a Wishbone-like structure.
    
2.  Parameterized Interface: Using `<width: int>` to scale data paths.
    
3.  Role-Based Modports: Defining Master and Slave views of the same wires.
    
4. First-Class Citizens

Modules, Pipelines, and FSMs are specialized implementation styles for interfaces.

### 4.1 Pipelines (Dataflow-Centric)

Pipelines define a sequence of operations. The compiler performs Live-Range Analysis to automatically bridge variables across stages.

```
pipeline MultPipe implements StreamingOp<int>.Server {  
  pipe int temp;  
  stage S1 { temp := in_data * 2; }  
  stage S2 { out_data := temp + 5; }  
}  
```

### 4.2 State Machines (Control-Centric)

FSMs manage sequential transitions and protocol logic.

```
fsm Handshaker implements StreamingOp<int>.Server {  
  state Idle {  
    if (in_data > 0) { next => Processing; }  
  }  
  state Processing {  
    out_data := in_data;  
    next => Idle;  
  }  
}  
``` 

### 4.3 Modules (Structural-Centric)

Modules are used for hierarchical grouping and manual structural wiring.

## 5. Latency-Insensitive Dataflow (Tags)

Tags manage synchronization. The distinction between combinatorial and sequential logic is derived from the tag's implementation.

### 5.1 Tag Definitions & Implicit Scoping

Tags can be applied explicitly or inherited via scope.

  

Code snippet

  
  
```
tag FrameSync : uint32 with handshake, capacity=8;  
  
module Top() {  
  default tag FrameSync; 
  // All assignments in this scope are 
  // now synchronized  
  
  val_a := input_x + 1; // Implicitly @FrameSync  
  val_b := ~input_y;
  // Explicitly "raw" (combinatorial ZST)  
}  
```  

### 5.2 Resource Scoreboarding

When a tag is defined with capacity=N, the compiler instantiates a hardware scoreboard (Priority Encoder + Bitmask) to manage in-flight transactions.

### Examples

1.  Capacity-Limited Processing: A 3-bit tag (8 slots) ensuring a pipeline never overflows.
    
2.  Handshaked Streams: Automatically injecting Ready/Valid logic into a design.
    
3.  Tag-Based Dispatch: Matching on a struct-based tag to route data to parallel workers.
    

## 6. Composition & Configuration

The Configuration Facet allows for late binding and architectural swapping.

### 6.1 Chaining & Parallelism

The pipe operator `|>` chains compatible interfaces, while fork/join manages parallel branches.

```
module ImageProcessor() {  
  // Chain multiple pipelines that satisfy StreamingOp  
  p_out := p_in |> Grayscale() |> Blur() |> Sharpen();  
  
  // Parallel fork-join with automatic latency balancing  
  fork {  
  branch A { res_a := @Sync heavy_op(p_in); }  
  branch B { res_b := @Sync light_op(p_in); }  
  } join @Sync (result => res_a + res_b);  
}  
```  

### 6.2 Implementation Binding

```
// Configuration Facet  
instance my_op : StreamingOp<int>;  
  
// Swap implementations without changing the Design Facet  
bind my_op => MultPipe;  
```  

## 7. Interoperability & Testing

LHdl avoids a non-synthesizable subset for verification. Instead, it provides:

-   Conventional Interop: Ability to wrap Verilog/VHDL modules as LHdl interfaces.
-   Language Hooks: Direct interoperability with Go or Rust for simulation drivers and file I/O.
-   Behavioral Facets: High-level implementations of interfaces used specifically for verification.
    

## 8. Synthesis Strategy

The compiler resolves the abstract "time-less" Design Facet by:

1.  Analyzing Tags: Determining which paths require registers (Sequential) vs. wires (Combinatorial).
    
2.  Latency Balancing: Inserting bridge registers or elastic buffers to satisfy the Tag Dependency Graph.
    
3.  Flat Mapping: Resolving multidimensional array access and struct padding into a target-agnostic netlist before final VHDL/Verilog emission.
<!--stackedit_data:
eyJoaXN0b3J5IjpbMTcwMTExNTk3OSwtNjQ5OTIzNjMzXX0=
-->