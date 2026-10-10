/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The rendering (#999): scenes drawn through GL ES 1.1 and read back with
 * glReadPixels, each pixel held to a reference that follows the
 * specification's rules in double precision: the rasterisation of
 * points (section 3.3.1) and polygons (3.5.1), the viewport (2.10.1),
 * smooth shading (3.5.1), the per-fragment tests and operations in
 * chapter 4, fog (3.8), the texture environments (3.7.12) and lighting
 * (2.12.1). Where the specification leaves a choice to the
 * implementation, a pixel whose centre lies exactly on a polygon's edge,
 * the pixel is not checked. A colour is held to within one unit of the
 * reference in each channel unless the test says otherwise, since GL
 * leaves the last bit of a conversion to the implementation.
 *
 * Every scene is drawn in window coordinates, GL's rows from the bottom,
 * given exactly: the matrices are the identity and the viewport 1024 by
 * 512 from the window's corner, so that a window x of k sixteenths is the
 * device x k / 8192 - 1 and a window y k / 4096 - 1, both exact in 16.16.
 * An orthographic projection of the window would scale by 2/640, which
 * 16.16 does not hold, and move a vertex at the window's far side by more
 * than half a pixel, within what GL ES allows a fixed-point transform.
 */
#include <math.h>
#include <stdio.h>
#include <string.h>

#include "conform.h"

#define ONE CONFORM_ONE
#define W CONFORM_WIDTH
#define H CONFORM_HEIGHT

static GLubyte fb[W * H * 4];

/* A pixel's channel, GL's rows from the bottom. */
static int at(int x, int y, int c) { return fb[(y * W + x) * 4 + c]; }

/* The window read back. */
static void read_window(void) {
  glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, fb);
}

/* A window x and y, in 16.16, as the device coordinates the scene's
 * viewport maps to them. */
static GLfixed dx(GLfixed x) { return x / 512 - ONE; }
static GLfixed dy(GLfixed y) { return y / 256 - ONE; }

/* A fresh scene: GL's state as the suite leaves it between tests, the
 * window cleared to `clear` and the depth to the farthest. */
static void scene(GLuint clear) {
  glDisable(GL_BLEND);
  glDisable(GL_DEPTH_TEST);
  glDisable(GL_ALPHA_TEST);
  glDisable(GL_STENCIL_TEST);
  glDisable(GL_SCISSOR_TEST);
  glDisable(GL_COLOR_LOGIC_OP);
  glDisable(GL_FOG);
  glDisable(GL_LIGHTING);
  glDisable(GL_TEXTURE_2D);
  glDisable(GL_CULL_FACE);
  glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
  glDepthMask(GL_TRUE);
  glShadeModel(GL_SMOOTH);
  glViewport(0, 0, 1024, 512);
  glMatrixMode(GL_PROJECTION);
  glLoadIdentity();
  glMatrixMode(GL_MODELVIEW);
  glLoadIdentity();
  GLfixed c[4];
  for (int k = 0; k < 4; k++)
    c[k] = (GLfixed)(((clear >> (24 - 8 * k)) & 0xff) * ONE / 255);
  glClearColorx(c[0], c[1], c[2], c[3]);
  glClearDepthx(ONE);
  glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
  glDisableClientState(GL_COLOR_ARRAY);
  glDisableClientState(GL_NORMAL_ARRAY);
  glDisableClientState(GL_TEXTURE_COORD_ARRAY);
  glEnableClientState(GL_VERTEX_ARRAY);
}

/* The colour `0xRRGGBBAA` as the current colour. */
static void colour(GLuint rgba) {
  GLfixed c[4];
  for (int k = 0; k < 4; k++)
    c[k] = (GLfixed)(((rgba >> (24 - 8 * k)) & 0xff) * ONE / 255);
  glColor4x(c[0], c[1], c[2], c[3]);
}

/* A rectangle from (x0, y0) to (x1, y1) in window coordinates, at
 * object z `z`, as two triangles. */
static void rect(GLfixed x0, GLfixed y0, GLfixed x1, GLfixed y1, GLfixed z) {
  GLfixed v[12] = {dx(x0), dy(y0), z, dx(x1), dy(y0), z,
                   dx(x1), dy(y1), z, dx(x0), dy(y1), z};
  static const GLubyte idx[6] = {0, 1, 2, 0, 2, 3};
  glVertexPointer(3, GL_FIXED, 0, v);
  glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, idx);
}

/* Whether channel value `got` is within `tol` of `want`, a double from
 * 0 to 255. */
static int near(int got, double want, double tol) {
  return fabs(got - want) <= tol + 1e-6;
}

/* A failure's report: the first pixel that differs. */
static int first_x, first_y, first_got, first_want;

/* Whether every pixel whose `want` is not negative holds it in every
 * channel within `tol`; `want` gives the four channels, or -1 to skip
 * the pixel. */
static int window_is(void (*want)(int x, int y, double out[4]), double tol) {
  for (int y = 0; y < H; y++)
    for (int x = 0; x < W; x++) {
      double w[4];
      want(x, y, w);
      if (w[0] < 0)
        continue;
      for (int c = 0; c < 4; c++)
        if (!near(at(x, y, c), w[c], tol)) {
          first_x = x, first_y = y, first_got = at(x, y, c);
          first_want = (int)lround(w[c]);
          return 0;
        }
    }
  return 1;
}

static void report(const char *name, int ok) {
  if (ok)
    conform_pass("render", name);
  else
    conform_fail("render", name, "at %d,%d: %d, not %d", first_x, first_y,
                 first_got, first_want);
}

/* A colour as four doubles. */
static void unpack(GLuint rgba, double out[4]) {
  for (int k = 0; k < 4; k++)
    out[k] = (rgba >> (24 - 8 * k)) & 0xff;
}

/* --- Clears, the scissor and the viewport. ------------------------- */

static GLuint want_clear;
static void want_flat(int x, int y, double out[4]) {
  (void)x, (void)y;
  unpack(want_clear, out);
}

static void clears(void) {
  want_clear = 0x3366cc99;
  scene(want_clear);
  read_window();
  report("clear", window_is(want_flat, 1));
}

static int box[4];
static GLuint inside, outside;
static void want_box(int x, int y, double out[4]) {
  int in = x >= box[0] && x < box[0] + box[2] && y >= box[1] &&
           y < box[1] + box[3];
  unpack(in ? inside : outside, out);
}

static void scissor(void) {
  outside = 0x0000ffff, inside = 0xff0000ff;
  scene(outside);
  box[0] = 100, box[1] = 50, box[2] = 200, box[3] = 120;
  glScissor(box[0], box[1], box[2], box[3]);
  glEnable(GL_SCISSOR_TEST);
  glClearColorx(ONE, 0, 0, ONE);
  glClear(GL_COLOR_BUFFER_BIT);
  read_window();
  report("scissor_clear", window_is(want_box, 0));
  scene(outside);
  glScissor(box[0], box[1], box[2], box[3]);
  glEnable(GL_SCISSOR_TEST);
  colour(inside);
  rect(0, 0, W * ONE, H * ONE, 0);
  read_window();
  report("scissor_draw", window_is(want_box, 0));
}

static void viewport(void) {
  outside = 0x000000ff, inside = 0x00ff00ff;
  scene(outside);
  box[0] = 10, box[1] = 20, box[2] = 100, box[3] = 50;
  glViewport(box[0], box[1], box[2], box[3]);
  colour(inside);
  /* The device's square from -1 to 1, which the viewport maps onto its
   * rectangle. */
  GLfixed v[8] = {-ONE, -ONE, ONE, -ONE, ONE, ONE, -ONE, ONE};
  static const GLubyte idx[6] = {0, 1, 2, 0, 2, 3};
  glVertexPointer(2, GL_FIXED, 0, v);
  glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, idx);
  read_window();
  report("viewport", window_is(want_box, 0));
}

/* --- Triangles: coverage, shared edges, smooth shading. ------------ */

/* A triangle's vertices in sixteenths of a pixel, the precision the
 * library keeps, and its colours. */
static long long tv[3][2];
static double tc[3][4];

/* Twice the signed area of (a, b, p), in sixteenths squared. */
static long long edge(const long long *a, const long long *b, long long px,
                      long long py) {
  return (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0]);
}

/* Whether the pixel's centre is inside the triangle (1), outside (0), or
 * on an edge (-1), which the specification leaves to the
 * implementation. */
static int covers(int x, int y) {
  long long px = 16LL * x + 8, py = 16LL * y + 8;
  long long e0 = edge(tv[0], tv[1], px, py);
  long long e1 = edge(tv[1], tv[2], px, py);
  long long e2 = edge(tv[2], tv[0], px, py);
  if (e0 == 0 || e1 == 0 || e2 == 0)
    return -1;
  return (e0 > 0 && e1 > 0 && e2 > 0) || (e0 < 0 && e1 < 0 && e2 < 0);
}

/* The triangle's colour at the pixel's centre by barycentric weights. */
static void shade(int x, int y, double out[4]) {
  double px = 16.0 * x + 8, py = 16.0 * y + 8;
  long long a = edge(tv[0], tv[1], tv[2][0], tv[2][1]);
  double w0 = edge(tv[1], tv[2], (long long)px, (long long)py) / (double)a;
  double w1 = edge(tv[2], tv[0], (long long)px, (long long)py) / (double)a;
  double w2 = 1 - w0 - w1;
  for (int c = 0; c < 4; c++)
    out[c] = w0 * tc[0][c] + w1 * tc[1][c] + w2 * tc[2][c];
}

static int smooth_alpha;
static void want_triangle(int x, int y, double out[4]) {
  int in = covers(x, y);
  if (in < 0) {
    out[0] = -1;
    return;
  }
  if (!in) {
    unpack(outside, out);
    return;
  }
  shade(x, y, out);
  if (!smooth_alpha)
    out[3] = tc[2][3];
}

/* Draws the triangle `tv`, `tc` through client arrays. */
static void draw_triangle(void) {
  GLfixed v[6];
  GLubyte c[12];
  for (int k = 0; k < 3; k++) {
    v[2 * k] = (GLfixed)(tv[k][0] * 8) - ONE;
    v[2 * k + 1] = (GLfixed)(tv[k][1] * 16) - ONE;
    for (int ch = 0; ch < 4; ch++)
      c[4 * k + ch] = (GLubyte)tc[k][ch];
  }
  glVertexPointer(2, GL_FIXED, 0, v);
  glColorPointer(4, GL_UNSIGNED_BYTE, 0, c);
  glEnableClientState(GL_COLOR_ARRAY);
  glDrawArrays(GL_TRIANGLES, 0, 3);
  glDisableClientState(GL_COLOR_ARRAY);
}

/* Pseudorandom numbers from a literal seed. */
static unsigned seed = 0x2545f491u;
static unsigned next(void) {
  seed ^= seed << 13;
  seed ^= seed >> 17;
  seed ^= seed << 5;
  return seed;
}

static void triangles(void) {
  outside = 0x101010ff;
  char name[64];
  for (int t = 0; t < 8; t++) {
    for (int k = 0; k < 3; k++) {
      tv[k][0] = next() % (W * 16);
      tv[k][1] = next() % (H * 16);
    }
    GLuint flat = next() | 0xff;
    for (int k = 0; k < 3; k++)
      unpack(flat, tc[k]);
    smooth_alpha = 0;
    scene(outside);
    glShadeModel(GL_FLAT);
    draw_triangle();
    read_window();
    snprintf(name, sizeof name, "flat_triangle_%d", t);
    report(name, window_is(want_triangle, 0));
  }
  for (int t = 0; t < 4; t++) {
    for (int k = 0; k < 3; k++) {
      tv[k][0] = next() % (W * 16);
      tv[k][1] = next() % (H * 16);
      unpack(next() | 0xff, tc[k]);
    }
    smooth_alpha = 0;
    scene(outside);
    draw_triangle();
    read_window();
    snprintf(name, sizeof name, "smooth_triangle_%d", t);
    report(name, window_is(want_triangle, 2));
  }
  /* Alpha is shaded smoothly too, as every other channel is. */
  for (int k = 0; k < 3; k++) {
    tv[k][0] = (k == 1 ? 600 : 40) * 16 + 5;
    tv[k][1] = (k == 2 ? 440 : 30) * 16 + 3;
  }
  unpack(0xff000010, tc[0]);
  unpack(0x00ff0080, tc[1]);
  unpack(0x0000fff0, tc[2]);
  smooth_alpha = 1;
  scene(outside);
  draw_triangle();
  read_window();
  report("smooth_alpha", window_is(want_triangle, 2));
}

/* A fan of eight triangles round a point, each sharing its edges with its
 * neighbours, added together: every pixel the fan covers is drawn once
 * (section 3.5.1), so none is brighter than one triangle's colour. */
static void shared_edges(void) {
  scene(0x000000ff);
  glEnable(GL_BLEND);
  glBlendFunc(GL_ONE, GL_ONE);
  colour(0x20202020);
  GLfixed v[2 * 10];
  v[0] = dx(320 * ONE + 4096 * 3), v[1] = dy(240 * ONE + 4096 * 5);
  for (int k = 0; k <= 8; k++) {
    double a = k * 3.14159265358979 / 4;
    v[2 + 2 * k] = dx((GLfixed)((320 + 150 * cos(a)) * 16) * 4096);
    v[3 + 2 * k] = dy((GLfixed)((240 + 150 * sin(a)) * 16) * 4096);
  }
  glVertexPointer(2, GL_FIXED, 0, v);
  glDrawArrays(GL_TRIANGLE_FAN, 0, 10);
  read_window();
  int once = 1, drawn = 0;
  for (int y = 0; y < H && once; y++)
    for (int x = 0; x < W && once; x++) {
      int r = at(x, y, 0);
      drawn += r != 0;
      if (r != 0 && r != 0x20) {
        once = 0;
        first_x = x, first_y = y, first_got = r, first_want = 0x20;
      }
    }
  if (once && drawn < 3 * 150 * 150 / 2) {
    once = 0;
    first_x = first_y = 0, first_got = drawn, first_want = 3 * 150 * 150 / 2;
  }
  report("shared_edges_once", once);
}

/* --- Points of each width (section 3.3.1). ------------------------- */

static double px_w, py_w;
static int point_size;
static void want_point(int x, int y, double out[4]) {
  double cx, cy;
  if (point_size % 2) {
    cx = floor(px_w) + 0.5, cy = floor(py_w) + 0.5;
  } else {
    cx = floor(px_w + 0.5), cy = floor(py_w + 0.5);
  }
  double h = point_size / 2.0;
  int in = fabs(x + 0.5 - cx) < h && fabs(y + 0.5 - cy) < h;
  unpack(in ? inside : outside, out);
}

static void points(void) {
  outside = 0x000000ff, inside = 0xffff00ff;
  char name[64];
  for (int s = 1; s <= 6; s++) {
    px_w = 100.3 + 50 * s, py_w = 200.7 - 10 * s;
    point_size = s;
    scene(outside);
    colour(inside);
    glPointSizex(s * ONE);
    GLfixed v[2] = {dx((GLfixed)(px_w * ONE)), dy((GLfixed)(py_w * ONE))};
    glVertexPointer(2, GL_FIXED, 0, v);
    glDrawArrays(GL_POINTS, 0, 1);
    glPointSizex(ONE);
    read_window();
    snprintf(name, sizeof name, "point_size_%d", s);
    report(name, window_is(want_point, 0));
  }
}

/* --- Lines (section 3.4): the rules an algorithm other than the
 * diamond exit must keep. ------------------------------------------- */

/* One line, or a strip of `n` vertices, from window coordinates `p`, in
 * sixteenths, drawn added together, so that a pixel drawn twice is
 * brighter than one drawn once. */
static void draw_lines(GLenum mode, const long long *p, int n, int width) {
  scene(0x000000ff);
  glEnable(GL_BLEND);
  glBlendFunc(GL_ONE, GL_ONE);
  glLineWidthx(width * ONE);
  colour(0x40404040);
  GLfixed v[16];
  for (int k = 0; k < n; k++) {
    v[2 * k] = (GLfixed)(p[2 * k] * 8) - ONE;
    v[2 * k + 1] = (GLfixed)(p[2 * k + 1] * 16) - ONE;
  }
  glVertexPointer(2, GL_FIXED, 0, v);
  glDrawArrays(mode, 0, n);
  glLineWidthx(ONE);
  read_window();
}

/* The fragments of a line of width one from `a` to `b`, in sixteenths,
 * held to the specification's rules 1 to 3: none more than a unit from
 * the ideal line, one a column for an x-major line or a row for a
 * y-major one, none drawn twice, and as many as the line's columns or
 * rows, to within two for the ends, which the rules let differ. */
static int line_keeps_rules(const long long *a, const long long *b,
                            char *why, size_t room) {
  double ax = a[0] / 16.0, ay = a[1] / 16.0, bx = b[0] / 16.0,
         by = b[1] / 16.0;
  int xmajor = fabs(bx - ax) >= fabs(by - ay);
  static int count[W > H ? W : H];
  memset(count, 0, sizeof count);
  int total = 0;
  for (int y = 0; y < H; y++)
    for (int x = 0; x < W; x++) {
      int v = at(x, y, 0);
      if (v == 0)
        continue;
      if (v != 0x40) {
        snprintf(why, room, "%d,%d drawn twice", x, y);
        return 0;
      }
      total++;
      double cx = x + 0.5, cy = y + 0.5, off;
      if (xmajor) {
        double ty = ay + (by - ay) * (cx - ax) / (bx - ax);
        off = fabs(cy - ty);
        count[x]++;
      } else {
        double tx = ax + (bx - ax) * (cy - ay) / (by - ay);
        off = fabs(cx - tx);
        count[y]++;
      }
      if (off > 1.5) {
        snprintf(why, room, "%d,%d is %.2f from the line", x, y, off);
        return 0;
      }
    }
  for (int k = 0; k < (W > H ? W : H); k++)
    if (count[k] > 1) {
      snprintf(why, room, "%d fragments in %s %d", count[k],
               xmajor ? "column" : "row", k);
      return 0;
    }
  double span = xmajor ? fabs(bx - ax) : fabs(by - ay);
  if (fabs(total - span) > 2) {
    snprintf(why, room, "%d fragments for a span of %.1f", total, span);
    return 0;
  }
  return 1;
}

static void lines(void) {
  char name[64], why[128];
  for (int t = 0; t < 8; t++) {
    long long p[4];
    for (int k = 0; k < 2; k++) {
      p[2 * k] = 160 + next() % ((W - 20) * 16);
      p[2 * k + 1] = 160 + next() % ((H - 20) * 16);
    }
    draw_lines(GL_LINES, p, 2, 1);
    snprintf(name, sizeof name, "line_%d", t);
    if (line_keeps_rules(p, p + 2, why, sizeof why))
      conform_pass("render", name);
    else
      conform_fail("render", name, "%s", why);
  }
  /* Rule 4: a strip of two x-major segments, both left to right, neither
   * doubles nor drops the pixel at their shared end: one fragment in each
   * column from the first column past the start to the last before the
   * end. */
  static const long long strip[6] = {50 * 16 + 3, 100 * 16 + 5,
                                     300 * 16 + 9, 180 * 16 + 1,
                                     560 * 16 + 7, 120 * 16 + 11};
  draw_lines(GL_LINE_STRIP, strip, 3, 1);
  int ok = 1;
  for (int x = 52; x < 558 && ok; x++) {
    int n = 0;
    for (int y = 0; y < H; y++) {
      int v = at(x, y, 0);
      if (v > 0x40) {
        ok = 0;
        snprintf(why, sizeof why, "%d,%d drawn twice", x, y);
      }
      n += v != 0;
    }
    if (ok && n != 1) {
      ok = 0;
      snprintf(why, sizeof why, "%d fragments in column %d", n, x);
    }
  }
  if (ok)
    conform_pass("render", "line_strip_joins");
  else
    conform_fail("render", "line_strip_joins", "%s", why);
  /* A wide x-major line (section 3.4.2): each column between its ends
   * holds as many fragments as it is wide. */
  static const long long wide[4] = {100 * 16 + 4, 200 * 16 + 6,
                                    500 * 16 + 12, 260 * 16 + 2};
  for (int width = 2; width <= 5; width++) {
    draw_lines(GL_LINES, wide, 2, width);
    ok = 1;
    for (int x = 102; x < 498 && ok; x++) {
      int n = 0;
      for (int y = 0; y < H; y++)
        n += at(x, y, 0) != 0;
      if (n != width) {
        ok = 0;
        snprintf(why, sizeof why, "%d fragments in column %d", n, x);
      }
    }
    snprintf(name, sizeof name, "wide_line_%d", width);
    if (ok)
      conform_pass("render", name);
    else
      conform_fail("render", name, "%s", why);
  }
}

/* --- The per-fragment tests and operations (chapter 4). ------------ */

static int passes(GLenum func, double a, double b) {
  switch (func) {
  case GL_NEVER:
    return 0;
  case GL_LESS:
    return a < b;
  case GL_EQUAL:
    return a == b;
  case GL_LEQUAL:
    return a <= b;
  case GL_GREATER:
    return a > b;
  case GL_NOTEQUAL:
    return a != b;
  case GL_GEQUAL:
    return a >= b;
  default:
    return 1;
  }
}

/* A small rectangle's pixels, read back: (x, y) to (x + 7, y + 7). */
static int small_is(int x0, int y0, GLuint rgba, double tol) {
  double w[4];
  unpack(rgba, w);
  for (int y = y0; y < y0 + 8; y++)
    for (int x = x0; x < x0 + 8; x++)
      for (int c = 0; c < 4; c++)
        if (!near(at(x, y, c), w[c], tol)) {
          first_x = x, first_y = y, first_got = at(x, y, c);
          first_want = (int)w[c];
          return 0;
        }
  return 1;
}

static void depth_tests(void) {
  static const GLenum funcs[8] = {GL_NEVER,   GL_LESS,     GL_EQUAL,
                                  GL_LEQUAL,  GL_GREATER,  GL_NOTEQUAL,
                                  GL_GEQUAL,  GL_ALWAYS};
  char name[64];
  for (int f = 0; f < 8; f++) {
    GLuint back = 0x000000ff, a = 0xff0000ff, b = 0x00ff00ff;
    scene(back);
    glEnable(GL_DEPTH_TEST);
    glDepthFunc(GL_ALWAYS);
    /* A at window depth 0.5, object z 0; B at depth 0.25, object z
     * -0.5, over half of it and past it. */
    colour(a);
    rect(100 * ONE, 100 * ONE, 200 * ONE, 200 * ONE, 0);
    glDepthFunc(funcs[f]);
    colour(b);
    rect(150 * ONE, 100 * ONE, 250 * ONE, 200 * ONE, -ONE / 2);
    read_window();
    int over = passes(funcs[f], 0.25, 0.5), bare = passes(funcs[f], 0.25, 1);
    int ok = small_is(160, 140, over ? b : a, 0) &&
             small_is(220, 140, bare ? b : back, 0) &&
             small_is(110, 140, a, 0);
    snprintf(name, sizeof name, "depth_%04x", funcs[f]);
    report(name, ok);
  }
}

static void alpha_tests(void) {
  static const GLenum funcs[8] = {GL_NEVER,   GL_LESS,     GL_EQUAL,
                                  GL_LEQUAL,  GL_GREATER,  GL_NOTEQUAL,
                                  GL_GEQUAL,  GL_ALWAYS};
  char name[64];
  for (int f = 0; f < 8; f++) {
    for (int r = 0; r < 3; r++) {
      /* The reference below, at and above the fragment's alpha, 0x80. */
      static const GLfixed refs[3] = {ONE / 4, 0x8080, 3 * ONE / 4};
      GLuint back = 0x000000ff, a = 0x00ff0080;
      scene(back);
      glEnable(GL_ALPHA_TEST);
      glAlphaFuncx(funcs[f], refs[r]);
      colour(a);
      rect(100 * ONE, 100 * ONE, 200 * ONE, 200 * ONE, 0);
      read_window();
      double ra = refs[r] * 255.0 / ONE;
      int pass = passes(funcs[f], 128.0, round(ra));
      snprintf(name, sizeof name, "alpha_%04x_%d", funcs[f], r);
      report(name, small_is(120, 120, pass ? a : back, 0));
    }
  }
}

/* GL's blend factor `f` for channel `c` (3 is alpha) of source `s` and
 * destination `d`, each four doubles from 0 to 1. */
static double factor(GLenum f, const double *s, const double *d, int c) {
  switch (f) {
  case GL_ZERO:
    return 0;
  case GL_ONE:
    return 1;
  case GL_SRC_COLOR:
    return s[c];
  case GL_ONE_MINUS_SRC_COLOR:
    return 1 - s[c];
  case GL_DST_COLOR:
    return d[c];
  case GL_ONE_MINUS_DST_COLOR:
    return 1 - d[c];
  case GL_SRC_ALPHA:
    return s[3];
  case GL_ONE_MINUS_SRC_ALPHA:
    return 1 - s[3];
  case GL_DST_ALPHA:
    return d[3];
  case GL_ONE_MINUS_DST_ALPHA:
    return 1 - d[3];
  default: /* GL_SRC_ALPHA_SATURATE */
    return c == 3 ? 1 : (s[3] < 1 - d[3] ? s[3] : 1 - d[3]);
  }
}

static void blending(void) {
  static const GLenum src[9] = {
      GL_ZERO,      GL_ONE,           GL_DST_COLOR, GL_ONE_MINUS_DST_COLOR,
      GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_DST_ALPHA,
      GL_ONE_MINUS_DST_ALPHA, GL_SRC_ALPHA_SATURATE};
  static const GLenum dst[8] = {
      GL_ZERO,      GL_ONE,           GL_SRC_COLOR, GL_ONE_MINUS_SRC_COLOR,
      GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_DST_ALPHA,
      GL_ONE_MINUS_DST_ALPHA};
  GLuint dcol = 0x4080c060, scol = 0xc0602090;
  double s[4], d[4];
  unpack(scol, s);
  unpack(dcol, d);
  for (int k = 0; k < 4; k++)
    s[k] /= 255, d[k] /= 255;
  int failed = 0;
  char why[96] = "";
  for (int i = 0; i < 9; i++)
    for (int j = 0; j < 8; j++) {
      scene(dcol);
      glEnable(GL_BLEND);
      glBlendFunc(src[i], dst[j]);
      colour(scol);
      rect(100 * ONE, 100 * ONE, 120 * ONE, 120 * ONE, 0);
      read_window();
      for (int c = 0; c < 4; c++) {
        double v = s[c] * factor(src[i], s, d, c) +
                   d[c] * factor(dst[j], s, d, c);
        if (v > 1)
          v = 1;
        if (!near(at(110, 110, c), v * 255, 1) && !failed) {
          failed = 1;
          snprintf(why, sizeof why, "%04x %04x channel %d: %d, not %.1f",
                   src[i], dst[j], c, at(110, 110, c), v * 255);
        }
      }
    }
  if (failed)
    conform_fail("render", "blend_factors", "%s", why);
  else
    conform_pass("render", "blend_factors");
}

static void masks_and_logic(void) {
  GLuint back = 0x5a3cc396, ink = 0xa5c3693c;
  int bad = 0;
  char why[96] = "";
  for (int m = 0; m < 16; m++) {
    scene(back);
    glColorMask(m & 8 ? 1 : 0, m & 4 ? 1 : 0, m & 2 ? 1 : 0, m & 1 ? 1 : 0);
    colour(ink);
    rect(100 * ONE, 100 * ONE, 120 * ONE, 120 * ONE, 0);
    read_window();
    for (int c = 0; c < 4 && !bad; c++) {
      int keep = (m >> (3 - c)) & 1;
      int want = ((keep ? ink : back) >> (24 - 8 * c)) & 0xff;
      if (at(110, 110, c) != want) {
        bad = 1;
        snprintf(why, sizeof why, "mask %x channel %d: %d, not %d", m, c,
                 at(110, 110, c), want);
      }
    }
  }
  if (bad)
    conform_fail("render", "colour_mask", "%s", why);
  else
    conform_pass("render", "colour_mask");
  bad = 0;
  for (int op = 0; op < 16; op++) {
    scene(back);
    glEnable(GL_COLOR_LOGIC_OP);
    glLogicOp(GL_CLEAR + op);
    colour(ink);
    rect(100 * ONE, 100 * ONE, 120 * ONE, 120 * ONE, 0);
    read_window();
    GLuint want = 0;
    for (int bit = 0; bit < 32; bit++) {
      int s = (ink >> bit) & 1, d = (back >> bit) & 1;
      want |= (GLuint)((op >> (3 - (2 * s + d))) & 1) << bit;
    }
    for (int c = 0; c < 4 && !bad; c++) {
      int w = (want >> (24 - 8 * c)) & 0xff;
      if (at(110, 110, c) != w) {
        bad = 1;
        snprintf(why, sizeof why, "op %x channel %d: %d, not %d", op, c,
                 at(110, 110, c), w);
      }
    }
  }
  if (bad)
    conform_fail("render", "logic_ops", "%s", why);
  else
    conform_pass("render", "logic_ops");
}

static void fog(void) {
  GLuint back = 0x000000ff, ink = 0xffc08040, fogc = 0x2040a0ff;
  scene(back);
  glEnable(GL_FOG);
  glFogx(GL_FOG_MODE, GL_LINEAR);
  glFogx(GL_FOG_START, 0);
  glFogx(GL_FOG_END, 2 * ONE);
  GLfixed fc[4] = {0x20 * ONE / 255, 0x40 * ONE / 255, 0xa0 * ONE / 255, ONE};
  glFogxv(GL_FOG_COLOR, fc);
  colour(ink);
  /* Object z -0.5 is eye z -0.5: the factor is (2 - 0.5) / 2. */
  rect(100 * ONE, 100 * ONE, 140 * ONE, 140 * ONE, -ONE / 2);
  read_window();
  double f = 0.75, s[4], c[4];
  unpack(ink, s);
  unpack(fogc, c);
  int ok = 1;
  for (int k = 0; k < 3 && ok; k++) {
    double want = f * s[k] + (1 - f) * c[k];
    if (!near(at(120, 120, k), want, 2)) {
      ok = 0;
      first_x = first_y = 120, first_got = at(120, 120, k);
      first_want = (int)want;
    }
  }
  if (ok && at(120, 120, 3) != (int)s[3]) {
    ok = 0;
    first_got = at(120, 120, 3), first_want = (int)s[3];
  }
  report("fog_linear", ok);
}

static void stencil(void) {
  GLuint back = 0x000000ff, ink = 0x00ffffff;
  scene(back);
  glClearStencil(0);
  glClear(GL_STENCIL_BUFFER_BIT);
  glEnable(GL_STENCIL_TEST);
  glStencilFunc(GL_ALWAYS, 5, 0xff);
  glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
  glColorMask(0, 0, 0, 0);
  rect(100 * ONE, 100 * ONE, 150 * ONE, 150 * ONE, 0);
  glColorMask(1, 1, 1, 1);
  glStencilFunc(GL_EQUAL, 5, 0xff);
  glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
  colour(ink);
  rect(0, 0, W * ONE, H * ONE, 0);
  read_window();
  inside = ink, outside = back;
  box[0] = 100, box[1] = 100, box[2] = 50, box[3] = 50;
  report("stencil_equal", window_is(want_box, 0));
}

/* A read in the middle of a frame (#1504): the depth and the stencil the
 * frame made before glReadPixels hold for what it draws after, as GL
 * keeps both across a read. */
static void reads_mid_frame(void) {
  GLuint back = 0x000000ff, a = 0xff0000ff, b = 0x00ff00ff, c = 0x0000ffff;
  GLubyte one[4];
  /* The depth: cleared to 0.375, A drawn at 0.25 over the left, a read;
   * then B at 0.5 over both halves under GL_LESS, which neither lets
   * through, and C at 0.3, which only the cleared half does. */
  scene(back);
  glClearDepthx(ONE * 3 / 8);
  glClear(GL_DEPTH_BUFFER_BIT);
  glEnable(GL_DEPTH_TEST);
  glDepthFunc(GL_LESS);
  colour(a);
  rect(100 * ONE, 100 * ONE, 150 * ONE, 200 * ONE, -ONE / 2);
  glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, one);
  colour(b);
  rect(100 * ONE, 100 * ONE, 200 * ONE, 200 * ONE, 0);
  colour(c);
  rect(100 * ONE, 100 * ONE, 200 * ONE, 200 * ONE, -ONE * 2 / 5);
  read_window();
  report("read_keeps_depth",
         small_is(110, 140, a, 0) && small_is(170, 140, c, 0));
  /* The stencil: 5 written in a square with the colour masked, a read,
   * then the window drawn where the stencil is 5. */
  scene(back);
  glClearStencil(0);
  glClear(GL_STENCIL_BUFFER_BIT);
  glEnable(GL_STENCIL_TEST);
  glStencilFunc(GL_ALWAYS, 5, 0xff);
  glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
  glColorMask(0, 0, 0, 0);
  rect(100 * ONE, 100 * ONE, 150 * ONE, 150 * ONE, 0);
  glColorMask(1, 1, 1, 1);
  glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, one);
  glStencilFunc(GL_EQUAL, 5, 0xff);
  glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
  colour(b);
  rect(0, 0, W * ONE, H * ONE, 0);
  read_window();
  inside = b, outside = back;
  box[0] = 100, box[1] = 100, box[2] = 50, box[3] = 50;
  report("read_keeps_stencil", window_is(want_box, 0));
}

/* --- Texturing (section 3.7). -------------------------------------- */

static GLubyte texels[4 * 4 * 4];
static void want_texture(int x, int y, double out[4]) {
  if (x < 100 || x >= 164 || y < 100 || y >= 164) {
    unpack(outside, out);
    return;
  }
  int i = (x - 100) / 16, j = (y - 100) / 16;
  for (int c = 0; c < 4; c++)
    out[c] = texels[(j * 4 + i) * 4 + c];
}

static void draw_textured_rect(void) {
  GLfixed v[8] = {dx(100 * ONE), dy(100 * ONE), dx(164 * ONE), dy(100 * ONE),
                  dx(164 * ONE), dy(164 * ONE), dx(100 * ONE), dy(164 * ONE)};
  GLfixed st[8] = {0, 0, ONE, 0, ONE, ONE, 0, ONE};
  static const GLubyte idx[6] = {0, 1, 2, 0, 2, 3};
  glVertexPointer(2, GL_FIXED, 0, v);
  glTexCoordPointer(2, GL_FIXED, 0, st);
  glEnableClientState(GL_TEXTURE_COORD_ARRAY);
  glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, idx);
  glDisableClientState(GL_TEXTURE_COORD_ARRAY);
}

static void texturing(void) {
  for (int k = 0; k < 16; k++) {
    texels[4 * k] = (GLubyte)(k * 16);
    texels[4 * k + 1] = (GLubyte)(255 - k * 16);
    texels[4 * k + 2] = (GLubyte)(k * 7);
    texels[4 * k + 3] = 0xff;
  }
  outside = 0x000000ff;
  scene(outside);
  GLuint name;
  glGenTextures(1, &name);
  glBindTexture(GL_TEXTURE_2D, name);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
  glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE,
               texels);
  glTexEnvi(GL_TEXTURE_ENV, GL_TEXTURE_ENV_MODE, GL_REPLACE);
  glEnable(GL_TEXTURE_2D);
  draw_textured_rect();
  read_window();
  report("texture_nearest", window_is(want_texture, 0));
  /* The five environments, on a texture of one texel, by Table 3.15 for
   * an RGBA texture: REPLACE gives Ct; MODULATE Cf Ct; DECAL
   * Cf (1 - At) + Ct At; BLEND Cf (1 - Ct) + Cc Ct; ADD Cf + Ct, at most
   * one; the alpha Af At for all but REPLACE's At and DECAL's Af. */
  static const GLubyte one_texel[4] = {0x80, 0xc0, 0x40, 0x60};
  glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE,
               one_texel);
  static const GLenum modes[5] = {GL_REPLACE, GL_MODULATE, GL_DECAL, GL_BLEND,
                                  GL_ADD};
  GLuint frag = 0x4080ffc0, envc = 0xff200080;
  double cf[4], ct[4], cc[4];
  unpack(frag, cf);
  unpack(envc, cc);
  for (int k = 0; k < 4; k++)
    ct[k] = one_texel[k];
  char name2[64];
  for (int m = 0; m < 5; m++) {
    scene(outside);
    glEnable(GL_TEXTURE_2D);
    glTexEnvi(GL_TEXTURE_ENV, GL_TEXTURE_ENV_MODE, modes[m]);
    GLfixed e[4];
    for (int k = 0; k < 4; k++)
      e[k] = (GLfixed)(cc[k] * ONE / 255);
    glTexEnvxv(GL_TEXTURE_ENV, GL_TEXTURE_ENV_COLOR, e);
    colour(frag);
    draw_textured_rect();
    read_window();
    int ok = 1;
    for (int c = 0; c < 4 && ok; c++) {
      double f = cf[c] / 255, t = ct[c] / 255, at_ = ct[3] / 255,
             env = cc[c] / 255, want;
      switch (modes[m]) {
      case GL_REPLACE:
        want = t;
        break;
      case GL_MODULATE:
        want = f * t;
        break;
      case GL_DECAL:
        want = c == 3 ? f : f * (1 - at_) + t * at_;
        break;
      case GL_BLEND:
        want = c == 3 ? f * t : f * (1 - t) + env * t;
        break;
      default:
        want = c == 3 ? f * t : (f + t > 1 ? 1 : f + t);
      }
      if (!near(at(130, 130, c), want * 255, 1)) {
        ok = 0;
        first_x = first_y = 130, first_got = at(130, 130, c);
        first_want = (int)lround(want * 255);
      }
    }
    snprintf(name2, sizeof name2, "texture_env_%04x", modes[m]);
    report(name2, ok);
  }
  glDisable(GL_TEXTURE_2D);
  glDeleteTextures(1, &name);
}

/* --- Lighting (section 2.12.1). ------------------------------------ */

static void lighting(void) {
  outside = 0x000000ff;
  scene(outside);
  glEnable(GL_LIGHTING);
  glEnable(GL_LIGHT0);
  /* A directional light from (0, 0.6, 0.8), white diffuse, and GL's
   * defaults otherwise: the scene's ambient 0.2, the material's ambient
   * 0.2 and diffuse 0.8, light 0's ambient nought. */
  GLfixed pos[4] = {0, (GLfixed)(0.6 * ONE), (GLfixed)(0.8 * ONE), 0};
  glLightxv(GL_LIGHT0, GL_POSITION, pos);
  glNormal3x(0, 0, ONE);
  rect(100 * ONE, 100 * ONE, 140 * ONE, 140 * ONE, 0);
  read_window();
  double c = 0.2 * 0.2 + 0.8 * 0.8;
  int ok = 1;
  for (int k = 0; k < 3 && ok; k++)
    if (!near(at(120, 120, k), c * 255, 2)) {
      ok = 0;
      first_x = first_y = 120, first_got = at(120, 120, k);
      first_want = (int)lround(c * 255);
    }
  /* The alpha is the material's diffuse alpha, one. */
  if (ok && at(120, 120, 3) != 255) {
    ok = 0;
    first_got = at(120, 120, 3), first_want = 255;
  }
  report("lighting_diffuse", ok);
  glDisable(GL_LIGHTING);
}

void conform_render(void) {
  clears();
  scissor();
  viewport();
  triangles();
  shared_edges();
  points();
  lines();
  depth_tests();
  alpha_tests();
  blending();
  masks_and_logic();
  fog();
  stencil();
  reads_mid_frame();
  texturing();
  lighting();
}
