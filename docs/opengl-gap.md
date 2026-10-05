<!-- SPDX-License-Identifier: Apache-2.0 -->
# Razboj and OpenGL: what is missing, and a path there

Status: analysis, October 4, 2026, for issue 926.
Read against commit `467ff15`; file paths and line numbers are at that commit.
Author: automated coding assistant, with human supervision.

This note answers issue 926: what Razboj, the GPU under `//gpu/razboj`, would need before a program could draw with OpenGL.
It compares what Razboj does with what OpenGL ES 1.1 asks for, then with a subset of OpenGL ES 2.0.
It lists the gaps, sizes each one, and proposes an order to close them in.
The issue also asks for the flagship to carry Razboj and for the icosahedron to be drawn by it; section 7 covers both, as the first stage.
The issue asks for an issue per action, with order labels, and a project.
Section 8 lists those issues, ready to file; none is filed until the user has read this.

## 1. The answer, short

OpenGL ES 1.1 is reachable, in stages, with most of the geometry in software on Vreteno and the per-pixel work in Razboj.
ES 1.1 has a profile, Common-Lite, whose arithmetic is all fixed point, and Vreteno has no floating point, so that profile fits the machine as it is.
A full ES 2.0 is a different machine: it needs floating point in programmable shader cores.
A subset of 2.0 is possible later, with shaders compiled offline on the host, which ES 2.0 permits.

The first stage is short and is what the issue asks for first.
Razboj goes into the flagship, draws into DDR3, and the scanout of issue 151 shows what it drew; then the icosahedron is drawn by Razboj on the board.
Nothing in that stage is OpenGL yet, but every later stage needs it.

## 2. What Razboj is today

Razboj is a rasteriser and an AXI host (`gpu/razboj/src/raster.rs`), with a framebuffer for its runs (`gpu/razboj/src/fb.rs`).
Its document is `//docs:razboj`.

**The instruction set.**
Three kinds of entry: clear, rectangle and triangle (`gpu/razboj/src/op.rs`).
Each entry has one colour, 24 bits, `0xRRGGBB` (`op.rs:67`).
A triangle carries three vertices of 12 bits each, signed, in whole pixels (`op.rs:78`, `op.rs:91`).
A rectangle or a triangle carries a box of 10-bit screen coordinates (`op.rs:71`), so the screen is at most 1024 by 1024.
The host assembles the list: it clips the box to the screen and winds each triangle so that its inside is where all three edge functions are non-negative (`Op::encode`).
An entry is eight words in memory, six of them used (`gpu/razboj/src/dl.rs`).

**The walk.**
Per triangle, the setup does six 32-bit multiplications in one cycle.
Then the walk visits every pixel of the box, one per cycle at best, with three additions and three sign tests per pixel.
A pixel is inside when no edge function is negative, sampled at the pixel's integer corner.
There is no fill rule, so a pixel on an edge two triangles share is drawn by both.

**The memory side.**
Each pixel inside the primitive is one AXI write of one beat (`raster.rs:424`).
Nothing is read from the framebuffer, so there is no depth test and no blending.
The display list is read one word at a time, six reads per entry (`raster.rs:247`).
The list's length is an 8-bit count, so at most 255 entries (`raster.rs:108`).
When the list is drawn, Razboj sets `finished` and waits for ever (`raster.rs:469`): it draws one list per reset.

**What it has been checked by.**
A model written with loops, the run against it, and its netlists under nvc and Verilator.
It has never been synthesised: no Vivado target names it.
`//docs:razboj` names what it is not, in its section "What It Is Not": one pixel per cycle, no depth buffer, no interpolation, no texture, no clipping of geometry, and one read out at a time.
Its framebuffer for the runs holds one write at a time, so the demonstration draws at about three cycles per pixel (`docs/razboj.tex:263`).

## 3. The system around it

**Where Razboj runs now.**
Only in simulation.
`//soc` puts it on a two by two network on chip with Vreteno, a memory and a serial port.
The memory there is the simulation-only `Ram` (`soc/src/lib.rs:18`), and the screen is 128 by 96 (`soc/src/lib.rs:60`).
In that system `cpu/vreteno/rust/ico.rs` turns an icosahedron, culls its back faces, lights each face, and writes the faces as a display list; Razboj fills them.
That program already does in software what ES 1.1 calls transform and lighting, in fixed point with eight fractional bits (`ico.rs:19`).

**The icosahedron on the board.**
That one is a different program, `cpu/vreteno/rust/ico_hdmi.rs`, and Razboj is not in it.
The core draws every pixel itself into the HDMI peripheral's framebuffer: 160 by 120, each pixel four by four on the 640 by 480 screen (`ico_hdmi.rs:14`).

**The HDMI framebuffer.**
It is block RAM inside the video peripheral (`lib/parts/src/hdmi.rs:33`): 12-bit pixels, at most 256 by 128.
A host writes it a pixel at a time through a cursor register over AXI-Lite (`hdmi.rs:140`).
Razboj writes 32-bit pixels at addresses over AXI4, so it cannot draw into this framebuffer.

**The scanout, issue 151.**
`lib/parts/src/scanout.rs` reads each line from DDR3 a line ahead of the beam.
A scanout pixel is one 32-bit word, `0x00RRGGBB` (`scanout.rs:394`), at the full 640 by 480.
That is Razboj's pixel exactly: the same word, the same byte order.
The frame's base address is a register taken at the vertical sync (`scanout.rs:84`), so software can draw into one buffer while the other is shown, and swap them with one write.
The distance in bytes from one line to the next is a parameter, `STRIDE`, so Razboj's rows, a power of two wide (1024 pixels for a 640-pixel screen), need no change on either side.
`docs/board-checks.md` says nothing yet joins the scanout to the video peripheral in a bitstream; issue 151 is doing that now, and on the way found issue 979, two netlists in one design each defining `txhdl_chan`.

**The core.**
Vreteno is RV32IMAC at 100 MHz, with multiply and divide as a multi-cycle sequence and no floating point (`//docs:vreteno`).
Both icosahedron programs work in fixed point for that reason (`ico_hdmi.rs:35`).

**The memory.**
A gigabyte of DDR3 behind AMD's MIG, reached through the bridge from AXI to Wishbone, which takes one burst at a time (`lib/parts/src/bus/wb.rs:7`).
The memory is two x16 chips on one command bus, so one memory 32 bits wide, not two: its peak at 400 MHz, on both edges, is 3.2 GB/s (issue 1040; this said 16 bits and 1.6 GB/s at first).
What stands in front of it is far slower: the bridge has one access in flight, and the wrapper before the controller spends a whole controller burst of eight words on each 32-bit word (issue 1023).

**The room on the part.**
The flagship uses 10,999 slice LUTs (8.2 per cent of the XC7A200T), 16.5 of 365 block RAM tiles and four DSP blocks (`docs/flagship.tex:450`).
The part has 740 DSP blocks.
Room is not what limits a GPU here; memory bandwidth is.

## 4. What OpenGL ES 1.1 asks for

OpenGL ES 1.1 is the fixed-function API: no shaders, a pipeline whose stages are switched on and off and configured.
It has two profiles.
Common takes floating point; Common-Lite takes only fixed point, 16.16, in the type `GLfixed`.
Common-Lite is the profile for a machine without floating point, which is this one.

The table follows the pipeline in order.
"Where" says where the work would sit in the proposal of section 6: on the CPU (Vreteno, in a library), in Razboj, or in the scanout.

| Stage | ES 1.1 asks for | Razboj and the system today | Where it would go |
|---|---|---|---|
| Vertex input | Vertex arrays (position, colour, normal, texture coordinates), `glDrawArrays` and `glDrawElements` | A display list of filled shapes the host writes | CPU library |
| Transform | Modelview, projection and texture matrix stacks; `glOrtho`, `glFrustum`, `glRotate` and the rest | `ico.rs` does one fixed rotation and a perspective divide by hand | CPU library |
| Lighting | Per-vertex lighting: at least eight lights, materials, ambient, diffuse, specular | `ico.rs` lights each face with one light at the eye | CPU library |
| Primitives | Points, lines, line strips and loops, triangles, triangle strips and fans | Triangles and rectangles; no points, no lines | CPU expands strips and fans; Razboj draws points and lines |
| Clipping | Against the view frustum, and at least one user clip plane | The host clips the box to the screen; a vertex must lie within 12 bits | CPU library |
| Culling | `glCullFace`, `glFrontFace` | `ico.rs` culls by the face normal; Razboj draws either winding | CPU library |
| Viewport | Viewport transform, depth range | None | CPU library |
| Rasterisation | Sub-pixel vertex positions, a fill rule, flat and smooth (Gouraud) shading, polygon offset | Whole-pixel vertices, no fill rule, one colour per primitive | Razboj |
| Texturing | 2D textures, at least two texture units, nearest and linear filtering, mipmaps, repeat and clamp, texture environments (modulate, replace, decal, blend, add, combine), the paletted compressed formats | None | Razboj, with a texture cache |
| Fog | Linear, exponential and squared exponential | None | Razboj |
| Per-fragment tests | Scissor, alpha test, stencil test, depth test | None; every pixel inside is written | Razboj |
| Blending and the rest | Blending, dithering, logic operations, colour and depth masks | None; a write replaces the pixel | Razboj |
| Framebuffer | Colour with alpha, depth, optionally stencil; `glClear` of each; `glReadPixels` | 24-bit colour, no alpha, no depth | Razboj and DDR3 |
| Display | EGL surfaces and contexts, double buffering, `eglSwapBuffers` | One buffer; one list per reset | Scanout (base flip) and a driver |
| Points | Point sprites and point size arrays, required in 1.1 | None | Razboj |

Two notes on the table.
A depth buffer is optional in the sense that an EGL configuration may have none, and so is stencil, but a GL with no depth test draws little that a user expects, so this note treats depth as required.
Precision is a requirement too: `GL_SUBPIXEL_BITS` must be at least 4, and Razboj's whole-pixel vertices have none.

## 5. What a subset of OpenGL ES 2.0 adds

ES 2.0 removes the fixed-function pipeline and replaces it with two programs a user writes in GLSL ES 1.00: a vertex shader and a fragment shader.
Everything in the transform, lighting, texturing and fog rows of section 4 becomes shader code.

What it asks of the hardware:

* **Floating point in both shaders.**
  A vertex shader must have `highp` float: a relative precision of 2^-16 over a range of 2^62, which is single precision in practice.
  A fragment shader may have only `mediump`: relative precision 2^-10 over a range of 2^14, which is half precision.
  Fixed point in 32 bits cannot reach `highp`'s range, and keeps `mediump`'s relative precision only for values well away from zero.
* **Limits a conforming implementation must reach.**
  At least 8 vertex attributes, 128 vertex uniform vectors, 8 varying vectors, 16 fragment uniform vectors and 8 texture units in the fragment shader; texture units in the vertex shader may be zero.
* **Framebuffer objects**: rendering into a texture or a renderbuffer, part of 2.0 itself rather than an extension.
* **A compiler, or not.**
  An implementation may leave out the shader compiler (`GL_SHADER_COMPILER` false) and accept only shader binaries through `glShaderBinary`.
  That lets the compiler run on the host, at build time, and is how a small implementation ships 2.0.

So a 2.0 subset needs shader processors with floating point, which is an XL item on its own (section 6).
A natural shape for them here is several small RISC-V cores with the single-precision float extension, `F` or `Zfinx`, fed by a fixed-function rasteriser that is Razboj grown up.
Shaders compiled offline to RISC-V code would then be ordinary programs; what is new is the work distribution, not the instruction set.

## 6. The gaps, ranked by effort

Sizes are relative, by what this repository's work has taken before.
**S** is one pull request; **M** is a few, a week or so of one session; **L** is a subsystem with its own document; **XL** is a project of its own.

| # | Gap | Size | Why that size |
|---|---|---|---|
| 1 | Draw a list, then another: `finished` clears when a new count is written, and Razboj raises a "done" line | S | One state change in `raster.rs` and a status bit |
| 2 | More than 255 entries: a wider count | S | A field width, and the format in `dl.rs` |
| 3 | Fetch an entry in one burst of six or eight beats, not six single reads | S | The fetch loop at `raster.rs:247` becomes one read burst |
| 4 | Write a span in bursts: consecutive pixels of a row go out as one multi-beat burst | M | DDR3 behind one-burst-at-a-time Wishbone cannot take one beat per pixel at speed |
| 5 | Razboj synthesised, then in the flagship, writing DDR3, shown by the scanout | M | A Vivado target, an arbiter port, the address map; waits on issue 151 and issue 979 |
| 6 | Double buffering: draw into one buffer while the scanout shows the other | S | The scanout already flips at vsync; software and an address field |
| 7 | Sub-pixel vertices and a top-left fill rule | M | Wider vertex fields, the setup's fixed point, and a test that no pixel is drawn twice |
| 8 | Points and lines | M | Two new walks, or lines as thin quads from the CPU |
| 9 | Per-vertex colour, interpolated: Gouraud shading | M | Three more edge-like interpolants, one per channel |
| 10 | Alpha in the pixel, scissor | S | Spare bits in the pixel word, a box test |
| 11 | Depth buffer and depth test | L | Read before write; the bandwidth question below decides the design |
| 12 | Blending, alpha test, colour and depth masks | M | Once depth reads the framebuffer, colour reads it the same way |
| 13 | A `GLES_CM` library on Vreteno: the API, matrix stacks, transform, lighting, clipping, viewport, building Razboj's lists | L | The largest piece of software here; Common-Lite, fixed point |
| 14 | Textures: texture coordinates interpolated with perspective, a texture cache over DDR3, nearest then bilinear, the texture environments | L | A per-pixel reciprocal, a cache, a second memory stream |
| 15 | Stencil, fog, logic operations, dithering, polygon offset, point sprites, paletted textures | M each | Each is small once 11, 12 and 14 exist |
| 16 | EGL on Zephyr: surfaces, contexts, `eglSwapBuffers` waiting for the vsync | M | A driver in `zephyr/drivers`, beside the console and Ethernet |
| 17 | A geometry unit in hardware: matrix multiply and divide per vertex | L | Only when the CPU becomes the limit; 740 DSP blocks are there |
| 18 | Running the Khronos conformance tests for ES 1.1 | L | The tests need an operating system and a file system the board does not have yet |
| 19 | Shader processors with floating point, and a work distributor, for an ES 2.0 subset | XL | A new machine |
| 20 | GLSL ES compiled offline to the shader processors' code, through `glShaderBinary` | XL | Mesa's GLSL front end is the likely starting point |

**The bandwidth question, which item 11 turns on.**
At one pixel per cycle and 100 MHz, a depth test reads 2 bytes and writes 2 more, and the colour write is 4: 800 MB/s before blending, which adds a 4-byte read.
The scanout takes about 74 MB/s for 640 by 480 at 60 frames a second.
Against DDR3 at 3.2 GB/s peak the arithmetic would allow it, but not through the path in front of the memory today, one access in flight and a controller burst spent on each word, so per-pixel reads and writes to DDR3 will not run at one pixel per cycle until issue 1023 widens that path.
Two designs avoid it.

* **Depth in block RAM.**
  A 16-bit depth buffer for 640 by 480 is about 4.9 Mbit, about 134 of the 365 block RAM tiles.
  It fits, and DDR3 then carries only colour.
* **Tiles.**
  Razboj draws the screen one tile at a time, 64 by 64 say, with colour and depth for the tile in block RAM, about six block RAM tiles for 32-bit colour and 16-bit depth, and writes each finished tile to DDR3 in long bursts.
  The CPU sorts each primitive into the tiles it touches.
  This is how most mobile GPUs work, it makes blending and depth free of DDR3 traffic, and a display list is already the input it wants.

This note recommends tiles, decided when item 11 is designed, not now.

## 7. A staged path

Each stage ends with something drawn on the board, and each later stage builds on the earlier ones.

**Stage 0: Razboj on the board, drawing the icosahedron.**
The issue's first two points.
Items 1, 2, 3, 5 and 6, then item 4 if the frame rate needs it.
Razboj goes into the flagship as a second host beside the core, with its framebuffer in DDR3 at an address the scanout of issue 151 shows.
`ico_hdmi.rs` stops drawing pixels: it computes the faces as `ico.rs` does and writes a display list per frame, double buffered, and the picture is 640 by 480 at full resolution instead of 160 by 120.
Depends on issue 151 (the scanout in a bitstream) and issue 979 (two netlists in one design).
Done when the board shows the turning icosahedron drawn by Razboj, and the time per frame is measured against `ico_hdmi.rs` drawing it on the core.

**Stage 1: the rasteriser a fixed-function GL needs, without textures.**
Items 7, 8, 9, 10, 11 and 12.
Done when a scene with overlapping, Gouraud-shaded, depth-tested and blended triangles matches Razboj's model pixel for pixel, and runs on the board.

**Stage 2: a GL ES 1.1 Common-Lite library on Vreteno.**
Items 13 and 16.
The library is no_std Rust or C, links into a Zephyr image or a bare program, and draws through Razboj.
Done when the icosahedron program is rewritten against `glDrawElements`, `glFrustum` and `glLight`, and draws the same picture.

**Stage 3: textures.**
Item 14, then the rest of item 15.
Done when a textured, mipmapped, perspective-correct quad draws on the board, and the texture environments match a software reference.

**Stage 4: conformance and speed.**
Item 18 as far as the board allows, and item 17 if the CPU is the bottleneck by then.

**Stage 5: an ES 2.0 subset.**
Items 19 and 20.
To be planned in a document of its own once stage 3 is done; nothing before it depends on the choices it makes.

## 8. The issues to file, when the user agrees

The issue asks for one issue per action, with order labels, and a project to hold them.
The titles below are in the repository's style; `order/N` is the order to work them in.
Stage 5 is one issue for now, a design document, rather than a list nobody could yet order.

| Order | Title | Gap |
|---|---|---|
| order/1 | `feat(razboj): draw another list after the first, and say when one is done` | 1 |
| order/2 | `feat(razboj): fetch a display list entry in one burst, and count more than 255` | 2, 3 |
| order/3 | `build(razboj): a Vivado synthesis target, and its size and timing` | 5, first half |
| order/4 | `feat(flagship): Razboj as a second host, drawing into DDR3 for the scanout` | 5, second half; after 151 and 979 |
| order/5 | `feat(vreteno): the icosahedron drawn by Razboj on the board, double buffered` | 6, the issue's second point |
| order/6 | `feat(razboj): write a run of pixels as one burst` | 4 |
| order/7 | `feat(razboj): sub-pixel vertices and a top-left fill rule` | 7 |
| order/8 | `feat(razboj): Gouraud shading, a colour per vertex` | 9 |
| order/9 | `feat(razboj): alpha in the pixel, and a scissor box` | 10 |
| order/10 | `docs(razboj): depth and blending, in DDR3 or in tiles` (a design note) | 11, the decision |
| order/11 | `feat(razboj): a depth buffer and a depth test` | 11 |
| order/12 | `feat(razboj): blending, alpha test and masks` | 12 |
| order/13 | `feat(razboj): points and lines` | 8 |
| order/14 | `feat(gles): a GL ES 1.1 Common-Lite library on Vreteno, without textures` | 13 |
| order/15 | `feat(zephyr): EGL for Razboj, with eglSwapBuffers on the vsync` | 16 |
| order/16 | `feat(razboj): textures, perspective-correct, through a texture cache` | 14 |
| order/17 | `feat(razboj): fog, stencil, logic operations, dithering, polygon offset, point sprites` | 15 |
| order/18 | `test(gles): the ES 1.1 conformance tests, as far as the board allows` | 18 |
| order/19 | `docs(gpu): an ES 2.0 subset, shader processors and an offline compiler` | 19, 20 |

The project would be "Razboj: OpenGL ES", holding these issues in this order.

## 9. What the user has to decide

* **Whether to file section 8 as it stands**, and whether stage 5 belongs in the project at all yet.
* **Tiles or not** (item 11): this note recommends deciding when the depth buffer is designed, with tiles as the default.
* **Where the library lives** (item 13): Rust, as the core's programs are, or C, as Zephyr's drivers are and as most GL code is.
  A C library is what an existing GL program can link against.
* **Whether to aim at conformance** (item 18) or at running a chosen set of programs; conformance is the larger promise.
