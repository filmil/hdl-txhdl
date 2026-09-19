// SPDX-License-Identifier: Apache-2.0
// Reduces the TxHDL logo to the framebuffer's colour and size, and
// writes it as Rust: a width, a height, and one 12-bit word per pixel.
//
//	logo2rs -in txhdl-icon.png -w 34 -h 34 > lib/logo/logo.rs
//
// The video peripheral's framebuffer is 160 by 120 pixels of twelve
// bits, four each of red, green and blue. A logo in a corner of that
// is a few dozen pixels square, so the 512 by 512 source is averaged
// down in boxes and each box quantised to four bits a channel.
//
// This is run by hand and its output is committed, which is the same
// arrangement the layout numbers under docs/ have: a Bazel action
// cannot reach the shared drive the logo lives on, and the logo
// changes about as often as the die does.
//
// The background of the source is a dark gradient that reads as noise
// once it is four bits, so a pixel darker than -dark is written as
// transparent, which is the one colour a caller can test for and skip.
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
	w := flag.Int("w", 34, "width in framebuffer pixels")
	h := flag.Int("h", 34, "height in framebuffer pixels")
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
	words := make([]uint16, 0, *w**h)
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
			if n == 0 {
				words = append(words, transparent)
				continue
			}
			r8 := int(sr / n)
			g8 := int(sg / n)
			b8 := int(sb / n)
			if r8+g8+b8 <= *dark*3 {
				words = append(words, transparent)
				continue
			}
			words = append(words, pack(r8, g8, b8))
		}
	}

	fmt.Print(rust(filepath.Base(*in), *w, *h, words))
}

// The colour a caller skips. Twelve bits hold a colour, so the
// thirteenth says there is none.
const transparent = 0x1000

// pack turns eight bits a channel into four, rounding rather than
// truncating, so a value just under a step does not fall a step.
func pack(r, g, b int) uint16 {
	q := func(v int) uint16 {
		n := (v*15 + 127) / 255
		if n > 15 {
			n = 15
		}
		return uint16(n)
	}
	return q(r)<<8 | q(g)<<4 | q(b)
}

func rust(in string, w, h int, words []uint16) string {
	s := `// SPDX-License-Identifier: Apache-2.0
// The TxHDL logo, at the size and the colour a framebuffer pixel has.
//
// Written by //tools/logo2rs from ` + in + `, and committed, because a
// Bazel action cannot reach the drive the logo is kept on. Regenerate
// it with the command in that tool's comment when the logo changes.
//
// A word is a colour, four bits each of red, green and blue from the
// top. TRANSPARENT is not a colour: it is what the background of the
// logo became, and a caller skips it rather than drawing it, so the
// logo sits on whatever is already there.
//
// The crate is no_std: it is constants, and it is read by programs
// compiled for the core, where there is no std to find.
#![no_std]

/// Columns.
pub const W: usize = ` + fmt.Sprint(w) + `;
/// Rows.
pub const H: usize = ` + fmt.Sprint(h) + `;
/// Not a colour: leave what is underneath.
pub const TRANSPARENT: u16 = 0x1000;

/// The logo, a row at a time from the top.
pub const PIXELS: [u16; W * H] = [
`
	for i, v := range words {
		if i%8 == 0 {
			s += "   "
		}
		s += fmt.Sprintf(" 0x%03x,", v)
		if i%8 == 7 {
			s += "\n"
		}
	}
	if len(words)%8 != 0 {
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
