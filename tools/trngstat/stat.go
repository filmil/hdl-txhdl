// SPDX-License-Identifier: Apache-2.0

package main

import (
	"bufio"
	"fmt"
	"io"
	"math"
	"strconv"
	"strings"
)

// Capture is what the program printed: the raw words, the extractor's
// words, and whether it reached `end` or stopped on `fault`.
type Capture struct {
	Raw, Words []uint32
	Ended      bool
	Fault      bool
}

// Parse reads a capture. Lines before `raw` are the loader's and the
// program's greeting and are skipped; a line that is not eight hex
// digits inside a section is an error, since a dropped or garbled
// character on the serial line is exactly what must not be averaged
// away.
func Parse(r io.Reader) (Capture, error) {
	var c Capture
	var into *[]uint32
	s := bufio.NewScanner(r)
	for n := 1; s.Scan(); n++ {
		line := strings.TrimSpace(s.Text())
		switch line {
		case "raw":
			into = &c.Raw
			continue
		case "words":
			into = &c.Words
			continue
		case "end":
			c.Ended = true
			return c, nil
		case "fault":
			c.Fault = true
			return c, nil
		}
		if into == nil || line == "" {
			continue
		}
		if len(line) != 8 {
			return c, fmt.Errorf("line %d: %q is not a word", n, line)
		}
		v, err := strconv.ParseUint(line, 16, 32)
		if err != nil {
			return c, fmt.Errorf("line %d: %q is not a word", n, line)
		}
		*into = append(*into, uint32(v))
	}
	return c, s.Err()
}

// Stats are the numbers for one run of words, read as bits, the most
// significant first.
type Stats struct {
	Words, Bits int
	// Ones is the fraction of bits set; Z is its distance from one
	// half in standard deviations of a fair source.
	Ones, Z float64
	// Corr is the correlation of each bit with the next.
	Corr float64
	// MinEntropyBit is SP 800-90B's most common value estimate over
	// bits, per bit; MinEntropyByte is the same over bytes, per bit.
	MinEntropyBit, MinEntropyByte float64
	// Distinct counts the different words.
	Distinct int
}

func bits(words []uint32) []int {
	b := make([]int, 0, 32*len(words))
	for _, w := range words {
		for i := 31; i >= 0; i-- {
			b = append(b, int(w>>uint(i)&1))
		}
	}
	return b
}

// mcv is SP 800-90B section 6.3.1: the upper bound of a 99 percent
// confidence interval on the most common value's probability, and the
// min-entropy it leaves, per sample.
func mcv(counts map[int]int, n int) float64 {
	top := 0
	for _, c := range counts {
		if c > top {
			top = c
		}
	}
	p := float64(top) / float64(n)
	pu := math.Min(1, p+2.576*math.Sqrt(p*(1-p)/float64(n-1)))
	return -math.Log2(pu)
}

// Measure computes the numbers. It wants at least two words.
func Measure(words []uint32) Stats {
	b := bits(words)
	n := len(b)
	st := Stats{Words: len(words), Bits: n}

	ones := 0
	for _, x := range b {
		ones += x
	}
	st.Ones = float64(ones) / float64(n)
	st.Z = (st.Ones - 0.5) / (0.5 / math.Sqrt(float64(n)))

	// Pearson's coefficient between b[i] and b[i+1].
	var sx, sy, sxx, syy, sxy float64
	for i := 0; i+1 < n; i++ {
		x, y := float64(b[i]), float64(b[i+1])
		sx, sy = sx+x, sy+y
		sxx, syy, sxy = sxx+x*x, syy+y*y, sxy+x*y
	}
	m := float64(n - 1)
	den := math.Sqrt((m*sxx - sx*sx) * (m*syy - sy*sy))
	if den > 0 {
		st.Corr = (m*sxy - sx*sy) / den
	}

	st.MinEntropyBit = mcv(map[int]int{0: n - ones, 1: ones}, n)
	bytes := map[int]int{}
	for _, w := range words {
		for i := 0; i < 4; i++ {
			bytes[int(w>>uint(8*i)&0xff)]++
		}
	}
	st.MinEntropyByte = mcv(bytes, 4*len(words)) / 8

	seen := map[uint32]bool{}
	for _, w := range words {
		seen[w] = true
	}
	st.Distinct = len(seen)
	return st
}

// Judge says whether the extractor's words look like a fair source, by
// the bounds docs/board-checks.md states: the bias within four
// standard deviations, the serial correlation within four of its own,
// and at least 0.97 bits of min-entropy per bit over bits. A fair
// source of 4096 words scores about 0.986 there, the confidence
// interval alone costing a hundredth, and one four deviations off
// scores about 0.975, so the bound is the bias bound again with room.
// The raw
// words are measured and not judged, since folded samples before the
// extractor are not expected to be fair.
func Judge(s Stats) []string {
	var bad []string
	if math.Abs(s.Z) >= 4 {
		bad = append(bad, fmt.Sprintf("bias: z %.2f", s.Z))
	}
	if lim := 4 / math.Sqrt(float64(s.Bits)); math.Abs(s.Corr) >= lim {
		bad = append(bad, fmt.Sprintf("correlation %.5f, bound %.5f", s.Corr, lim))
	}
	if s.MinEntropyBit < 0.97 {
		bad = append(bad, fmt.Sprintf("min-entropy %.4f per bit", s.MinEntropyBit))
	}
	return bad
}
