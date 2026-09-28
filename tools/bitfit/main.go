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
	flag.Parse()
	off, err := strconv.ParseUint(*offset, 0, 32)
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
