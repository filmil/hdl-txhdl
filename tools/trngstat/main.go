// SPDX-License-Identifier: Apache-2.0
//
// The entropy source's capture read into numbers (issue 458).
//
// `trng_ram_bin` prints `trng ok`, then `raw` and 4096 words of the
// folded samples before the extractor, then `words` and 4096 words of
// the extractor's output, then `end`, each word as eight hex digits on
// a line. This reads that capture, as the serial watcher saved it, and
// prints for each half the bias, the serial correlation of bits and
// the most common value estimate of min-entropy of NIST SP 800-90B,
// section 6.3.1, over bits and over bytes. It exits 1 when the
// extractor's words fail the bounds `Judge` states, or the capture is
// short or stopped on a fault.
//
//	bazel run //tools/trngstat -- $PWD/trng.log
package main

import (
	"fmt"
	"os"
)

func report(name string, s Stats) {
	fmt.Printf("%s: %d words, %d distinct\n", name, s.Words, s.Distinct)
	fmt.Printf("  ones %.5f (z %+.2f)\n", s.Ones, s.Z)
	fmt.Printf("  serial correlation %+.5f\n", s.Corr)
	fmt.Printf("  min-entropy (MCV) %.4f per bit over bits, %.4f per bit over bytes\n",
		s.MinEntropyBit, s.MinEntropyByte)
}

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: trngstat <capture>")
		os.Exit(2)
	}
	f, err := os.Open(os.Args[1])
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	defer f.Close()
	c, err := Parse(f)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	ok := true
	if c.Fault {
		fmt.Println("the program stopped on `fault`: the health test tripped")
		ok = false
	} else if !c.Ended {
		fmt.Println("the capture has no `end`: it was cut short")
		ok = false
	}
	if len(c.Raw) >= 2 {
		report("raw", Measure(c.Raw))
	}
	if len(c.Words) < 2 {
		fmt.Println("no extractor words to judge")
		os.Exit(1)
	}
	s := Measure(c.Words)
	report("words", s)
	for _, b := range Judge(s) {
		fmt.Println("  FAIL", b)
		ok = false
	}
	if !ok {
		os.Exit(1)
	}
	fmt.Println("words: within bounds")
}
