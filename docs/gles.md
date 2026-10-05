<!-- SPDX-License-Identifier: Apache-2.0 -->
# GL ES 1.1 Common-Lite on Vreteno: the design, before the language

Status: design note, October 4, 2026, for issue 995 (gl-order/14), item 13 of `opengl-gap.md`; section 10 records the steps done.
Read against `main` at `c3103332`, and against the open pull requests it names.
Author: automated coding assistant, with human supervision.

Issue 995 asks for a `GLES_CM` library on Vreteno: the API, the matrix stacks, transform, lighting, clipping, the viewport, and building Razboj's display lists.
Whether that library is written in Rust or in C is the user's decision and is not made here.
This note designs what does not depend on it: what the library implements, the order of the work a vertex goes through, the fixed-point format at every step, what the library hands Razboj, and how the icosahedron becomes its first program.
Section 9 sets out what each language choice costs, for the user to decide.
No code is written until then.

## 1. What is being built, and what is not

OpenGL ES 1.1 has two profiles.
Common takes floating point; Common-Lite takes only fixed point, `GLfixed`, a signed 32-bit number with 16 bits of fraction (16.16).
Common-Lite's entry points are the ones whose arguments are fixed point, the `x` forms such as `glFrustumx`, `glRotatex` and `glLightxv`, with the integer forms such as `glColor4ub`.
Vreteno has no floating point, so Common-Lite is the profile, as `opengl-gap.md` section 4 says.

The library draws through Razboj and through nothing else.
Everything before the pixel is the library's: vertices, matrices, lighting, clipping, the viewport and culling, all on Vreteno.
Everything from the pixel on is Razboj's: coverage, the fill rule, interpolating colour across a triangle, and later depth and blending.

It is not conformant ES 1.1, and does not claim to be.
ES 1.1 requires texturing, which waits for issue 997, and the library leaves out fog, stencil, logic operations and point sprites, which wait for issue 998.
The conformance run is issue 999.
Until then the library is "Common-Lite without textures", as issue 995's title says, and `glGetString(GL_VERSION)` should say so rather than claim the profile.

## 2. The API subset

The table groups the entry points by what they do and says when each can work.
"Now" means on Razboj as it is once pull requests 1064 (sub-pixel vertices, #988) and 1066 (Gouraud shading, #989) land.
A later issue means the entry point is accepted from the start but does what that issue lets it do only once the issue lands.

| Group | Entry points | When |
|---|---|---|
| Viewport | `glViewport`, `glDepthRangex` | Now; depth range matters from #992 |
| Matrices | `glMatrixMode`, `glLoadIdentity`, `glLoadMatrixx`, `glMultMatrixx`, `glPushMatrix`, `glPopMatrix`, `glTranslatex`, `glRotatex`, `glScalex`, `glFrustumx`, `glOrthox` | Now |
| Vertex arrays | `glVertexPointer`, `glColorPointer`, `glNormalPointer`, `glEnableClientState`, `glDisableClientState`, `glDrawArrays`, `glDrawElements` | Now, for triangles, strips and fans |
| Buffer objects | `glGenBuffers`, `glBindBuffer`, `glBufferData`, `glBufferSubData`, `glDeleteBuffers` | Now; they are core in 1.1, and here they are memory the library owns |
| Current values | `glColor4x`, `glColor4ub`, `glNormal3x` | Now |
| Shading and faces | `glShadeModel`, `glFrontFace`, `glCullFace` | Now |
| Lighting | `glLightx`, `glLightxv`, `glLightModelx`, `glLightModelxv`, `glMaterialx`, `glMaterialxv` | Now |
| Clip planes | `glClipPlanex` | Now, one plane at least |
| Clearing | `glClearColorx`, `glClear`; `glClearDepthx` | Colour now; depth from #992 |
| Scissor | `glScissor` | #990 |
| Depth | `glDepthFunc`, `glDepthMask` | #992 |
| Blending and masks | `glBlendFunc`, `glAlphaFuncx`, `glColorMask` | #993 |
| Points and lines | `glPointSizex`, `glLineWidthx`, and the point and line modes of the draw calls | #994 |
| Switches | `glEnable`, `glDisable`, `glIsEnabled` for `GL_LIGHTING`, `GL_LIGHT0` to `GL_LIGHT7`, `GL_CULL_FACE`, `GL_NORMALIZE`, `GL_RESCALE_NORMAL`, `GL_COLOR_MATERIAL`, `GL_CLIP_PLANE0`; and the rest as the issues above land | Now, and growing |
| Queries and errors | `glGetError`, `glGetIntegerv`, `glGetFixedv`, `glGetBooleanv`, `glGetString`, `glGetPointerv` | Now |
| Completion | `glFlush`, `glFinish` | Now (section 7) |
| Hints | `glHint` | Accepted and ignored, as the specification allows |

Left out until their issues: every texture entry point (#997), fog (#998), stencil (#998), `glLogicOp` (#998), `glPolygonOffsetx` (#998), point sprites (#998), and `glReadPixels`, which needs a read path from the framebuffer that nothing has asked for yet.
An entry point that is left out still exists, so that a program links, and sets `GL_INVALID_ENUM` or `GL_INVALID_OPERATION` as the specification says for an unsupported value.

The limits the library reports are the specification's minimums: a modelview stack of 16, projection and texture stacks of 2, eight lights, one clip plane.
`GL_SUBPIXEL_BITS` is 4, which is what pull request 1064 gives Razboj and what the specification asks for at least.

## 3. The pipeline, in order

A draw call takes each vertex and each primitive through these steps.
Section 4 gives the format at each one.

1. **Fetch.**
   A vertex's position, colour and normal from the arrays or the current values, as the client state says, converted to 16.16.
   Arrays of `GL_FIXED`, `GL_SHORT` and `GL_BYTE` positions, and `GL_UNSIGNED_BYTE` and `GL_FIXED` colours, are what Common-Lite allows.
2. **Modelview.**
   Position to eye space by the modelview matrix, and the normal by its inverse transpose.
   The inverse transpose is worked out when the modelview changes, not per vertex.
   With `GL_RESCALE_NORMAL` or `GL_NORMALIZE`, the normal is scaled or made unit length.
3. **Lighting.**
   With `GL_LIGHTING` on, the vertex's colour is the specification's sum over the enabled lights, in eye space.
   With it off, the colour is the vertex's own.
   Section 5 says what this costs and where it is cheap.
4. **Projection.**
   Eye space to clip space by the projection matrix.
5. **Clipping.**
   A triangle is clipped against the near plane, the far plane and any enabled user plane, and against the guard band in section 6, and only when it crosses one of them.
   A triangle wholly outside any one plane is dropped.
6. **Divide and viewport.**
   Clip space to window space: the perspective divide, then the viewport and the depth range.
   The window's y grows upwards in GL and downwards in Razboj, so y is turned over here, once (section 6).
7. **Culling.**
   The signed area in window space says which way the triangle faces, against `glFrontFace`, and `glCullFace` drops the faces it names.
8. **Primitive assembly.**
   Triangles, strips and fans become triangles.
   Points and lines wait for issue 994, and either become Razboj's own points and lines or, if that issue chooses, thin triangles from here.
9. **Shading.**
   Flat shading takes the colour of the provoking vertex, the last of each triangle in ES 1.1, and writes a flat triangle.
   Smooth shading writes a Gouraud triangle with a colour at each vertex, and the library works out the three colour planes Razboj steps (section 4).
10. **The frame.**
    Each surviving triangle goes into the frame's list, and the frame is handed to Razboj on `glFlush`, `glFinish` or, later, `eglSwapBuffers` (section 7).

## 4. The fixed-point formats

Every number has one format at each step, and every narrowing says how it rounds and what it does with a value out of range.

| Step | Quantity | Format | Notes |
|---|---|---|---|
| API | Every `x` argument | 16.16, signed 32 bits | `GLfixed` |
| Matrices | Each of the 16 elements | 16.16 | A product of two is 64 bits (`mul` and `mulh` together), shifted back by 16 and saturated |
| Eye space | Position, normal, light vectors | 16.16 | A dot product sums its three or four 64-bit products before the shift, so it rounds once |
| Colour, inside | Each channel | 16.16, nominally 0 to 1 | The lighting sum is clamped to 0 to 1 at the end, as the specification says |
| Clip space | x, y, z, w | 16.16 | |
| Divide | 1/w | 2.30 of the reciprocal, then multiplied | One division per vertex rather than three (section 5) |
| Window space | x, y | Sixteenths of a pixel, 16 bits signed | `Op::TriQ4`'s and `Op::Gouraud`'s vertices (`gpu/razboj/src/op.rs` on pull request 1064); rounded to nearest |
| Window space | z | 16 bits, 0 to 65535 | For #992's depth plane; unused before it |
| Colour, out | Each channel | 8 bits | `round(c * 255)`, so 1.0 is 255 and 0.5 is 128 |
| Planes | Value and two steps per channel | 16 bits of fraction in 32 | Exactly what `op.rs`'s `plane` writes on pull request 1066 |

**Overflow.**
16.16 holds about plus or minus 32768.
A scene in units of a metre with a camera a few kilometres off overflows it, and a GL ES 1.1 Common-Lite program is expected to keep its numbers in range; the specification leaves overflow undefined.
The library saturates rather than wraps wherever it narrows, so an overflow distorts a picture rather than throwing a vertex to the other side of the screen.

**The planes in 64 bits.**
Pull request 1066's encoder works the planes out in 128-bit integers on the host.
On Vreteno that would be a library routine per operation, so the library does it in 64 bits, and the arithmetic shows 64 is enough.
With vertices in plus or minus 2^14 sixteenths, a difference of two is under 2^15, and a channel's difference is under 2^8.
So the gradient's numerator is under 2^24, its product with an offset inside the box under 2^39, the sum of two such under 2^40, and that times 2^16 for the fraction under 2^56.
Twice the area is under 2^31.
Every intermediate fits in a signed 64-bit integer, and the quotients are the same numbers the 128-bit encoder gets.
The library's planes are therefore checked bit for bit against `op.rs`'s, in the library's tests.

## 5. Lighting, and what it costs

The specification's colour at a vertex is the material's emission, plus the scene's ambient times the material's, plus for each enabled light an attenuated and spot-limited sum of three terms: ambient, diffuse (the normal against the direction to the light) and specular (the normal against the half vector, raised to the shininess).

Most of that is multiplications and additions in 16.16, which Vreteno does in three cycles a multiply.
Three parts are expensive, and each has a cheap case the common program uses.

* **A positional light** needs the distance to it for attenuation, a square root and a division per vertex.
  A directional light, `w = 0` in `GL_POSITION`, needs neither, and is what the icosahedron uses.
* **Specular** raises a number between 0 and 1 to a shininess between 0 and 128.
  A table of 2^x and one of log2(x), each a few hundred bytes, make it two lookups and a multiply.
  With a shininess of 0 or a black specular colour it is skipped.
* **Normalising**, with `GL_NORMALIZE`, needs a reciprocal square root.
  One Newton step from a 256-entry table is accurate to 16 bits.
  `GL_RESCALE_NORMAL`, which a program with only rotations and uniform scales can use instead, is one multiply per component.

Vreteno's division is 34 cycles (`//docs:vreteno`, section "The Core"), so the divide in step 6 is done once per vertex as a reciprocal of w and then two multiplications.
A 64-bit division, which the planes need three of per smooth triangle, is a library routine on RV32; the library divides once, for the reciprocal of the triangle's area, and multiplies by it for each channel, if its tests show that rounds as `op.rs` does.
If it does not, it divides three times, and section 10 measures what that costs.

## 6. Clipping, the guard band and the window

**The guard band.**
Razboj accepts vertices anywhere in 1024 pixels either side of the origin (`op.rs`'s `VMIN` and `VMAX`), and it clips the box it walks to the screen, and from #990 to the scissor box.
So the library does not clip a triangle against the screen's edges.
A triangle that crosses the left edge of a 640 by 480 window is drawn as it is, and Razboj walks only its part on the screen.
The library clips against the screen's edges only when a vertex would land outside Razboj's range: the guard band is that range, about 1024 pixels left and above and 384 right and 544 below for a 640 by 480 window at the origin.
Triangles that big are rare, and the common triangle is never clipped in x or y at all.

**Near, far and user planes.**
The near plane is always clipped against, because a vertex behind the eye has a w of zero or less and its divide means nothing.
The far plane and the enabled user planes are clipped against too.
A clipped triangle is a polygon of up to ten vertices, three and one more for each of the seven planes, clipped plane by plane with the colours and the depths interpolated linearly in clip space, as the specification says, and then cut into a fan.

**The window.**
GL's window has its origin at the bottom left with y up, and Razboj's has it at the top left with y down, so the library writes `y_razboj = height - y_window`.
Two things follow from that one line.
The winding turns over, so the culling test of step 7 compares the signed area with the opposite sign from GL's; the library decides front and back in GL's window space, before the turn, so that `glFrontFace` means what a GL program expects.
And Razboj's top-left fill rule, from pull request 1064, becomes a bottom-left rule in GL's terms.
GL does not specify which fill rule, only that a pixel on an edge two triangles share is drawn by exactly one of them, and both rules do that.

**Pixel centres.**
GL samples a pixel at its centre, `x + 0.5`, and so does Razboj since pull request 1064, so the viewport transform needs no half-pixel correction.

## 7. What the library hands Razboj

**The instruction.**
Each triangle becomes one of Razboj's sixteen-word instructions (`gpu/razboj/src/dl.rs` on pull request 1066): `Op::TriQ4` for a flat triangle and `Op::Gouraud` for a smooth one.
The library does what `Op::encode` does, the winding, the box and the planes, and writes the words itself, so that the format stays stated once, in `dl.rs`, and the library's tests check its words against `dl::encode`'s.

**The frame.**
A GL program issues draw calls one after another, and Razboj with tiles (issue 991, `docs/razboj-tiles.md` on its branch) draws a frame one tile at a time.
So the library defers: a draw call transforms, lights, clips and shades its triangles and appends them to the frame's list in window space, and nothing goes to Razboj until the frame is ended.
That is how every tiled renderer works, and it costs memory in DDR3 for the frame, about 64 bytes a triangle before binning.

**Ending a frame.**
`glFlush` ends the frame and hands it over: before tiles, the list itself with its count written last, as `dl.rs` requires; with tiles, the frame binned into the tile table of `razboj-tiles.md` section 4, through the binning library that note proposes.
`glFinish` does the same and then waits for Razboj's done signal, which is issue 982.
`eglSwapBuffers`, issue 996, ends the frame, waits, and flips the scanout's base at the vertical sync.
`glClear` at the start of a frame becomes the tile buffer's clear values rather than an instruction (`razboj-tiles.md` section 3), and before tiles it is a clear instruction at the head of the list.

**Binning.**
The tiles note asks for one binning library, in Rust, usable both from a program on Vreteno and from Razboj's model, so that the board and the tests run the same code.
It clips each triangle's box to each 64 by 64 tile it touches and steps the planes to each tile's first pixel rather than working them out again, so that the tiled picture is the untiled one bit for bit.
The GL library calls it on the frame's triangles and does not do its own.
That is the one place where a choice already made reaches into this one (section 9).

## 8. The first program: the icosahedron

Issue 995 is done when the icosahedron program is written against `glDrawElements`, `glFrustumx` and `glLightxv` and draws the same picture.
The program it replaces is the one issue 986 makes: `cpu/vreteno/rust/ico_hdmi.rs` writing Razboj display lists by hand on the board, double buffered.
Today's `ico_hdmi.rs` turns the solid itself, decides which faces show by their turned normals, lights each face from the eye, and draws every pixel on the core.

In GL the same picture is:

* **The solid.**
  Faceted, so every face has its own normal and each of its three corners carries it.
  That is 60 vertices, three per face, not 12, and `glDrawElements` with indices 0 to 59, `GL_UNSIGNED_BYTE`.
  Twelve shared vertices with averaged normals would draw a smooth ball, which is a different picture.
* **The view.**
  `glFrustumx` with the focal length the program uses now, and `glTranslatex` and `glRotatex` on the modelview per frame for the turn.
* **The light.**
  One directional light at the eye: `GL_POSITION` of `(0, 0, 1, 0)` in eye space, set with the modelview at identity.
  The face's colour between the program's dark and light colours becomes the material's ambient and diffuse.
* **Hidden faces.**
  `glEnable(GL_CULL_FACE)`, since the solid is convex, as the program's own comment says, and no depth test is needed.
* **Flat shading.**
  `glShadeModel(GL_FLAT)`, so each face is one colour, as now.

"The same picture" needs saying precisely, because the GL path rounds in different places from the hand-written one.
The check is in the model: both programs' lists drawn by Razboj's model on the same frame of the turn, the same faces drawn, each face's colour within 2 of the other's in each channel, and no pixel more than one pixel from the other picture's edge.
On the board, the two are shown one after the other, as issue 986's own board check will show its program.

## 9. The language, for the user to decide

Three choices, each with what it costs.

**C.**
What existing GL ES programs are written in and link against, and the headers they include, `GLES/gl.h`, are Khronos's and are C.
Zephyr's drivers here are C, and EGL (issue 996) will be one.
Its cost is the display list's format: `dl.rs` states it in Rust, once, and a C library would need it stated again, or generated from the Rust.
The tiles note's binning library would be called across the boundary, or written twice.
The tests that compare against Razboj's model, which is Rust, would call the C library through a foreign function interface.

**Rust.**
What Vreteno's programs are written in (`cpu/vreteno/rust`), what Razboj's model and its encoder are, and what the binning library will be.
The library would use `dl.rs` and the binning library directly, and its tests would run against the model with no boundary.
Its cost is the API: the entry points a C program calls are `extern "C"` functions with C's names and types, which Rust can export, but then the library is Rust that pretends to be C at its edge, and every existing GL program still needs the C header.

**Rust inside, C at the edge.**
The library is Rust, as in the second choice, and exports the GL entry points as `extern "C"` functions, with `GLES/gl.h` from Khronos as the header.
A C program links against it as against any GL library, and a Rust program calls the same functions or a thin Rust layer over them.
It keeps the format and the binning in one language.
Its cost is the boundary itself: no panics may cross it, which `ico.rs`'s rules already forbid anyway, and the build needs a static library a C link can take.

The note recommends the third, for the reasons given, but the decision is the user's, and nothing in sections 1 to 8 changes with it.

## 10. The order of the work, once the language is chosen

Each step is a pull request, checked before the next.

1. **The fixed-point core and its tests.**
   16.16 arithmetic, the matrices and their stacks, the transform and the viewport, tested on the host against a reference written with wide integers.
   No Razboj yet.
2. **Triangles to Razboj's words.**
   Clipping, culling, flat and smooth shading, the planes in 64 bits, and the instruction words, tested bit for bit against `op.rs` and `dl.rs`, and drawn by the model.
   Steps 1 and 2 are done together in issue 1159, as the crate `//gles`, with the frame's binning of step 4 through `razboj_tile` (issue 1157).
   It departs from sections 4 to 6 in three places.
   The divide is one 64-bit division a window coordinate, straight to sixteenths and rounded to the nearest, because a reciprocal of w in 2.30 cannot hold 1/w for a w under a quarter.
   The guard band stands a pixel inside Razboj's range, so that a vertex clipped onto it and rounded stays in range.
   A vertex keeps its own colour or the current one until step 3 lights it.
   The pictures are checked pixel for pixel against a second pipeline written with 128-bit integers to the same rules, through Razboj's model, binned and not; the words bit for bit against `op.rs` and `dl.rs`; and the window and the matrices against floating point.
3. **Lighting.**
   The specification's sum, with the tables, tested against a reference.
   Done in issue 1164, in `gles/src/light.rs`: eight lights, directional and positional, with attenuation and spots; materials; the scene's ambient and two-sided lighting; `GL_NORMALIZE`, `GL_RESCALE_NORMAL` and `GL_COLOR_MATERIAL`; normals carried by the modelview's inverse transpose, worked out once a draw.
   It departs from section 5 in one place: the powers come from a fixed-point log2 and exp2, the logarithm by squaring and the exponential from sixteen roots of two worked out at compile time, rather than from tables of a few hundred bytes, and they agree with floating point to within two thousandths over the shininess's range.
   The colour at a vertex is within one step of its byte of a floating-point reference of the same sum, over three thousand random cases and through the pipeline into Razboj's model.
4. **The frame.**
   Deferral, `glFlush` and `glFinish`, the list or, once issue 991's step 1 lands, the binning library.
5. **The icosahedron.**
   The program of section 8, checked in the model against issue 986's, and then on the board.
   The cycles a frame takes on Vreteno are measured here, and compared with issue 986's hand-written list.
   Done in `cpu/vreteno/rust/ico_gl.rs`, which writes the frames `ico_list.rs` writes by hand through `glFrustumx`, `glRotatex`, `glLightxv`, `glMaterialxv` and `glDrawElements`, and `ico_gl_hdmi`, the board's program built with it.
   It departs from section 8 in two places.
   Issue 986's program lights a face as the square root of `0.4 + 0.6 l`, where `l` is how directly the face looks at the eye, and GL's sum has no square root.
   One directional light at the eye has its half vector at the eye too, so the specular term is `l` raised to the shininess, and the scene's ambient at `sqrt(0.4)` of the colour, no diffuse, and the rest of the colour as specular at a shininess of 0.8715 stay within 0.004 of the square root for every `l`.
   And the backdrop's rectangle at the head of each list is issue 986's own rather than `glClear`, since a clear is the whole screen and the second frame shares the framebuffer with the first.
   `//cpu/vreteno/rust:ico_gl_test` draws both through Razboj's model at 64 angles in each frame: the same faces, every face's colour within 2 of the other's in each channel, the worst being 2, and every pixel where the two differ beyond that on an edge of both.
   The cycles on the board wait for a board session.
6. **The C ABI.**
   The user chose Rust inside with C at the edge (section 9), and this is the edge, issue 1224, which EGL (issue 996) needs.
   `gles/capi/lib.rs` has `extern "C"` functions with the names and types of Khronos's `GLES/gl.h` for the 41 entry points the library implements, over a current context and its client arrays.
   The client arrays are read a vertex at a time, in the types Common-Lite allows, through `Gl::draw_vertices`, so nothing is copied or allocated.
   `GLES/gl.h`, `GLES/glplatform.h` and `KHR/khrplatform.h` are not copied into the tree: `MODULE.bazel` fetches them from Khronos's OpenGL-Registry and EGL-Registry at pinned commits, by their sha256, and `//third_party/khronos:gles1` lays them out for `#include <GLES/gl.h>`.
   The other 104 entry points gl.h declares, the floating-point ones among them, are C that `gles/capi/stubs.sh` writes from gl.h itself, leaving out every name `lib.rs` defines; each sets `GL_INVALID_OPERATION`, so every GL ES 1.1 program links and is told what it asked for is not there.
   `glGetString(GL_VERSION)` says "OpenGL ES-CL 1.1 TxHDL, Common-Lite without textures, not conformant", which answers the last question of section 11 for now.
   It departs from the plan in one place: making a context current is EGL's, so until issue 996 lands `gles_make_current` does it over a frame its caller owns, and `glFlush` and `glFinish` leave the frame for EGL to hand to Razboj.
   `//gles:capi_test` draws one scene in C through `:gles_c` and through the Rust API, and holds the two frames to each other word for word, with what an unimplemented entry point and `glGetString` say.
7. **EGL, in the model.**
   Issue 996: `gles/egl/lib.rs` has Khronos's `EGL/egl.h` calls a GL ES 1.1 program makes, as `extern "C"` functions over the C entry points, Rust inside as the library is.
   There is one display, one configuration and one window of 640 by 480, double buffered in Razboj's framebuffer at rows 0 and 512, as issue 986's icosahedron is.
   GL draws into the buffer not shown: `Gl::retarget` moves the context there with its state kept, clips every triangle to that buffer's rows, and clears it as a rectangle, since Razboj's own clear is of rows 0 to 479.
   `eglSwapBuffers` has Razboj draw the frame and waits until it is written, points the scanout at that buffer, and waits for the vertical blanking, so a frame is never shown half drawn.
   What a swap does to the hardware is behind the `Machine` trait.
   `//gles:egl_test` gives one that draws through Razboj's model.
   A program sets up through EGL and draws two frames.
   Each lands in the buffer not shown, nothing lands on the buffer being shown, a triangle reaching above the window is clipped at its top, and GL's state holds across the swaps.
   The calls egl.h declares and this does not implement are written from egl.h by the same `stubs.sh`, and fail with `EGL_BAD_MATCH`.
   The EGL headers are fetched as the GL ones are, with `EGL_NO_PLATFORM_SPECIFIC_TYPES`, since neither Zephyr nor the host has a window system.
   The board's `Machine`, and a Zephyr program drawing through it, are the issue's next step.

Points, lines, depth, blending and the scissor are added as issues 990, 992, 993 and 994 land, each with the entry points section 2 holds for it.

## 11. What the user has to decide

* **The language** (section 9): C, Rust, or Rust inside with C at the edge, which this note recommends.
* **Whether "the same picture" in section 8 is the right check**, or whether the GL icosahedron should match issue 986's pixel for pixel, which would mean rounding as the hand-written program does rather than as GL does.
* **What `glGetString(GL_VERSION)` says** while the library is not conformant (section 1).
