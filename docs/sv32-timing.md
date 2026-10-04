<!-- SPDX-License-Identifier: Apache-2.0 -->
# Sv32 on Vreteno: where the translation goes, and what it costs

This is the design note and the timing experiment of issue 1009, item M2 of the Linux plan in issue 279.
It answers one question before the memory management unit is built (issue 1014): does Sv32 fit the core's ten nanosecond clock, and if not, should the core get an extra fetch stage, a slower clock, or a pipelined ALU?

The short answer is that Sv32 fits at ten nanoseconds with none of the three.
Translation stays off the two paths that set the clock, if it is done from addresses already held in a register.
That costs a cycle on each access that goes to the bus.
What the measurements also show is that the bus is where Linux will spend its time anyway: a loop fetched over the bus runs at about fourteen and a half cycles an instruction today, against one and a third from the boot memory.
So the instruction cache of issue 1021 matters far more to Linux than any of the three options.

Nothing here merges into the core.
The prototypes are on two branches kept for reference, `ai-exp-20261004-sv32tlb1009` and `ai-exp-20261004-sv32opC1009`, and the section on method says how to run them again.

## The two paths that set the clock

Every number below is from `//cpu/vreteno:vreteno_pnr`, the core alone, out of context, placed and routed for the AX7A200B's xc7a200t against a ten nanosecond clock (issue 971).
The baseline is main at `63f6ea8a`, on 2026-10-04.
It meets ten nanoseconds with 0.267 ns of setup slack and 0.132 ns of hold.

Two paths are within half a nanosecond of the limit, and they start in the same place.

* **The ALU path**, 9.717 ns, slack 0.267 ns.
  From the first register field of the instruction register, through the register file's read, the forwarding from writeback and the ALU's adder, into the writeback stage's result, `wb_alu`.
  Seventeen levels, nine of them carry.
* **The fetch path**, 9.007 ns, slack 0.429 ns.
  From the same field, through the register file and the branch's compare, into the next program counter and the address of the boot memory's block RAM.
  Twelve levels.

Issue 1009 quoted 0.296 ns of slack on the fetch path.
That figure is an older core's.
Since issue 972 the worst path is the ALU's, and the fetch path is the second.

The two are close enough that placement decides which one is worst.
Changing logic elsewhere in the core moved each of them by up to a quarter of a nanosecond between runs, in either direction.
A difference smaller than that between two runs below is noise, not a result.

## Where the translation sits

Sv32 translates in supervisor and user mode only.
Machine mode fetches and accesses memory untranslated.
That is what keeps the fetch path out of it.

### Fetch

The boot memory holds the first four kilobytes and is machine mode's.
The kernel and every user program run from DDR3, above it, and the core fetches those over the bus, into its buffer of two words, one request per word.
So the fetch path above, the next program counter into the block RAM's address, never sees a virtual address that needs translating.
The lookup belongs on the bus fetch's request instead.

Variant A put a fetch TLB there in the simplest way: eight entries, looked up in full in the cycle the request goes out.

| | baseline | A |
|---|---|---|
| setup slack, ns | 0.267 | 0.183 |
| fetch path slack, ns | 0.429 | 0.183 |
| ALU path slack, ns | 0.267 | 0.554 |
| from a TLB register, worst path, ns | | 5.697 |
| to the bus request's port, ns | 8.900 | 12.527 |
| LUTs | 2622 | 2917 |
| flip-flops | 809 | 1218 |

The TLB's own paths are short: from a tag register through the compare to the walker's state is 5.7 ns, more than four nanoseconds inside the clock.
The design still meets ten nanoseconds, and the two critical paths moved by no more than the noise.

The cost is the request.
Out of context the bus request leaves the core at a port, and a path to a port is not timed, so the report gives its delay rather than a slack.
Today the worst path to that port is 8.9 ns, from the instruction register through the register file to the request.
With the lookup in the request cycle it is 12.5 ns, from the program counter through the buffer's compare, the next word's address and the TLB's lookup, in nineteen levels, eleven of them carry.
On the board the bus tracker sits behind that port, so this does not fit.

So the fetch TLB looks up an address that is already in a register: the virtual address of the word the buffer misses, held for a cycle, then translated, then sent.
That is the 5.7 ns path above, and it costs one cycle per bus fetch.

### Data

The load or store address is the adder's output in execute, and it goes to the bus in the same cycle.
A lookup after the adder was not run as a variant of its own.
Variant A's request grew by 3.6 ns with a lookup in it, and the data address already spends 8.9 ns on its way to the port, so a lookup there is not worth building to find out.

The data TLB therefore looks up the address in `wb_alu`, the register writeback already holds it in, and the access goes out from writeback a cycle later.
A load already waits in writeback for its answer, so the wait grows by a cycle.
A store is posted, so it leaves a cycle later and the core stalls only if the next access is behind it.

### With an instruction cache

When issue 1021 adds an instruction cache, its lookup becomes the fetch path.
A cache indexed by the page offset, four kilobytes or less per way, can be read with the virtual address while the TLB translates it, and its tag compared with the physical page number after.
The TLB then runs in parallel with the cache read rather than in front of it, which is how the fetch keeps one cycle.

## The walker

* Two levels, `satp`'s page then the entry's, with four kilobyte pages and four megabyte megapages.
* One walker shared by the fetch TLB and the data TLB, reading page table entries over the core's own bus port, one read outstanding, as the prototype does.
* It never writes.
  An entry whose accessed bit is clear, or whose dirty bit is clear on a store, raises a page fault, and the kernel sets the bit, which the privileged specification allows and names Svade.
  A walker that wrote would need an atomic read and write on the bus, which the AXI link does not have today (issue 1010).
* A page fault is cause 12, 13 or 15, with the virtual address in `stval`.
  A misaligned megapage, a leaf at the first level whose low page number is not zero, is a page fault too.
* The permission checks are on a hit: R, W, X and U against the mode, with `sstatus.SUM` and `sstatus.MXR`.
* A write to `satp` or an `sfence.vma` empties both TLBs.
  Address space identifiers are not kept, which the specification allows.

### Sizes

Eight entries each, fully associative, replaced in turn.
Variant A's fetch TLB of eight entries and its walker cost 295 LUTs and 409 flip-flops: a ninth of the core's LUTs, and half as many flip-flops again as it had.
A second eight for data costs about the same again.
Eight megapages cover 32 MiB, so a kernel mapped with megapages where its alignment allows it keeps its own text and data in few entries.

## The three options

### An extra fetch stage

It would buy slack on the fetch path.
Nothing above needs it: the TLB is not on that path.
Its cost is a cycle on every taken transfer of control, since the redirect would wait one stage longer.

### A slower clock

Nothing measured needs one.
Every variant met ten nanoseconds, the worst at 0.089 ns of slack.

### A pipelined ALU

Option C is the cheapest form, and it was built and run.
The register file is read at the edge the instruction enters execute, at the fetched word's register fields, with that edge's write passed through, into two operand registers.
Execute keeps its forwarding from writeback.
No instruction waits for it, so it costs no cycles.
With A beside it, the lockstep test, the counters test and the OpenOCD test pass, and the five programs below retire in exactly as many cycles as on the baseline.

| | baseline | C | A and C |
|---|---|---|---|
| setup slack, ns | 0.267 | 0.180 | 0.089 |
| fetch path slack, ns | 0.429 | 0.180 | 0.089 |
| ALU path slack, ns | 0.267 | 0.432 | 0.654 |
| to the operand registers, slack, ns | | 0.694 | 0.525 |
| LUTs | 2622 | 2916 | 2891 |
| flip-flops | 809 | 870 | 1276 |

The ALU path gains about 0.17 ns, the register file's read, which is within the noise named above: variant A alone, with no change to the operands, moved it by more.
The clock does not rise, because the fetch path is then the worst, and it was within noise of the ALU path to begin with.
The new path, from the boot memory's output through the halfword's choice, the expander and the register file into an operand register, has half a nanosecond to spare.

The other form, the adder split over two stages, was not built.
Its cost in cycles is counted from the programs instead: a bubble whenever an instruction reads the result of the one retired the cycle before, and if the branch is decided in the second half, a cycle on every taken transfer as well.
It would not raise the clock either, for the same reason.

### The cost in cycles

Five programs run on `vreteno32::run`, the whole machine in simulation, and `//cpu/vreteno:cpi` counts their retirements.
The demonstration and the icosahedron are cut off by their cycle limit, so theirs are a window rather than a whole run.
None of them is a benchmark; there is no Dhrystone in the tree.

| program | instructions | cycles | CPI | split ALU | split ALU, late branch | extra fetch stage | data lookup at writeback |
|---|---|---|---|---|---|---|---|
| demo | 4429 | 19997 | 4.515 | 4.841 | 5.160 | 4.834 | 4.835 |
| hello | 357 | 1069 | 2.994 | 3.235 | 3.462 | 3.221 | 3.232 |
| traps | 1166 | 3912 | 3.355 | 3.558 | 3.757 | 3.554 | 3.646 |
| steps | 3665 | 11940 | 3.258 | 3.514 | 3.723 | 3.466 | 3.436 |
| ico | 736580 | 1999997 | 2.715 | 2.877 | 2.994 | 2.833 | 2.912 |

The last four columns add a cycle per event to the measured cycles: per dependent pair retired back to back, per taken transfer, and per load or store.
They are counts on today's schedule, so they are an upper bound where an existing stall would have hidden the new one.

## The cost that dominates: fetching over the bus

The same loop of three instructions runs at 1.333 cycles per instruction from the boot memory and at 14.655 from the data memory, fetched over the bus.
That is in simulation, against the data memory one hop away on the router; DDR3 on the board is further.
Linux runs entirely from there.

Against about thirteen cycles more a word, the fetch TLB's extra cycle is a few per cent, and the options above are of the same order.
A kernel at fourteen cycles an instruction would boot, eventually, and nobody would use it.

## Recommendation

1. Keep the ten nanosecond clock and the three stages.
   Sv32 does not need an extra fetch stage, a slower clock or a pipelined ALU.
2. Build the fetch TLB on the bus fetch's request, looking up a virtual address held in a register: eight entries, a cycle per bus fetch.
3. Build the data TLB at writeback, on `wb_alu`: eight entries, a cycle per load or store, five to nine per cent on the programs above.
4. One walker for both, two levels, never writing, A and D by page fault.
5. Bring the instruction cache of issue 1021 forward, before the memory management unit is on the board, and build it indexed by the page offset with the fetch TLB beside it.
   Without it the kernel runs about eleven times slower than the same code from the boot memory.
6. Option C is free in cycles and buys 0.17 ns on the ALU path.
   Take it only with work on the fetch path, since alone it moves the worst path and not the clock.
7. Check the bus request's path on the board build once issue 1014 has a TLB in it.
   Out of context it is a port, and only the board says what the tracker behind it adds.

## Method

The prototypes were written in the core's own source, so they lowered and simulated like the rest of it:

* Variant A, on `ai-exp-20261004-sv32tlb1009` at `c83a8876`: `satp` at `0x180`, eight tags and eight physical page numbers, valid bits, a turn-by-turn victim, and a two-level walker on the bus port, filling an entry on a leaf.
  It has no permission checks and no page faults; it is the lookup and the walker, which is what the clock sees.
* Variant C alone, on `ai-exp-20261004-sv32opC1009` at `06e82790`.
* A and C together, on `ai-exp-20261004-sv32tlb1009` at `bdabaf6f`.

`cpu/vreteno/BUILD.bazel` on those branches adds Tcl to the place and route that prints the worst path to the boot memory's address, to `wb_alu`, to the operand registers, from the TLB, and to the bus request's port, each after a `TXHDL-PROBE` line in `bazel-bin/cpu/vreteno/vreteno_pnr.log`.

```sh
git checkout ai-exp-20261004-sv32tlb1009
bazel build //cpu/vreteno:vreteno_pnr    # manual, Vivado, about five minutes
bazel run //cpu/vreteno:cpi              # the cycle counts
```

The lockstep test passes on variant A, and the lockstep, counters and OpenOCD tests pass on A and C together, with `satp` at zero, which leaves translation off.
Variant C alone was placed and routed and not simulated on its own.
