// SPDX-License-Identifier: Apache-2.0
//
// Whether a bitstream leaves the configuration flash's program area
// alone (issue 312).
//
// The flash holds the bitstream from offset zero and programs from
// `-offset` on, 0x00A0_0000 on the AX7A200B. This reads each `.bit`
// file's header, prints the length of its configuration data, the part
// and the room left before the offset, and exits 1 if the data runs
// past the offset, which would overwrite a program or be overwritten
// by one.
// An uncompressed bitstream's length is fixed by the part, so what
// changes it is compression, or another part.
//
//	bazel run //tools/bitfit -- -offset=0xa00000 $PWD/design.bit
package main

import (
	"flag"
	"fmt"
	"os"
	"strconv"
)

func main() {
	offset := flag.String("offset", "0xa00000", "where the programs begin in the flash")
	image := flag.String("image", "", "a flash image (.mcs) to check instead of bitstreams")
	program := flag.String("program", "", "with -image: the file the image must hold at the offset")
	flag.Parse()
	off, err := strconv.ParseUint(*offset, 0, 32)
	if err == nil && *image != "" {
		os.Exit(checkImage(*image, *program, uint32(off)))
	}
	if err != nil || flag.NArg() == 0 {
		fmt.Fprintln(os.Stderr, "usage: bitfit -offset=0xa00000 <design.bit>...")
		os.Exit(2)
	}
	ok := true
	for _, path := range flag.Args() {
		b, err := os.ReadFile(path)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(2)
		}
		bit, err := Parse(b)
		if err != nil {
			fmt.Fprintf(os.Stderr, "%s: %v\n", path, err)
			os.Exit(2)
		}
		room := int(off) - bit.DataLen
		fmt.Printf("%s: %s for %s, %d bytes of configuration data\n",
			path, bit.Design, bit.Part, bit.DataLen)
		if room >= 0 {
			fmt.Printf("  %d bytes to spare before the programs at 0x%x\n", room, off)
		} else {
			fmt.Printf("FAIL %s runs %d bytes into the programs at 0x%x\n", path, -room, off)
			ok = false
		}
	}
	if !ok {
		os.Exit(1)
	}
}

// checkImage reads a flash image and says whether it holds `program`
// byte for byte at `off`, and where what sits below the programs, the
// bitstream, ends. It returns the exit status: 1 when the program is
// missing or differs, 2 when a file cannot be read.
func checkImage(image, program string, off uint32) int {
	text, err := os.ReadFile(image)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		return 2
	}
	img, err := ParseMCS(text)
	if err != nil {
		fmt.Fprintf(os.Stderr, "%s: %v\n", image, err)
		return 2
	}
	want, err := os.ReadFile(program)
	if err != nil || len(want) == 0 {
		fmt.Fprintln(os.Stderr, "usage: bitfit -image=<flash.mcs> -program=<program.bin> [-offset=0xa00000]")
		return 2
	}
	if hi, ok := img.Below(off); ok {
		fmt.Printf("%s: the bitstream ends at 0x%x, %d bytes before the programs at 0x%x\n",
			image, hi, off-hi-1, off)
	}
	if at, ok := img.Holds(want, off); !ok {
		fmt.Printf("FAIL %s does not hold %s at 0x%x: it differs at 0x%x\n", image, program, off, at)
		return 1
	}
	fmt.Printf("  holds %s, %d bytes, at 0x%x\n", program, len(want), off)
	return 0
}
