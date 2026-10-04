// SPDX-License-Identifier: Apache-2.0
// The throughput run's two modes, issue 1038: `send` and `count`.
package main

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"net"
	"os"
	"sort"
	"syscall"
	"time"
)

func perf(mode string, args []string) {
	if (mode == "send" && len(args) != 3 && len(args) != 4) ||
		(mode == "count" && len(args) != 2) {
		fmt.Fprintln(os.Stderr,
			"usage: ethtest send INTERFACE COUNT FRAME_BYTES [GAP_US] | ethtest count INTERFACE SECONDS")
		os.Exit(2)
	}
	iface, err := net.InterfaceByName(args[0])
	check(err)
	if iface.Flags&net.FlagUp == 0 {
		fmt.Fprintf(os.Stderr, "%s is down: `ip link set %s up` first\n",
			args[0], args[0])
		os.Exit(1)
	}
	fd, err := syscall.Socket(syscall.AF_PACKET, syscall.SOCK_RAW,
		int(htons(perfType)))
	if err != nil {
		fmt.Fprintf(os.Stderr, "the raw socket: %v\n", err)
		fmt.Fprintln(os.Stderr, "this needs root, or cap_net_raw on this binary")
		os.Exit(1)
	}
	defer syscall.Close(fd)
	addr := &syscall.SockaddrLinklayer{
		Protocol: htons(perfType),
		Ifindex:  iface.Index,
		Halen:    6,
	}
	copy(addr.Addr[:], []byte{0xff, 0xff, 0xff, 0xff, 0xff, 0xff})
	check(syscall.Bind(fd, addr))
	if mode == "send" {
		gap := 0
		if len(args) == 4 {
			gap = atoi(args[3])
		}
		send(fd, addr, atoi(args[1]), atoi(args[2]), gap)
	} else {
		count(fd, atoi(args[1]))
	}
}

// send sends `n` frames of `size` bytes, without the check sequence,
// to every station, numbered from zero, one after the other as fast as
// the socket takes them, or `gap` microseconds apart, start to start.
func send(fd int, addr *syscall.SockaddrLinklayer, n, size, gap int) {
	if size < 14+len(perfMagic)+2 {
		size = 14 + len(perfMagic) + 2
	}
	frames := make([][]byte, n)
	for i := range frames {
		f := make([]byte, 0, size)
		f = append(f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff)
		f = append(f, source...)
		f = append(f, byte(perfType>>8), byte(perfType&0xff))
		f = append(f, perfMagic...)
		f = append(f, byte(i>>8), byte(i))
		for j := len(f); j < size; j++ {
			f = append(f, byte(i*31+j))
		}
		frames[i] = f
	}
	start := time.Now()
	for i, f := range frames {
		if gap > 0 {
			// Spun rather than slept, since a sleep this short
			// overshoots by more than the gap.
			at := start.Add(time.Duration(i*gap) * time.Microsecond)
			for time.Now().Before(at) {
			}
		}
		check(syscall.Sendto(fd, f, 0, addr))
	}
	took := time.Since(start)
	fmt.Printf("[ethtest] sent %d frames of %d bytes in %v, %.0f frames/s\n",
		n, size, took, float64(n)/took.Seconds())
}

// count counts the board's frames of this type for `seconds`, by
// length, with the time from the first of each length to its last.
func count(fd, seconds int) {
	tv := syscall.Timeval{Usec: 100000}
	check(syscall.SetsockoptTimeval(fd, syscall.SOL_SOCKET,
		syscall.SO_RCVTIMEO, &tv))
	type tally struct {
		n           int
		first, last time.Time
	}
	by := map[int]*tally{}
	buf := make([]byte, 2048)
	end := time.Now().Add(time.Duration(seconds) * time.Second)
	fmt.Printf("[ethtest] counting type %#04x for %d s\n", perfType, seconds)
	for time.Now().Before(end) {
		n, _, err := syscall.Recvfrom(fd, buf, 0)
		if err != nil {
			continue
		}
		got := buf[:n]
		if n < 14+len(perfMagic) ||
			binary.BigEndian.Uint16(got[12:14]) != perfType ||
			!bytes.Equal(got[14:14+len(perfMagic)], perfMagic) {
			continue
		}
		now := time.Now()
		t := by[n]
		if t == nil {
			t = &tally{first: now}
			by[n] = t
		}
		t.n++
		t.last = now
	}
	lengths := make([]int, 0, len(by))
	for l := range by {
		lengths = append(lengths, l)
	}
	sort.Ints(lengths)
	for _, l := range lengths {
		t := by[l]
		span := t.last.Sub(t.first)
		rate := 0.0
		if t.n > 1 && span > 0 {
			rate = float64(t.n-1) / span.Seconds()
		}
		fmt.Printf("[ethtest] counted %d frames of %d bytes over %v, %.0f frames/s\n",
			t.n, l, span, rate)
	}
	if len(lengths) == 0 {
		fmt.Println("[ethtest] counted no frames")
	}
}
