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
// words, a run of raw words if the program printed one, and whether it
// reached `end` or stopped on `fault`.
//
// A raw word is a window of 32 samples in a row, the oldest in bit 31,
// and two raw words are far apart in time: the program reads one while
// the serial line is busy with the last. A `rawrun` is the other kind:
// words whose samples follow on from each other, the oldest in bit 31
// of the first, so a lag longer than a word can be measured.
type Capture struct {
	Raw, RawRun, Words []uint32
	Ended              bool
	Fault              bool
}

// Parse reads a capture. Lines before the first section are the
// loader's and the program's greeting and are skipped; a line that is
// not eight hex digits inside a section is an error, since a dropped
// or garbled character on the serial line is exactly what must not be
// averaged away.
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
		case "rawrun":
			into = &c.RawRun
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

// Stats are the numbers for one set of words, read as bits, the most
// significant first.
type Stats struct {
	Words, Bits int
	// Ones is the fraction of bits set; Z is its distance from one
	// half in standard deviations of a fair source.
	Ones, Z float64
	// Lags is the correlation of a bit with the bit `lag` further on in
	// the same word, for lags 1 to 31.
	Lags []Lag
	// MinEntropyBit is SP 800-90B's most common value estimate over
	// bits, per bit; MinEntropyByte is the same over bytes, per bit.
	MinEntropyBit, MinEntropyByte float64
	// Distinct counts the different words.
	Distinct int
}

// Lag is one correlation: Pearson's coefficient R of pairs of bits Lag
// apart, over N pairs, and Z, R in standard deviations of a fair,
// independent source, which is R times the root of N.
type Lag struct {
	Lag  int
	R, Z float64
	N    int
}

// Sigma is one standard deviation of R for a fair, independent source
// over the lag's pairs.
func (l Lag) Sigma() float64 { return 1 / math.Sqrt(float64(l.N)) }

func bits(words []uint32) []int {
	b := make([]int, 0, 32*len(words))
	for _, w := range words {
		for i := 31; i >= 0; i-- {
			b = append(b, int(w>>uint(i)&1))
		}
	}
	return b
}

func pearson(sx, sy, sxx, syy, sxy, n float64) float64 {
	den := math.Sqrt((n*sxx - sx*sx) * (n*syy - sy*sy))
	if den == 0 {
		return 0
	}
	return (n*sxy - sx*sy) / den
}

// WordLags is the correlation of bits `lag` apart for lags 1 to max,
// pairs taken inside a word only: two words the program read one after
// the other were not made one after the other, so a pair across them
// says nothing about the source and dilutes what does.
func WordLags(words []uint32, max int) []Lag {
	out := make([]Lag, 0, max)
	for lag := 1; lag <= max && lag < 32; lag++ {
		var sx, sy, sxx, syy, sxy, n float64
		for _, w := range words {
			for i := 0; i+lag < 32; i++ {
				x := float64(w >> uint(i) & 1)
				y := float64(w >> uint(i+lag) & 1)
				sx, sy = sx+x, sy+y
				sxx, syy, sxy = sxx+x*x, syy+y*y, sxy+x*y
				n++
			}
		}
		r := pearson(sx, sy, sxx, syy, sxy, n)
		out = append(out, Lag{lag, r, r * math.Sqrt(n), int(n)})
	}
	return out
}

// RunLags is the correlation of samples `lag` apart for lags 1 to max
// over a run of samples in a row, which a `rawrun` is.
func RunLags(words []uint32, max int) []Lag {
	b := bits(words)
	out := make([]Lag, 0, max)
	for lag := 1; lag <= max && lag < len(b); lag++ {
		var sx, sy, sxx, syy, sxy, n float64
		for i := 0; i+lag < len(b); i++ {
			x, y := float64(b[i]), float64(b[i+lag])
			sx, sy = sx+x, sy+y
			sxx, syy, sxy = sxx+x*x, syy+y*y, sxy+x*y
			n++
		}
		r := pearson(sx, sy, sxx, syy, sxy, n)
		out = append(out, Lag{lag, r, r * math.Sqrt(n), int(n)})
	}
	return out
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
	st.Lags = WordLags(words, 31)

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

// JudgedLags is how many of the words' lags the bound applies to. An
// output bit of von Neumann's extractor comes from one pair of samples
// in about four, so bits one to eight apart in a word sit some 4 to 32
// samples apart in the source, eight to sixty-four with a fold of two:
// the separations a source's periodic structure shows at (issue 780).
const JudgedLags = 8

// Judge says whether the extractor's words look like a fair source, by
// the bounds docs/board-checks.md states: the bias within four
// standard deviations; the correlation inside a word at every lag from
// one to JudgedLags within four of its own; and at least 0.97 bits of
// min-entropy per bit over bits. A fair source of 4096 words scores
// about 0.986 there, the confidence interval alone costing a
// hundredth, and one four deviations off scores about 0.975, so the
// bound is the bias bound again with room. The raw words are measured
// and not judged, since samples before the extractor are not expected
// to be fair.
func Judge(s Stats) []string {
	var bad []string
	if math.Abs(s.Z) >= 4 {
		bad = append(bad, fmt.Sprintf("bias: z %.2f", s.Z))
	}
	for _, l := range s.Lags {
		if l.Lag > JudgedLags {
			break
		}
		if math.Abs(l.Z) >= 4 {
			bad = append(bad, fmt.Sprintf("correlation at lag %d: %+.5f, z %+.1f, bound %.5f",
				l.Lag, l.R, l.Z, 4*l.Sigma()))
		}
	}
	if s.MinEntropyBit < 0.97 {
		bad = append(bad, fmt.Sprintf("min-entropy %.4f per bit", s.MinEntropyBit))
	}
	return bad
}
