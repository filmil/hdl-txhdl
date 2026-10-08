// SPDX-License-Identifier: Apache-2.0
//! EGL in the model (issue 996): a program's calls through EGL's and
//! GL's C entry points, with a machine whose Razboj is the model, so
//! that every swap is drawn and every buffer read back.
//!
//! The machine keeps Razboj's framebuffer, rows of 1024 words, and the
//! display list. A swap draws the list into the framebuffer as the
//! rasteriser does: its clear covers rows 0 to 479, its own screen, and
//! every other entry its box. The machine records each draw, each base
//! the scanout is given and each blanking waited for.
use gles::fixed::ONE;
use gles_capi::*;
use gles_egl::*;
use razboj::dl::{decode, decode_list};
use razboj::model::{render, render_textured, Textures};
use razboj::op::Kind;
use std::ffi::CStr;

const FW: usize = 1024;
const FH: usize = 1024;
const LIST: usize = 256;
const TEX_BUS: u32 = 0x0100_0000;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Event {
    Draw(usize),
    DrawTiled(usize, usize),
    Show(u32),
    Blank,
}

struct Model {
    list: &'static mut [[u32; 16]],
    scratch: &'static mut [[u32; 16]],
    textures: &'static mut [u32],
    fb: Vec<u32>,
    events: Vec<Event>,
}

impl Machine for Model {
    fn list(&mut self) -> &'static mut [[u32; 16]] {
        // SAFETY: the list lives as long as the test, and EGL hands it
        // to GL and to the draw in turn, never both at once.
        unsafe { &mut *(self.list as *mut [[u32; 16]]) }
    }

    fn draw(&mut self, entries: usize) {
        self.events.push(Event::Draw(entries));
        for w in &self.list[..entries] {
            let op = decode(w);
            let h = if op.kind == Kind::Clear { 480 } else { FH };
            let drawn = render(&[op], FW, h);
            for (p, d) in self.fb.iter_mut().zip(drawn) {
                if d != 0 {
                    *p = d;
                }
            }
        }
    }

    fn scratch(&mut self) -> &'static mut [[u32; 16]] {
        // SAFETY: as the list's.
        unsafe { &mut *(self.scratch as *mut [[u32; 16]]) }
    }

    /// A tile table, drawn as the rasteriser draws one: its entries,
    /// each clipped to its tile, through one depth buffer, which no two
    /// tiles share a pixel of.
    fn draw_tiled(&mut self, tiles: &[[u32; 2]], entries: &[[u32; 16]]) {
        self.events
            .push(Event::DrawTiled(tiles.len(), entries.len()));
        let words = &*self.textures;
        let read = |a: u32| words[((a - TEX_BUS) / 4) as usize];
        let t = Textures { mem: &read };
        let ops = decode_list(entries);
        let drawn = render_textured(&ops, FW, FH, vec![0; FW * FH], Some(&t));
        for (p, d) in self.fb.iter_mut().zip(drawn) {
            if d != 0 {
                *p = d;
            }
        }
    }

    fn textures(&mut self) -> Option<(&'static mut [u32], u32)> {
        // SAFETY: as the list's; the draw only reads it.
        let room = unsafe { &mut *(self.textures as *mut [u32]) };
        Some((room, TEX_BUS))
    }

    fn show(&mut self, row: u32) {
        self.events.push(Event::Show(row));
    }

    fn wait_blanking(&mut self) {
        self.events.push(Event::Blank);
    }
}

fn model() -> &'static mut Model {
    Box::leak(Box::new(Model {
        list: Box::leak(vec![[0u32; 16]; LIST].into_boxed_slice()),
        scratch: Box::leak(vec![[0u32; 16]; 4096].into_boxed_slice()),
        textures: Box::leak(vec![0u32; 1 << 14].into_boxed_slice()),
        fb: vec![0; FW * FH],
        events: Vec::new(),
    }))
}

/// The pixels of the rows `rows`, the 640 columns the window has.
fn rows(fb: &[u32], rows: std::ops::Range<usize>) -> Vec<u32> {
    rows.flat_map(|y| fb[y * FW..y * FW + 640].iter().copied())
        .collect()
}

/// A frame: cleared in `clear`, and a triangle of the current colour
/// in the middle of the window.
unsafe fn frame(clear: [i32; 3]) {
    static TRI: [i32; 6] = [0, 0, 1 << 15, 0, 0, 1 << 15];
    // A tall one in grey whose apex is three window heights above the
    // window's top edge, to be clipped there and not drawn on the
    // buffer above.
    static TALL: [i32; 6] = [-ONE / 2, ONE / 2, ONE / 2, ONE / 2, 0, 3 * ONE];
    glClearColorx(clear[0], clear[1], clear[2], ONE);
    glClear(0x4000);
    glEnableClientState(0x8074);
    glColor4x(ONE / 2, ONE / 2, ONE / 2, ONE);
    glVertexPointer(2, 0x140C, 0, TALL.as_ptr() as *const _);
    glDrawArrays(0x0004, 0, 3);
    glColor4x(0, ONE, 0, ONE);
    glVertexPointer(2, 0x140C, 0, TRI.as_ptr() as *const _);
    glDrawArrays(0x0004, 0, 3);
}

#[test]
fn a_program_draws_and_swaps_without_tearing() {
    let m = model();
    let m_ptr = m as *mut Model;
    install(m);
    unsafe {
        let dpy = eglGetDisplay(core::ptr::null_mut());
        let (mut major, mut minor) = (0, 0);
        assert_eq!(eglInitialize(dpy, &mut major, &mut minor), 1);
        assert_eq!((major, minor), (1, 4));
        // Eight bits a channel, a window, and a depth buffer of 16 bits,
        // which a program that tests depth asks for (#1502).
        let want = [
            0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3033, 4, 0x3025, 16, 0x3038,
        ];
        let mut config = core::ptr::null_mut();
        let mut n = 0;
        assert_eq!(
            eglChooseConfig(dpy, want.as_ptr(), &mut config, 1, &mut n),
            1
        );
        assert_eq!(n, 1, "one configuration fits");
        let mut depth = 0;
        assert_eq!(eglGetConfigAttrib(dpy, config, 0x3025, &mut depth), 1);
        assert_eq!(depth, 16, "the depth's bits");
        let surface = eglCreateWindowSurface(
            dpy,
            config,
            core::ptr::null_mut(),
            core::ptr::null(),
        );
        assert!(!surface.is_null());
        let es1 = [0x3098, 1, 0x3038];
        let ctx =
            eglCreateContext(dpy, config, core::ptr::null_mut(), es1.as_ptr());
        assert!(!ctx.is_null());
        assert_eq!(eglMakeCurrent(dpy, surface, surface, ctx), 1);
        let (mut w, mut h) = (0, 0);
        eglQuerySurface(dpy, surface, 0x3057, &mut w);
        eglQuerySurface(dpy, surface, 0x3056, &mut h);
        assert_eq!((w, h), (640, 480));

        // GL's state is set once, before either frame, and holds across
        // the swaps that move the window between the buffers.
        glMatrixMode(0x1701);
        glLoadIdentity();
        glOrthox(-ONE, ONE, -ONE, ONE, -ONE, ONE);
        glMatrixMode(0x1700);

        frame([ONE, 0, 0]);
        assert_eq!(eglSwapBuffers(dpy, surface), 1);
        let m = &mut *m_ptr;
        assert_eq!(
            m.events,
            [Event::Draw(3), Event::Show(512), Event::Blank],
            "the frame drawn, then shown from the next blanking"
        );
        let shown = rows(&m.fb, 512..992);
        let untouched = rows(&m.fb, 0..480);
        assert!(
            untouched.iter().all(|&p| p == 0),
            "nothing above the window"
        );
        assert!(shown.contains(&0xffff_0000), "the red clear");
        assert!(shown.contains(&0xff00_ff00), "the green triangle");

        frame([0, 0, ONE]);
        assert_eq!(eglSwapBuffers(dpy, surface), 1);
        assert_eq!(
            m.events[3..],
            [Event::Draw(3), Event::Show(0), Event::Blank],
            "the other buffer, the next time"
        );
        assert_eq!(
            rows(&m.fb, 512..992),
            shown,
            "the buffer shown was not drawn into while it was shown"
        );
        let second = rows(&m.fb, 0..480);
        assert!(second.contains(&0xff00_00ff), "the blue clear");
        // The same triangle at the same place in each buffer: the
        // projection set before the first frame held.
        let green = |b: &[u32]| {
            b.iter()
                .enumerate()
                .filter(|(_, &p)| p == 0xff00_ff00)
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        };
        assert_eq!(green(&second), green(&shown), "the triangle in both");
        assert_eq!(eglGetError(), 0x3000);

        let version = CStr::from_ptr(eglQueryString(dpy, 0x3054) as *const _);
        assert_eq!(version.to_str().unwrap(), "1.4 TxHDL Razboj");

        // A frame that tests depth (#1273): a green triangle near the
        // eye, then a larger red one behind it. The swap draws it as a
        // tile table, and where they overlap the green stays.
        static NEAR: [i32; 9] =
            [0, 0, ONE / 2, 1 << 15, 0, ONE / 2, 0, 1 << 15, ONE / 2];
        static FAR: [i32; 9] = [
            -ONE / 2,
            -ONE / 2,
            -ONE / 2,
            ONE,
            -ONE / 2,
            -ONE / 2,
            -ONE / 2,
            ONE,
            -ONE / 2,
        ];
        glClearColorx(0, 0, 0, ONE);
        glClear(0x4000 | 0x0100);
        glEnable(0x0B71);
        glColor4x(0, ONE, 0, ONE);
        glVertexPointer(3, 0x140C, 0, NEAR.as_ptr() as *const _);
        glDrawArrays(0x0004, 0, 3);
        glColor4x(ONE, 0, 0, ONE);
        glVertexPointer(3, 0x140C, 0, FAR.as_ptr() as *const _);
        glDrawArrays(0x0004, 0, 3);
        assert_eq!(eglSwapBuffers(dpy, surface), 1);
        let Event::DrawTiled(tiles, entries) = m.events[6] else {
            panic!("a frame that tests depth is a tile table: {:?}", m.events);
        };
        assert!(tiles > 0 && entries > 2 * tiles, "{tiles} tiles, {entries}");
        let third = rows(&m.fb, 512..992);
        let count = |c: u32| third.iter().filter(|&&p| p == c).count();
        let (green, red) = (count(0xff00_ff00), count(0xffff_0000));
        assert!(green > 1000 && red > 1000, "{green} green, {red} red");
        // Each triangle's every pixel is in the window's middle: the near
        // one is a quarter window across, the far one half a window.
        assert_eq!(green, green_alone(), "nothing of the green hidden");
        assert_eq!(eglGetError(), 0x3000);

        // A textured frame (#997): a texture uploaded into the machine's
        // room, two by two of one colour, replacing the triangle's. The
        // swap tells the machine where the descriptors are and draws the
        // frame as a tile table, which reads them.
        static TEXEL: [u8; 16] = [
            0x40, 0x80, 0xc0, 0xff, 0x40, 0x80, 0xc0, 0xff, 0x40, 0x80, 0xc0,
            0xff, 0x40, 0x80, 0xc0, 0xff,
        ];
        static ST: [i32; 6] = [0, 0, ONE, 0, 0, ONE];
        glDisable(0x0B71);
        glClear(0x4000);
        let mut name = 0;
        glGenTextures(1, &mut name);
        glBindTexture(0x0DE1, name);
        glTexParameteri(0x0DE1, 0x2801, 0x2600);
        glTexImage2D(
            0x0DE1,
            0,
            0x1908,
            2,
            2,
            0,
            0x1908,
            0x1401,
            TEXEL.as_ptr() as *const _,
        );
        glTexEnvi(0x2300, 0x2200, 0x1E01);
        glEnable(0x0DE1);
        glEnableClientState(0x8078);
        glTexCoordPointer(2, 0x140C, 0, ST.as_ptr() as *const _);
        glVertexPointer(3, 0x140C, 0, NEAR.as_ptr() as *const _);
        glDrawArrays(0x0004, 0, 3);
        assert_eq!(glGetError(), 0);
        assert_eq!(eglSwapBuffers(dpy, surface), 1);
        assert!(
            matches!(m.events[9], Event::DrawTiled(..)),
            "a textured frame is a tile table: {:?}",
            m.events
        );
        let fourth = rows(&m.fb, 0..480);
        let texel = fourth.iter().filter(|&&p| p == 0xff40_80c0).count();
        assert_eq!(texel, green_alone(), "the texture over the triangle");
        assert_eq!(eglGetError(), 0x3000);
    }
}

/// How many pixels the near triangle covers drawn alone, through the
/// library and the model: the green a frame with depth has to keep.
fn green_alone() -> usize {
    use gles::{gl, Gl};
    let mut frame = vec![[0u32; 16]; 8];
    let mut g = Gl::new(&mut frame, 640, 480);
    g.matrix_mode(gl::PROJECTION);
    g.ortho(-ONE, ONE, -ONE, ONE, -ONE, ONE);
    g.matrix_mode(gl::MODELVIEW);
    g.color(0, ONE, 0, ONE);
    let p = [
        [0, 0, ONE / 2, ONE],
        [1 << 15, 0, ONE / 2, ONE],
        [0, 1 << 15, ONE / 2, ONE],
    ];
    g.draw_arrays(gl::TRIANGLES, &p, None, None);
    let n = g.frame().len();
    let drawn = render(&decode_list(&frame[..n]), 640, 480);
    drawn.iter().filter(|&&p| p == 0xff00_ff00).count()
}
