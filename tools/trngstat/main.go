// SPDX-License-Identifier: Apache-2.0
//
// The entropy source's captures read into numbers (issues 458 and
// 794).
//
// `trng_ram_bin` prints `trng ok`, then `raw` and 4096 words of the
// samples before the extractor, then `words` and 4096 words of the
// extractor's output, then `end`, each word as eight hex digits on a
// line; a program may also print `rawrun`, words of samples in a row.
// This reads one or more captures, as the serial watcher saved them,
// and prints for each, and for all of them pooled when there are
// several:
//
//   - the bias, and the most common value estimate of min-entropy of
//     NIST SP 800-90B, section 6.3.1, over bits and over bytes;
//   - the correlation of bits inside a word by lag, 1 to 31, for the
//     raw words and for the extractor's, with one standard deviation
//     for a fair, independent source beside each; the extractor's lags
//     also as the source's samples they sit about apart, four a bit
//     times the fold;
//   - for a `rawrun`, the correlation of samples by lag out to -maxlag.
//
// It exits 1 when any capture's extractor words, or the pool's, fail
// the bounds `Judge` states, or a capture is short or stopped on a
// fault.
//
//	bazel run //tools/trngstat -- [-fold=2] [-maxlag=128] $PWD/trng-*.log
package main

import (
	"flag"
	"fmt"
	"math"
	"os"
)

// report prints one set of words' numbers. fold is the samples the
// hardware folds into one before the extractor, so that an extractor
// lag can be said in samples; zero for raw words, whose lags already
// are.
func report(name string, s Stats, fold int) {
	fmt.Printf("%s: %d words, %d distinct\n", name, s.Words, s.Distinct)
	fmt.Printf("  ones %.5f (z %+.2f)\n", s.Ones, s.Z)
	fmt.Printf("  min-entropy (MCV) %.4f per bit over bits, %.4f per bit over bytes\n",
		s.MinEntropyBit, s.MinEntropyByte)
	lags(s.Lags, fold)
}

func lags(ls []Lag, fold int) {
	if len(ls) == 0 {
		return
	}
	fmt.Printf("  correlation inside a word, sigma %.5f at lag 1\n", ls[0].Sigma())
	for _, l := range ls {
		mark := ""
		if math.Abs(l.Z) >= 4 {
			mark = "  <"
		}
		if fold > 0 {
			fmt.Printf("    lag %2d (~%3d samples) %+.5f z %+6.1f%s\n",
				l.Lag, 4*fold*l.Lag, l.R, l.Z, mark)
		} else {
			fmt.Printf("    lag %2d %+.5f z %+6.1f%s\n", l.Lag, l.R, l.Z, mark)
		}
	}
}

func main() {
	fold := flag.Int("fold", 1, "samples folded into one before the extractor")
	maxlag := flag.Int("maxlag", 128, "the longest lag measured over a rawrun")
	flag.Parse()
	if flag.NArg() == 0 {
		fmt.Fprintln(os.Stderr, "usage: trngstat [-fold=N] [-maxlag=N] <capture>...")
		os.Exit(2)
	}
	ok := true
	var pool Capture
	for _, path := range flag.Args() {
		f, err := os.Open(path)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(2)
		}
		c, err := Parse(f)
		f.Close()
		if err != nil {
			fmt.Fprintf(os.Stderr, "%s: %v\n", path, err)
			os.Exit(2)
		}
		fmt.Printf("== %s\n", path)
		if !one(c, *fold, *maxlag) {
			ok = false
		}
		pool.Raw = append(pool.Raw, c.Raw...)
		pool.Words = append(pool.Words, c.Words...)
	}
	if flag.NArg() > 1 {
		// Pooled: pairs stay inside a word, so words from different
		// captures never pair, and pooling only adds pairs.
		fmt.Printf("== pooled, %d captures\n", flag.NArg())
		pool.Ended = true
		if !one(pool, *fold, *maxlag) {
			ok = false
		}
	}
	if !ok {
		os.Exit(1)
	}
}

// one reports a capture and says whether it passes.
func one(c Capture, fold, maxlag int) bool {
	ok := true
	if c.Fault {
		fmt.Println("the program stopped on `fault`: the health test tripped")
		ok = false
	} else if !c.Ended {
		fmt.Println("the capture has no `end`: it was cut short")
		ok = false
	}
	if len(c.Raw) >= 2 {
		report("raw", Measure(c.Raw), 0)
	}
	if len(c.RawRun) >= 2 {
		fmt.Printf("rawrun: %d samples in a row\n", 32*len(c.RawRun))
		lags(RunLags(c.RawRun, maxlag), 0)
	}
	if len(c.Words) < 2 {
		fmt.Println("no extractor words to judge")
		return false
	}
	s := Measure(c.Words)
	report("words", s, fold)
	for _, b := range Judge(s) {
		fmt.Println("  FAIL", b)
		ok = false
	}
	if ok {
		fmt.Println("words: within bounds")
	}
	return ok
}
