// SPDX-License-Identifier: Apache-2.0

package main

import (
	"math"
	"math/rand"
	"strings"
	"testing"
)

func TestParseSkipsTheGreetingAndReadsEverySection(t *testing.T) {
	in := "ok 1234abcd\ntrng ok\nraw\r\n0000000f\nFFFFFFFF\n" +
		"rawrun\n00000001\n80000000\nwords\n12345678\nend\n"
	c, err := Parse(strings.NewReader(in))
	if err != nil {
		t.Fatal(err)
	}
	if !c.Ended || c.Fault {
		t.Fatalf("ended %v fault %v", c.Ended, c.Fault)
	}
	if len(c.Raw) != 2 || c.Raw[1] != 0xffffffff || len(c.RawRun) != 2 ||
		c.RawRun[1] != 0x80000000 || len(c.Words) != 1 {
		t.Fatalf("got %+v", c)
	}
}

// The joined run and the capture are two runs, each read into a field
// of its own, as main.go measures each on its own (issue 835).
func TestParseKeepsTheJoinedRunAndTheCaptureApart(t *testing.T) {
	in := "trng ok\nrawrun pairs 1 fit 1 moved 0 gap 0 bad 0\n" +
		"rawrun\n00000001\nraw\n00000000\nrawcap\n80000000\nffffffff\n" +
		"words\n12345678\nend\n"
	c, err := Parse(strings.NewReader(in))
	if err != nil {
		t.Fatal(err)
	}
	if len(c.RawRun) != 1 || c.RawRun[0] != 1 {
		t.Errorf("rawrun %x", c.RawRun)
	}
	if len(c.RawCap) != 2 || c.RawCap[0] != 0x80000000 || c.RawCap[1] != 0xffffffff {
		t.Errorf("rawcap %x", c.RawCap)
	}
}

// The issue's reproduction: two sections under one heading used to be
// appended into one run of 64 samples. They are refused now, naming
// both lines.
func TestParseRefusesAHeadingSeenTwice(t *testing.T) {
	in := "rawrun\n00000001\nrawrun\n80000000\nend\n"
	c, err := Parse(strings.NewReader(in))
	if err == nil {
		t.Fatalf("parsed as one run: %x", c.RawRun)
	}
	if !strings.Contains(err.Error(), "line 3") || !strings.Contains(err.Error(), "after line 1") {
		t.Errorf("error %q does not name both headings", err)
	}
}

func TestParseRefusesAGarbledWord(t *testing.T) {
	_, err := Parse(strings.NewReader("raw\n1234567\n"))
	if err == nil {
		t.Fatal("a seven digit word was taken")
	}
}

func TestParseStopsOnAFault(t *testing.T) {
	c, err := Parse(strings.NewReader("raw\n00000000\nwords\nfault\n"))
	if err != nil || !c.Fault {
		t.Fatalf("fault %v, err %v", c.Fault, err)
	}
}

// periodic is a fair source whose sample repeats the one `period`
// back with probability q, and is a fresh draw otherwise: correlated at
// the period and its multiples, and at nothing between.
func periodic(r *rand.Rand, n, period int, q float64) []int {
	s := make([]int, n)
	for i := range s {
		if i >= period && r.Float64() < q {
			s[i] = s[i-period]
		} else {
			s[i] = r.Intn(2)
		}
	}
	return s
}

// pack puts samples into words, the oldest in bit 31 of the first.
func pack(s []int) []uint32 {
	w := make([]uint32, 0, len(s)/32)
	for i := 0; i+32 <= len(s); i += 32 {
		var v uint32
		for _, b := range s[i : i+32] {
			v = v<<1 | uint32(b)
		}
		w = append(w, v)
	}
	return w
}

// A run of samples with a period of twelve shows twelve and twice
// twelve, and nothing between (issue 794): a contiguous capture is
// what measures a period longer than a word. The correlation at k
// periods is q to the k in this source, so the further multiples fade
// into the noise and are not asserted.
func TestARunShowsItsPeriod(t *testing.T) {
	r := rand.New(rand.NewSource(794))
	run := pack(periodic(r, 1<<17, 12, 0.3))
	ls := RunLags(run, 64)
	for _, l := range ls {
		onPeriod := l.Lag%12 == 0
		if (l.Lag == 12 || l.Lag == 24) && l.Z < 4 {
			t.Errorf("lag %d is the period's and was not seen: z %+.1f", l.Lag, l.Z)
		}
		if !onPeriod && math.Abs(l.Z) >= 4.5 {
			t.Errorf("lag %d is off the period and stood out: z %+.1f", l.Lag, l.Z)
		}
	}
	if len(ls) != 64 {
		t.Fatalf("%d lags, want 64", len(ls))
	}
}

// The same source read as windows of 32 far apart, which is what raw
// words are, shows a period shorter than a word, and pairs never cross
// a window.
func TestWindowsShowAShortPeriodInside(t *testing.T) {
	r := rand.New(rand.NewSource(12))
	var windows []uint32
	for i := 0; i < 4096; i++ {
		// Each window from a stretch of its own.
		windows = append(windows, pack(periodic(r, 64, 12, 0.3)[32:])...)
	}
	ls := WordLags(windows, 31)
	if ls[11].Lag != 12 || ls[11].Z < 4 {
		t.Fatalf("lag 12 not seen inside the windows: %+v", ls[11])
	}
	if want := 4096 * (32 - 12); ls[11].N != want {
		t.Fatalf("lag 12 counted %d pairs, want %d, all inside a window", ls[11].N, want)
	}
}

func TestAFairSourcePasses(t *testing.T) {
	r := rand.New(rand.NewSource(458))
	w := make([]uint32, 4096)
	for i := range w {
		w[i] = r.Uint32()
	}
	s := Measure(w)
	if bad := Judge(s); len(bad) != 0 {
		t.Fatalf("a fair source failed: %v", bad)
	}
}

func TestAConstantSourceFails(t *testing.T) {
	w := make([]uint32, 4096)
	for i := range w {
		w[i] = 0x0000ffff
	}
	s := Measure(w)
	if s.Distinct != 1 || len(Judge(s)) == 0 {
		t.Fatalf("a constant source passed: %+v", s)
	}
}

func TestABiasedSourceFails(t *testing.T) {
	// Each bit set with probability 0.52: over 131072 bits that is
	// about 14 standard deviations off.
	r := rand.New(rand.NewSource(1))
	w := make([]uint32, 4096)
	for i := range w {
		for b := 0; b < 32; b++ {
			if r.Float64() < 0.52 {
				w[i] |= 1 << uint(b)
			}
		}
	}
	s := Measure(w)
	if math.Abs(s.Z) < 4 || len(Judge(s)) == 0 {
		t.Fatalf("a biased source passed: z %+.1f", s.Z)
	}
}

// Words fair and uncorrelated with their neighbour but correlated two
// bits on fail at lag 2, which the old check, lag 1 alone, passed: the
// board's words before issue 780's fold were of that shape.
func TestWordsCorrelatedTwoApartFailAtLagTwo(t *testing.T) {
	r := rand.New(rand.NewSource(2))
	w := pack(periodic(r, 32*4096, 2, 0.06))
	s := Measure(w)
	if math.Abs(s.Lags[0].Z) >= 4 {
		t.Fatalf("lag 1 should look clean: z %+.1f", s.Lags[0].Z)
	}
	bad := Judge(s)
	found := false
	for _, b := range bad {
		if strings.Contains(b, "lag 2:") {
			found = true
		}
	}
	if !found {
		t.Fatalf("lag 2 not caught: %v", bad)
	}
}
