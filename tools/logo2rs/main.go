// SPDX-License-Identifier: Apache-2.0
// Reduces the TxHDL logo to the size it is shown at, and writes it as
// Rust: a width, a height, a palette of sixteen colours and a 4-bit
// index a pixel.
//
//	logo2rs -in txhdl-icon.png -w 144 -h 144 > lib/logo/logo.rs
//
// The logo is drawn one logo pixel to one screen pixel, 144 square in
// a corner of the 640 by 480 scanout, so that the wordmark under the
// hat can be read (issue 1213). At 36 square, four screen pixels to a
// logo pixel, it could not. The 512 by 512 source is averaged down in
// boxes, which is a proper downscale rather than a pick of one pixel
// in each box.
//
// A pixel is an index into a palette of sixteen 24-bit colours, the
// scanout's 0x00RRGGBB, two pixels to a byte: 144 square is 10 KB that
// way where a word a pixel was 41 KB. Index 0 is not a colour: it is
// what the background of the logo became, and a caller skips it, so
// the logo sits on whatever is already drawn. The fifteen colours are
// the logo's own, found by k-means from a fixed start, so the output is
// the same every run.
//
// This is run by hand and its output is committed, which is the same
// arrangement the layout numbers under docs/ have: a Bazel action
// cannot reach the shared drive the logo lives on, and the logo
// changes about as often as the die does.
//
// The background of the source is a dark gradient that reads as noise
// in a corner, so a box as dark as -dark or darker is transparent.
// That leaves the hat and the lettering, which is what a logo in a
// corner is for.
//
// The standard library alone: image/png is in it.
package main

import (
	"flag"
	"fmt"
	"image"
	_ "image/png"
	"os"
	"path/filepath"
)

func main() {
	in := flag.String("in", "", "the logo, a PNG")
	w := flag.Int("w", 36, "width in framebuffer pixels")
	h := flag.Int("h", 36, "height in framebuffer pixels")
	dark := flag.Int("dark", 60, "a box this dark or darker is transparent")
	flag.Parse()
	if *in == "" {
		fmt.Fprintln(os.Stderr, "usage: logo2rs -in LOGO.png [-w N] [-h N]")
		os.Exit(2)
	}

	f, err := os.Open(*in)
	check(err)
	defer f.Close()
	src, _, err := image.Decode(f)
	check(err)
	b := src.Bounds()

	// One output pixel per box of the source, averaged. The source is
	// square and the output need not be, so each axis scales on its own.
	// A transparent pixel is nil.
	px := make([]*[3]float64, 0, *w**h)
	for y := 0; y < *h; y++ {
		for x := 0; x < *w; x++ {
			x0 := b.Min.X + x*b.Dx() / *w
			x1 := b.Min.X + (x+1)*b.Dx() / *w
			y0 := b.Min.Y + y*b.Dy() / *h
			y1 := b.Min.Y + (y+1)*b.Dy() / *h
			var sr, sg, sb, n uint64
			for yy := y0; yy < y1; yy++ {
				for xx := x0; xx < x1; xx++ {
					r, g, bl, _ := src.At(xx, yy).RGBA()
					sr += uint64(r >> 8)
					sg += uint64(g >> 8)
					sb += uint64(bl >> 8)
					n++
				}
			}
			if n == 0 || int(sr/n+sg/n+sb/n) <= *dark*3 {
				px = append(px, nil)
				continue
			}
			px = append(px, &[3]float64{
				float64(sr) / float64(n),
				float64(sg) / float64(n),
				float64(sb) / float64(n),
			})
		}
	}
	pal := palette(px, 15)
	idx := make([]byte, len(px))
	for i, p := range px {
		if p != nil {
			idx[i] = byte(1 + nearest(pal, *p))
		}
	}
	fmt.Print(rust(filepath.Base(*in), *w, *h, pal, idx))
}

func dist(a, b [3]float64) float64 {
	d0, d1, d2 := a[0]-b[0], a[1]-b[1], a[2]-b[2]
	return d0*d0 + d1*d1 + d2*d2
}

func nearest(pal [][3]float64, p [3]float64) int {
	best, bd := 0, dist(pal[0], p)
	for i := 1; i < len(pal); i++ {
		if d := dist(pal[i], p); d < bd {
			best, bd = i, d
		}
	}
	return best
}

// palette finds k colours for the opaque pixels by k-means. It starts
// from the first opaque pixel and then, one at a time, the pixel
// furthest from every colour chosen so far, so the start and therefore
// the result are fixed; then it moves each colour to the mean of the
// pixels nearest it until none moves.
func palette(px []*[3]float64, k int) [][3]float64 {
	var pts [][3]float64
	for _, p := range px {
		if p != nil {
			pts = append(pts, *p)
		}
	}
	pal := [][3]float64{pts[0]}
	for len(pal) < k {
		far, fd := 0, -1.0
		for i, p := range pts {
			if d := dist(pal[nearest(pal, p)], p); d > fd {
				far, fd = i, d
			}
		}
		if fd == 0 {
			break
		}
		pal = append(pal, pts[far])
	}
	for it := 0; it < 100; it++ {
		sum := make([][4]float64, len(pal))
		for _, p := range pts {
			c := nearest(pal, p)
			sum[c][0] += p[0]
			sum[c][1] += p[1]
			sum[c][2] += p[2]
			sum[c][3]++
		}
		moved := false
		for c := range pal {
			if sum[c][3] == 0 {
				continue
			}
			n := sum[c][3]
			m := [3]float64{sum[c][0] / n, sum[c][1] / n, sum[c][2] / n}
			if dist(m, pal[c]) > 1e-9 {
				moved = true
			}
			pal[c] = m
		}
		if !moved {
			break
		}
	}
	return pal
}

func rgb(c [3]float64) uint32 {
	q := func(v float64) uint32 {
		n := int(v + 0.5)
		if n > 255 {
			n = 255
		}
		return uint32(n)
	}
	return q(c[0])<<16 | q(c[1])<<8 | q(c[2])
}

func rust(in string, w, h int, pal [][3]float64, idx []byte) string {
	s := `// SPDX-License-Identifier: Apache-2.0
// The TxHDL logo, at the size it is shown: one logo pixel to one screen
// pixel (issue 1213).
//
// Written by //tools/logo2rs from ` + in + `, and committed, because a
// Bazel action cannot reach the drive the logo is kept on. Regenerate
// it with the command in that tool's comment when the logo changes.
//
// A pixel is a 4-bit index into PALETTE, two to a byte, the lower
// nibble first. Index 0 is not a colour: it is what the background of
// the logo became, and a caller skips it, so the logo sits on whatever
// is already there. A colour is the scanout's 0x00RRGGBB.
//
// The crate is no_std: it is constants and two small functions, and it
// is read by programs compiled for the core, where there is no std.
#![no_std]

/// Columns.
pub const W: usize = ` + fmt.Sprint(w) + `;
/// Rows.
pub const H: usize = ` + fmt.Sprint(h) + `;

/// The colours, index 0 unused. Four to a line, which rustfmt is told
/// to leave, as below.
#[rustfmt::skip]
pub const PALETTE: [u32; 16] = [
`
	cols := make([]uint32, 16)
	for i, c := range pal {
		cols[1+i] = rgb(c)
	}
	for i := 0; i < 16; i += 4 {
		s += fmt.Sprintf("    0x%06x, 0x%06x, 0x%06x, 0x%06x,\n",
			cols[i], cols[i+1], cols[i+2], cols[i+3])
	}
	s += `];

/// A pixel's index, 0 where the logo is transparent.
pub const fn index(x: usize, y: usize) -> u8 {
    let i = y * W + x;
    (PIXELS[i / 2] >> (4 * (i % 2))) & 0xf
}

/// A pixel's colour, or None where the logo is transparent.
pub const fn colour(x: usize, y: usize) -> Option<u32> {
    match index(x, y) {
        0 => None,
        n => Some(PALETTE[n as usize]),
    }
}

/// The indices, a row at a time from the top, two to a byte.
///
/// Sixteen to a line, which rustfmt would reflow to as many as fit. It
/// is told not to: nothing here is written by hand, so there is
/// nothing for a formatter to improve, and without this the pass that
/// the repository requires before a commit rewrites this file in
/// every branch that runs it (issue 437).
#[rustfmt::skip]
pub const PIXELS: [u8; (W * H).div_ceil(2)] = [
`
	packed := make([]byte, (len(idx)+1)/2)
	for i, v := range idx {
		packed[i/2] |= v << (4 * uint(i%2))
	}
	for i, v := range packed {
		if i%16 == 0 {
			s += "   "
		}
		s += fmt.Sprintf(" 0x%02x,", v)
		if i%16 == 15 {
			s += "\n"
		}
	}
	if len(packed)%16 != 0 {
		s += "\n"
	}
	return s + "];\n"
}

func check(err error) {
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
