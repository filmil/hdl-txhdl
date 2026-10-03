// SPDX-License-Identifier: Apache-2.0
package main

import (
	"fmt"
	"strings"
	"testing"
)

// record writes one Intel HEX record, with its checksum.
func record(typ byte, addr uint16, data []byte) string {
	rec := append([]byte{byte(len(data)), byte(addr >> 8), byte(addr), typ}, data...)
	var sum byte
	for _, b := range rec {
		sum += b
	}
	rec = append(rec, -sum)
	return ":" + strings.ToUpper(fmt.Sprintf("%x", rec))
}

func TestAProgramAtTheOffsetIsFound(t *testing.T) {
	lines := []string{
		record(0x04, 0, []byte{0x00, 0x00}),
		record(0x00, 0x0000, []byte{0xff, 0xff, 0xaa, 0x99}),
		record(0x04, 0, []byte{0x00, 0xa0}),
		record(0x00, 0x0000, []byte{0x13, 0x00, 0x00, 0x00}),
		record(0x01, 0, nil),
	}
	img, err := ParseMCS([]byte(strings.Join(lines, "\n")))
	if err != nil {
		t.Fatal(err)
	}
	if at, ok := img.Holds([]byte{0x13, 0, 0, 0}, 0xa00000); !ok {
		t.Fatalf("the program differs at 0x%x", at)
	}
	if at, ok := img.Holds([]byte{0x13, 0, 0, 1}, 0xa00000); ok || at != 0xa00003 {
		t.Fatalf("a wrong program passed, or failed at 0x%x", at)
	}
	if hi, ok := img.Below(0xa00000); !ok || hi != 3 {
		t.Fatalf("the bitstream ends at 0x%x, %v", hi, ok)
	}
}

func TestABadChecksumIsRefused(t *testing.T) {
	line := record(0x00, 0, []byte{1, 2})
	bad := line[:len(line)-2] + "00"
	if _, err := ParseMCS([]byte(bad + "\n" + record(0x01, 0, nil))); err == nil {
		t.Fatal("a bad checksum was read")
	}
}

func TestAnImageWithNoEndIsRefused(t *testing.T) {
	if _, err := ParseMCS([]byte(record(0x00, 0, []byte{1}))); err == nil {
		t.Fatal("an image with no end record was read")
	}
}
