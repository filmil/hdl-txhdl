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
use razboj::dl::decode;
use razboj::model::render;
use razboj::op::Kind;
use std::ffi::CStr;

const FW: usize = 1024;
const FH: usize = 1024;
const LIST: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Event {
    Draw(usize),
    Show(u32),
    Blank,
}

struct Model {
    list: &'static mut [[u32; 16]],
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
        let want = [0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3033, 4, 0x3038];
        let mut config = core::ptr::null_mut();
        let mut n = 0;
        assert_eq!(
            eglChooseConfig(dpy, want.as_ptr(), &mut config, 1, &mut n),
            1
        );
        assert_eq!(n, 1, "one configuration fits");
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
    }
}
