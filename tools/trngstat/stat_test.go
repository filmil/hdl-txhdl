// SPDX-License-Identifier: Apache-2.0

package main

import (
	"math"
	"math/rand"
	"strings"
	"testing"
)

func TestParseSkipsTheGreetingAndReadsBothHalves(t *testing.T) {
	in := "ok 1234abcd\ntrng ok\nraw\r\n0000000f\nFFFFFFFF\nwords\n12345678\nend\n"
	c, err := Parse(strings.NewReader(in))
	if err != nil {
		t.Fatal(err)
	}
	if !c.Ended || c.Fault {
		t.Fatalf("ended %v fault %v", c.Ended, c.Fault)
	}
	if len(c.Raw) != 2 || c.Raw[1] != 0xffffffff || len(c.Words) != 1 {
		t.Fatalf("got %+v", c)
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

func TestAFairSourcePasses(t *testing.T) {
	r := rand.New(rand.NewSource(458))
	w := make([]uint32, 4096)
	for i := range w {
		w[i] = r.Uint32()
	}
	s := Measure(w)
	if bad := Judge(s); len(bad) != 0 {
		t.Fatalf("a fair source failed: %v (%+v)", bad, s)
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
		t.Fatalf("a biased source passed: %+v", s)
	}
}

func TestACorrelatedSourceFails(t *testing.T) {
	// Every bit repeated: fair, and each bit tells the next.
	r := rand.New(rand.NewSource(2))
	w := make([]uint32, 4096)
	for i := range w {
		v := r.Uint32() & 0xffff
		for b := 0; b < 16; b++ {
			bit := v >> uint(b) & 1
			w[i] |= bit<<uint(2*b) | bit<<uint(2*b+1)
		}
	}
	if s := Measure(w); s.Corr < 0.4 || len(Judge(s)) == 0 {
		t.Fatalf("a correlated source passed: %+v", s)
	}
}
