<!-- SPDX-License-Identifier: Apache-2.0 -->
# Multi-beat writes across the network on chip

Status: analysis and decision, September 19, 2026; the split built,
September 20, 2026, under issue 125. Section 7 says which steps are
done and which are not, and why.
Author: automated coding assistant, with human supervision.

This answers issue 133: why the network cannot carry a multi-beat AXI4
write today, which of the stated reasons survive measurement, what the
candidate designs cost, and which one issue 125 should implement.
Every number here was measured in this tree or counted from its own
generated tables, and each says where.

## 1. What the network does today

A packet is one beat of one AXI channel, 113 bits wide for the
configuration `//soc` uses, and it carries where it is going and where
it came from (`docs:datasheets`, the Switch sheet).
A switch routes in dimension order and moves one packet per cycle,
and holds three bits: the round robin that says which input the
choice starts from.
A link holds two packets, because a channel is an elastic buffer of
two.

A write crosses as a single packet on channel `W`, carrying the
address phase and its one data beat together.
A write of more than one beat is refused at `HostBridge`: its address
phase and every one of its beats are taken and dropped, and its host
is answered `SlvErr`, so nothing is corrupted and nothing hangs.
That guard is step 1 of the outlook in issue 133 and it is already in
the tree, with `a_write_of_two_beats_is_refused_rather_than_carried`
covering it.

## 2. What was measured

### Reads of several beats already work

`a_read_of_four_beats_crosses_the_lattice`: a host at one corner reads
four beats from a memory at the far corner, through both bridges and
four nodes, and the four words come back in order with the right data.

The reason is structural rather than lucky.
A read crosses as one request packet, and every beat of the answer is
a packet of its own carrying the identifier and `last`, so nothing has
to be remembered between beats and no beat can be attached to the
wrong burst.
A write has the opposite shape: AXI4 puts no identifier on write data,
so a `W` beat means nothing except by its position.

That closes one line of issue 133 and removes it from the work of
issue 125.

### A switch still does not keep one source's packets together

Two measurements on one switch, both inputs leaving by the same port
(`two_inputs_of_a_switch_contend_for_their_shared_output`):

| Both inputs offer | What leaves by the shared output |
| --- | --- |
| every cycle | one from each, alternating, and never two together |
| one of them every third cycle | the two sources mixed, the sparse one's between the other's |

This corrects blocker 2 as issue 133 states it, twice over.
The blocker says beats from different sources interleave wherever
their paths merge.
Under the fixed order this replaced they did not: the higher input
took the port and the lower was starved, which is issue 315, and the
two only mixed while the higher paused.
Under the round robin they always mix, one each.
Either way a burst's beats do not stay together, which is what the
blocker is about, and the round robin makes it a certainty rather than
a matter of who is busy.

### A busy link starved its node's exit, and no longer does

`a_busy_link_and_the_exit_take_turns`: one switch, its north input and
its exit both offering every cycle for the same output.

```
before, 200 cycles: 198 packets from the link, 0 from the exit
after,  200 cycles: the two alternate, within one of each other
```

The choice among the inputs that can move was a fixed order, the four
links and then the exit, so an input that never paused kept its output
for ever.
That was issue 315, a defect in the tree rather than a consequence of
bursts, and it is fixed: the choice is a round robin, the first input
at or after the one whose turn it is, and the turn moves on to the one
after whichever moved.

It was also a prerequisite for everything below.
Every candidate makes a path busy for longer, so an unfair switch
would have turned a burst into a way of starving a node.

### What this tree's hosts actually send

Both hardware hosts issue single-beat bursts: `len: 0` at
`cpu/vreteno/src/core.rs:1077` and at `gpu/razboj/src/raster.rs:342`.
Nothing here needs a multi-beat write today.
The need is prospective, and it is named: direct memory access for
Ethernet and HDMI (issue 151), which moves frames rather than words.

That matters for the decision.
The first requirement is correctness for traffic that does not exist
yet, not peak bandwidth for traffic that does.

## 3. What the blockers come to

Issue 133 lists four. After the measurements:

1. **`HostBridge` keeps no state per burst.** Stands. Beats 2 to N
   carry no address, so the destination decoded from the address phase
   has to be remembered, and AXI4 allows `W` before `AW`.
2. **`Switch` arbitrates per packet and keeps almost no state.**
   Stands, in the corrected form above. It now holds the round robin
   of issue 315, three bits, which bounds the wait but keeps nothing
   about a burst: a burst's beats are still cut by another source's
   wherever their paths merge.
3. **Locking `PerBridge` to a source until its last beat deadlocks.**
   Stands as an argument, and the sheet states the mechanism: "A
   request waits while the identifier whose turn it is has not been
   answered. The bridge does not look for another free identifier."
   One queue, one head: a lock whose next beat is behind another
   source's packet waits for a packet that waits for the lock.
4. **Reassembly at `PerBridge` deadlocks once its slots are full.**
   Stands, for the same head-of-line reason, unless space is reserved
   before a burst's first beat enters the network.

## 4. The candidates, and what each costs

State is counted for the configuration `//soc` uses: `A` 32, `D` 32,
`S` 4, `I` 2, `XB` and `YB` 2, `NIDS` 4, and a length cap `L` of 16.
A data beat carried across is 37 bits, data and strobe and `last`,
which is the width of the `w` port on both bridges.
Today `HostBridge` holds 4 bits, `PerBridge` 30, and a switch 3.

The baseline is measured rather than asserted.
`//lib/parts:switch_synth` and `//lib/parts:node_synth` put one
through Vivado for the board's part, and `//docs:noc` quotes the
result: 271 look-up tables and 3 flip-flops for a switch, 554 and 6
for a node, with four or five levels of logic on the worst path.
Those three flip-flops are the round robin's `turn`, and they are all
the state a node has; before it was fixed, issue 315, a switch had
none at all and cost 266 look-up tables.
The comparison to make when a candidate is written is against those
numbers, on those targets.

| Candidate | Where the state goes | Bits added | Deadlock argument | Cuts across |
| --- | --- | ---: | --- | --- |
| **A. Wormhole on the request channel** | every switch: which input holds each output, and whether it is held | 40 per node (4 bits x 5 ports x 2 channels) | A held output waits on an input that may itself be blocked; needs the request channel to drain independently, which is what the second virtual channel gives, and needs the lock released on the last beat of a burst that is refused | a lock per output is state of a different order from the three bits the round robin keeps |
| **B. A, plus store-and-forward at `HostBridge`** | A, plus a burst buffer at the sending bridge | 40 per node, plus 37 x L = 592 at each host bridge | as A | does not replace A: collecting a burst before sending it does not stop another source cutting into it downstream |
| **C. Reassembly at `PerBridge` with end-to-end credits** | the receiving bridge: a slot per beat per outstanding burst, plus credits | 37 x L x NIDS = 2368 at each peripheral bridge, plus 4 x 5 credit bits, plus grants on the response channel | credits reserve the slots before the first beat enters, so a burst never occupies the network waiting for room | the true answer, and the only one that tolerates arbitrary reordering; two orders of magnitude more state than the others |
| **D. Splitting at `HostBridge`** | the sending bridge: beats left, the running address, the answer so far | 44 per burst in flight (8 + 32 + 2 + 2) | nothing is held anywhere: each beat is an independent single-beat write, which is what the network already carries | a peripheral that distinguishes one burst of N from N writes of one sees the difference |
| **E. A request channel per source** | every link and every switch | a fifth set of ports per node, 113 bits of packet each | avoids the merge entirely | by far the largest, and it does not scale with the number of hosts |

Latency, for a burst of `N` beats between two corners of a two by two
lattice, counting a hop as a cycle:

* today, N separate single-beat writes from the host: `N` x (2 hops
  out, 2 back), pipelined to the extent the host's tracker has
  identifiers, which is `NIDS`;
* A or B: 2 hops out for the first beat and one cycle per beat after
  it, then 2 back for the response: the best of the five;
* C: as A, plus the wait for credits when the peripheral is busy;
* D: exactly today's, because it is today's, with the splitting done
  by the bridge rather than by the host.

## 5. The cap

AXI4 allows 256 beats for `INCR`, 16 for `FIXED` and `WRAP`, and no
burst may cross a 4 KiB boundary.
Nothing in this tree sends more than one beat, and the traffic that
will is a DMA engine moving a frame, which chooses its own burst
length.

So the cap is a parameter, `L`, and the policy above it is `SlvErr`
rather than splitting: a host that asks for more than the network was
built for should be told, not quietly served differently.
With D the cap can be large, since it costs nothing but a wider
counter; with C it is what sizes the reassembly buffer, and 16 is
already 2368 bits.

## 6. The decision

**D, splitting at `HostBridge`, with a cap and `SlvErr` above it.**
Issue 315, the starving arbiter, was the prerequisite and is fixed.

The reasons, in order:

* It needs no ordering mechanism anywhere, because it creates no
  ordering requirement: each beat is a single-beat write, which is
  exactly what this network carries correctly today and what both of
  its hardware hosts already send.
* It is 44 bits in one unit against 40 per node for A, 592 more for B
  and 2368 per peripheral bridge for C.
* It touches neither the switch nor `PerBridge`, so the parts with the
  deadlock arguments stay as they are.
* It lands in steps that each stand: the address arithmetic and the
  merged response first, `FIXED` and `WRAP` after, the cap last.

What it gives up, said plainly: a peripheral that means something by a
burst, a DRAM controller opening a row or a device with a side effect
per transaction, sees N transactions instead of one.
Every peripheral in this tree is a memory or an AXI-Lite bridge, for
which the two are identical.
When that stops being true, C is the answer, and its cost is in the
table above rather than to be worked out again.

`FIXED` becomes N writes to the same address, which is what `FIXED`
means.
`WRAP` needs the wrap computed where the splitting happens, and until
it is, it is refused as a long write is refused now.

## 7. The order for issue 125

1. Issue 315, the round robin in the switch. Done, and it came first
   because every step after it makes a path busier.
2. `HostBridge` splits an `INCR` write of `N` beats into `N` packets,
   each a single-beat write at its own address, and answers the host
   once, with the worst response it saw. **Done.** The phase is held
   in registers, 8 for the beats left, `A` for the address, `I` for
   the identifier, one for fixed, and 18 for the size and hints sent
   again with each beat; the merge is 8 for the answers still to come
   and one for whether any was an error. One burst is split at a time
   and the next long write waits for its answer; single-beat writes
   and reads do not. The answer is `SlvErr` if any beat's was not
   `Okay`, so a `DecErr` comes home as `SlvErr`. The address moves by
   the link's width, `S` bytes, and not by `size`: nothing in this
   tree acts on `size`, the memory model steps by the width, and the
   simulation client leaves it at zero, which is its own issue.
3. The cap `L` as a parameter, with `SlvErr` above it. **Not built,
   on purpose.** Under D there is no buffer for a cap to size: the
   counter is 8 bits whichever cap is chosen, since `len` is, and a
   burst of 256 beats costs the bridge nothing a burst of two does
   not. A parameter would be policy without a mechanism behind it,
   and `HostBridge` has twenty parameters already. The 4 KiB rule is
   the host's, as it is on any AXI link. If a cap is wanted later it
   is one comparison in `refuse`.
4. `FIXED`, which is the same address each time: **done**, the
   address does not move. `WRAP` is **still refused**, as every long
   write was, since the wrap is not computed at the bridge.
5. Tests: **done** for the network,
   `two_hosts_write_sixteen_beats_at_once_and_every_word_lands`,
   `a_write_of_two_beats_crosses_the_lattice_as_two_writes`,
   `a_fixed_write_lands_every_beat_on_one_word` and
   `a_wrapping_write_is_refused_and_the_one_after_it_is_served`, all
   in `lib/parts/src/bus/noc.rs`. No waveform lowers the bridges, so
   there is still no co-simulation of them; the showcase lists the
   network's netlists as not yet simulated, and that is unchanged.
6. `docs/noc.tex` loses its line about multi-beat writes not crossing,
   and says what happens instead. **Done**, with the `HostBridge`
   sheet.

## 8. What was not done

D is written and not yet synthesised: there is no `host_bridge_synth`
target beside `switch_synth` and `node_synth`, and adding one is the
next number this note wants.
The state above is counted from the design and from the widths the
generated tables give, which is exact for flip-flops and says nothing
about the logic around them.

The baseline to measure against is already in the tree, and it is
where the next number should come from:
`//lib/parts:switch_synth` and `//lib/parts:node_synth`, both manual,
both through Vivado for the board's part.
When step 2 of the order above is written, the same targets applied to
the bridges say what it cost, and that number belongs in this note
under the table.
