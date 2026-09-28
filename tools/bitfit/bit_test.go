// SPDX-License-Identifier: Apache-2.0

package main

import (
	"encoding/binary"
	"strings"
	"testing"
)

// bitFile makes a `.bit` file as Vivado writes one, with `n` bytes of
// data.
func bitFile(n int) []byte {
	b := append([]byte{}, magic...)
	for i, s := range []string{"top;UserID=0XFFFFFFFF", "7a200tfbg484", "2026/09/28", "16:20:25"} {
		b = append(b, byte('a'+i))
		b = binary.BigEndian.AppendUint16(b, uint16(len(s)+1))
		b = append(b, s...)
		b = append(b, 0)
	}
	b = append(b, 'e')
	b = binary.BigEndian.AppendUint32(b, uint32(n))
	return append(b, make([]byte, n)...)
}

func TestParseReadsTheHeader(t *testing.T) {
	bit, err := Parse(bitFile(1000))
	if err != nil {
		t.Fatal(err)
	}
	if bit.DataLen != 1000 || bit.Part != "7a200tfbg484" || bit.Date != "2026/09/28" {
		t.Errorf("got %+v", bit)
	}
	if bit.HeaderLen+bit.DataLen != len(bitFile(1000)) {
		t.Errorf("header %d and data %d are not the file", bit.HeaderLen, bit.DataLen)
	}
}

func TestParseRefusesATruncatedFile(t *testing.T) {
	b := bitFile(1000)
	if _, err := Parse(b[:len(b)-1]); err == nil || !strings.Contains(err.Error(), "the file holds 999") {
		t.Errorf("a short file: %v", err)
	}
}

func TestParseRefusesSomethingElse(t *testing.T) {
	if _, err := Parse([]byte("hello, not a bitstream")); err == nil {
		t.Error("a text file parsed")
	}
	b := bitFile(10)
	b[len(magic)] = 'x'
	if _, err := Parse(b); err == nil || !strings.Contains(err.Error(), "field 'a'") {
		t.Errorf("a missing field: %v", err)
	}
}
