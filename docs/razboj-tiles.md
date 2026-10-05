# Razboj in tiles: depth and blending without DDR3 traffic

Issue 991, item 11 of `opengl-gap.md`.
The user decided on October 4, 2026 that Razboj draws in tiles: colour and depth for a tile live in block RAM, and finished tiles go to DDR3 in long bursts, so that depth testing and blending cost no DDR3 traffic.
This note designs that.
It settles the tile size, how the CPU sorts primitives into tiles, the block RAM a tile takes beside the other users of the part, and how finished tiles reach DDR3 given what issue 1023 measured.
It leaves the depth test itself to issue 992 and blending to issue 993, and says in section 8 what each of them inherits from here.

## 1. What tiles change, and what they do not

Today Razboj writes every pixel it draws to memory as its own one-beat burst (`gpu/razboj/src/raster.rs`), and it reads nothing back.
A depth test reads the depth at a pixel before deciding to write, and blending reads the colour; at one pixel a cycle and 100 MHz that is 800 MB/s for depth and colour before blending and 1.2 GB/s with it (`opengl-gap.md`, section 6).
The link into DDR3 carries 400 MB/s at most, and today's path reads about 15 MB/s and, by its own arithmetic, writes about 66 MB/s (`docs/ddr3-throughput.md`, in pull request 1046 for issue 1023, sections 1 and 6).

In tiles, every one of those reads and writes is to block RAM beside the rasteriser, a cycle away.
DDR3 then sees exactly one write per pixel per frame, the finished colour, whatever the overdraw, the depth test and the blending did on the way.
At 640 by 480 that is 1.2 MB a frame, 74 MB/s at 60 frames a second: the same as the scanout reads.

Three things do not change.
The rasteriser still walks one box per entry at one pixel a cycle.
The instruction format and the model stay as they are, apart from what issue 992 adds for depth.
And the path into DDR3 still has to be widened by issue 1023 before anything runs at a frame rate, because the scanout alone needs 74 MB/s of reads and today's path gives 15.
Tiles make Razboj's own traffic small and fixed; they do not make the path wider.

## 2. The tile size: 64 by 64

A tile is square and a power of two on a side, so that a pixel's place in the tile is the low bits of its screen coordinates and costs no arithmetic.
The candidates are 32, 64 and 128.

| Tile | Tiles in 640 by 480 | Block RAM tiles, one bank | Both banks | Entries for a box of 16 / 64 / 200 pixels |
|---|---|---|---|---|
| 32 by 32 | 300 | 1.5 | 3 | 2.3 / 9 / 53 |
| 64 by 64 | 80 | 6 | 12 | 1.6 / 4 / 17 |
| 128 by 128 | 20 | 24 | 48 | 1.3 / 2.3 / 6.6 |

A bank is one tile's colour, 32 bits a pixel, and depth, 16 bits a pixel; section 3 says why there are two.
A 36 Kbit block RAM holds 1024 words of 32 bits or 2048 of 16, so a 64 by 64 tile's colour is four of them and its depth two.
The last column is how many tiles, and so how many binned entries, a primitive's box touches on average at a random position: about `(w/T + 1)(h/T + 1)` for a box `w` by `h` and tiles `T` on a side.
640 is ten tiles of 64 across and 480 is seven and a half down, so the last row of tiles is 32 pixels high, which the clipping in section 4 handles as it handles the screen's edge now.

Every entry a primitive is binned into costs Razboj one fetch of its sixteen words and the three cycles of the setup, whatever the size of its part of the box.
The fetch is the expensive part: through today's path a word is about 27 cycles, so an entry is about 430 cycles; after issue 1023 it is one burst, about the controller's latency of 23 cycles and sixteen beats.
So small tiles multiply the fixed cost per primitive, and large tiles multiply the block RAM.

64 by 64 is the choice.
Against 32 it bins a mid-sized primitive into half as many entries or fewer, and a large one into a third.
Against 128 it saves 36 block RAM tiles for at most a factor of two or three in entries, and the block RAM is the one thing here that every later user of the part also wants (section 6).
It also makes a tile row exactly 256 bytes, which section 5 uses.

## 3. The tile buffer

Two banks, each a tile's colour and depth.
Razboj draws into one while the other is written out to DDR3, and they swap when both are done.
A pixel's address in a bank is `{y[5:0], x[5:0]}`, twelve bits taken straight from the walk's screen coordinates, since tiles start at multiples of 64.

Each bank's block RAMs have two ports, and each port has one job.

* **The draw port** reads a pixel's depth and colour and writes them back a few cycles later, so the depth test and blending are a read, a decision and a write, pipelined at one pixel a cycle.
  Two pixels in flight at the same address, which a primitive drawn over itself never makes but two consecutive entries can, are the hazard issue 992 has to forward or stall on.
* **The write-out port** reads the finished tile a word a cycle for DDR3 and, in the same cycle, writes the clear values into the word it read.
  A 7 series block RAM port in read-first mode returns the old word and stores the new one in one cycle, so clearing the tile for its next use costs nothing.
  That needs the bank's memory to lower to read-first on that port, which step 2 of section 7 checks in the netlist before relying on it.

So a frame's clear is not an entry at all in the common case.
GL ES programs clear at the start of almost every frame, and the clear colour and clear depth become two registers the write-out port uses.
Only the first use of each bank in a frame needs a clear pass of its own, 4096 cycles, unless the bank was emptied with the same clear values at the end of the last frame.
A frame that does not begin with a clear has to load each tile from DDR3 before drawing it; that is the slow path, and it is left for when a program needs it.

The alpha that issue 990 put into the pixel's top byte lives in the bank's colour word as it does in memory.
The scanout drops that byte (`lib/parts/src/scanout.rs`, `VidMux`, `resize::<24>`), so nothing the screen shows changes.

## 4. Binning on the CPU

The CPU sorts primitives into tiles, because the display list already comes from the CPU and the CPU knows which primitives there are.
Razboj stays a machine that draws what it is given.

**Clipping to a tile is the scissor box.**
Issue 990 makes the scissor test a clip of the entry's box in the assembler, `Op::encode_in`; its pull request is open.
A tile is a scissor box, so binning a primitive is clipping its box to each tile it touches and writing one entry per tile.
The rasteriser walks only the clipped box and tests the edges against the same vertices, so the pixels it draws in a tile are exactly the pixels of the whole primitive that lie in the tile.

**The planes are stepped, not worked out again.**
A shaded triangle's planes are a value at the box's first pixel and two steps.
For the tiled picture to be the untiled one bit for bit, each tile's start has to be the untiled start stepped to the tile's first pixel, `start + dx * i + dy * j` in wrapping 32-bit arithmetic, which is what the rasteriser's own adders would have reached there.
Working the plane out afresh at each tile's first pixel, as `encode_in` does today, rounds at a different place and can differ in the last bit of a fraction and, rarely, a byte.
So the binning library computes each plane once per primitive and steps it per tile; that is also cheaper, two multiplications and two additions a channel against a division.
The edges need nothing: the rasteriser's setup computes them from the vertices at the clipped box's first pixel, exactly.

**What the CPU writes.**
A frame becomes a table of tiles, each naming its origin, where its entries start and how many there are, and the count the rasteriser polls becomes the table's length.
The format is the implementation's to fix, and it is stated once.
It is in `gpu/razboj/tile/lib.rs`, the crate `razboj_tile`, rather than beside the instruction's in `gpu/razboj/src/dl.rs`: a Vreteno program writes the table, and `dl.rs` sits in a crate with the standard library, which a program on the core cannot use (issue 1157).
A binning library in Rust, usable from a Vreteno program and from the model, does the sorting, so that the same code that the board runs is what the tests check.

**What it costs the CPU.**
The board measured sixteen stores from the core in 250 cycles to either memory (issue 1023's note, section 6), about sixteen cycles a word, the core's own pace.
A binned entry is sixteen words, so about 250 cycles to write and some fifty more to clip and step, about 3 microseconds at 100 MHz.
A thousand triangles with boxes of 16 pixels make about 1600 entries in 64 by 64 tiles, about 5 ms of the core's time a frame; the icosahedron's twenty faces, a few tiles each, take about a millisecond.
That is fast enough for the first stage, and if a scene ever makes the CPU the limit, binning can move into hardware as item 17 of `opengl-gap.md` proposes for the transform.

**Where the bins live.**
In DDR3, beside the framebuffers, since a frame's bins are tens to hundreds of kilobytes and the core's own data memory is a few: four in `soc`.
Razboj reads them with the bursts it already issues.

## 5. Finished tiles to DDR3

A finished tile goes out a row at a time, one AXI write burst of 64 beats for each of its 64 rows.
A tile row is 256 bytes, and it starts at a multiple of 256 bytes as long as the framebuffer's rows do, since the tile's left edge is a multiple of 64 pixels.
Razboj's board parameters make a row 1024 pixels, 4 KB, so with the framebuffer's base on a 4 KB boundary, which the driver chooses, no burst crosses the 4 KB boundary AXI forbids.
That replaces Razboj's one-beat burst a pixel, which is item 4 of `opengl-gap.md`, a span written in bursts, done for every pixel at once.

What a frame costs, at 640 by 480:

| Path | A word | A tile | A frame | Frames a second, write-out alone |
|---|---|---|---|---|
| Today, about 66 MB/s writes | about 6 cycles | 246 microseconds | 18.4 ms | about 54 |
| After issue 1023, at the link's 400 MB/s | about 1 cycle | 41 microseconds | 3.1 ms | over 300 |

Today's write figure is the wrapper's arithmetic rather than a board measurement: the core cannot fill the path, so the board has not yet measured a write-bound rate (issue 1023's note, section 6).
Write-out overlaps drawing, since the other bank is being drawn meanwhile, so a frame takes about the longer of the two, tile by tile.
At the link's rate a tile's write-out is 4096 cycles, the same as drawing a tile whose every pixel is covered once.

The scanout's 74 MB/s of reads and Razboj's 74 MB/s of writes at 60 frames a second together are about 150 MB/s, which fits the 400 MB/s of the link once issue 1023 has widened the path behind it, and does not fit today's.
So the first frame-rate picture from tiles waits on issue 1023, as the scanout already does.
A tiled Razboj can be built, checked against the model and run on the board at a low frame rate before that.

Double buffering is unchanged: Razboj draws a frame into one framebuffer while the scanout shows the other, and the program swaps them at the vertical sync once Razboj reports the frame done, which is item 6 of `opengl-gap.md`.

## 6. The block RAM budget

The XC7A200T has 365 block RAM tiles of 36 Kbit.

| User | Block RAM tiles | Source |
|---|---|---|
| The flagship as synthesised | 16.5 | `docs/flagship.tex`, "What it costs" |
| of which the video peripheral's own framebuffer | 12 | `docs/hdmi.tex`; `docs/flagship_place.tsv` has video at 12 |
| The scanout's two lines of 640 words, when it joins the flagship | about 2 | `lib/parts/src/scanout.rs`, `LinePair`; estimated, not yet placed |
| Razboj's two banks of 64 by 64 | 12 | Section 2 |
| Total | about 31, under 9 per cent | |

The part is not the constraint, as the flagship document already says of the rest.
What the budget does decide is the tile size: 128 by 128 would take 48 for Razboj alone, and the remaining users of block RAM are all still to come, a texture cache for item 14 first among them.
Once the scanout shows a framebuffer in DDR3, the video peripheral's 12 tiles of its own framebuffer are no longer needed for that picture, and they come back.
A 24-bit depth would take three block RAM tiles a bank where 16-bit takes two, 14 in all.

## 7. The order to build it in

Each step is a pull request, and each is checked against the model before the next.

1. **The binning library and its test.**
   Done in issue 1157: the crate `razboj_tile` and the tests in `gpu/razboj/src/tiles.rs`.
   `gpu/razboj` gains the tile table's format and a binning library that clips to tiles and steps the planes.
   The test renders scenes binned into tiles with the model and asserts the picture equals the model's untiled picture, pixel for pixel, including shaded triangles across tile edges and the mesh of issue 988 that draws every pixel once.
   No hardware changes, so this lands first and fixes the format.
2. **The tile buffer and the write-out.**
   Razboj walks the tile table, draws each tile's entries into a bank, writes finished tiles out a row a burst and clears as it reads.
   No depth yet: it draws exactly what it draws now, and every existing test, run through the binning library, still passes.
3. **The depth test, issue 992**, on the draw port, with the depth plane and the read-modify-write pipeline.
4. **Blending and the masks, issue 993**, on the same port, reading the colour the depth test already reads beside it.

## 8. What this leaves to issues 992 and 993

* **Depth needs three more words of the instruction.**
  A depth plane is a value and two steps, as a colour channel is, and the sixteen-word instruction has no word left after issue 990.
  Issue 992 chooses between a 32-word stride with a longer fetch for every entry, and a fetch whose length the kind decides, which costs the front end a second burst for the kinds that need it.
* **The hazard of two pixels at one address in flight**, which a read-modify-write pipeline has and today's write-only walk does not.
* **Depth's width and range.**
  16 bits is the assumption above, from section 6 of `opengl-gap.md`; section 6 here says what 24 would cost.
