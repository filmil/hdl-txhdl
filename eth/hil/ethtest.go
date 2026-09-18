// SPDX-License-Identifier: Apache-2.0
// Runs on the machine the board's Ethernet port is cabled to, uploaded
// there as one static binary. Sends frames to the board and reads back
// what the echo design returns.
//
//	ethtest INTERFACE COUNT PAYLOAD_BYTES SECONDS
//
// A frame goes out with a fixed source address of its own, the adapter's
// address as its destination, a type nothing else uses, and a payload
// that says which frame it is. The echo design returns every frame whose
// check sequence is correct, unchanged, so a frame that comes back is
// the same frame and the adapter accepts it because it is addressed to
// it.
//
// What is checked:
//
//   - every frame sent comes back, once;
//   - what comes back is what went out, byte for byte, up to the padding;
//   - a frame shorter than the minimum comes back padded to sixty bytes,
//     which is the transmitter doing its job.
//
// The standard library alone, and a raw socket, which needs either root
// or `cap_net_raw` on this binary.
package main

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"net"
	"os"
	"strconv"
	"syscall"
	"time"
)

// The type field. 0x88b5 is reserved for local experiments, so nothing
// else on a cable answers it.
const ethType = 0x88b5

// The first bytes of every payload, so that a frame of this test is not
// mistaken for anything else on the wire.
var magic = []byte("TXHDL-ETH-HIL\x00")

// The source address the test sends from: locally administered, and not
// the adapter's own, so that a frame coming back is plainly a frame that
// went out and returned.
var source = []byte{0x02, 0x00, 0x54, 0x58, 0x48, 0x4c}

func htons(v uint16) uint16 {
	return v<<8 | v>>8
}

func main() {
	if len(os.Args) != 5 {
		fmt.Fprintln(os.Stderr,
			"usage: ethtest INTERFACE COUNT PAYLOAD_BYTES SECONDS")
		os.Exit(2)
	}
	name := os.Args[1]
	count := atoi(os.Args[2])
	size := atoi(os.Args[3])
	seconds := atoi(os.Args[4])

	iface, err := net.InterfaceByName(name)
	check(err)
	if iface.Flags&net.FlagUp == 0 {
		fmt.Fprintf(os.Stderr, "%s is down: `ip link set %s up` first\n",
			name, name)
		os.Exit(1)
	}

	fd, err := syscall.Socket(syscall.AF_PACKET, syscall.SOCK_RAW,
		int(htons(ethType)))
	if err != nil {
		fmt.Fprintf(os.Stderr, "the raw socket: %v\n", err)
		fmt.Fprintln(os.Stderr,
			"this needs root, or cap_net_raw on this binary")
		os.Exit(1)
	}
	defer syscall.Close(fd)

	addr := &syscall.SockaddrLinklayer{
		Protocol: htons(ethType),
		Ifindex:  iface.Index,
		Halen:    6,
	}
	copy(addr.Addr[:], iface.HardwareAddr)
	check(syscall.Bind(fd, addr))

	tv := syscall.Timeval{Sec: int64(seconds)}
	check(syscall.SetsockoptTimeval(fd, syscall.SOL_SOCKET,
		syscall.SO_RCVTIMEO, &tv))

	fmt.Printf("[ethtest] %s, %s, sending %d frames of %d payload bytes\n",
		name, iface.HardwareAddr, count, size)

	// One frame at a time. The receiver offers a frame to its client
	// and drops what arrives while that frame is still being offered,
	// so a burst loses everything after the first. The design says so,
	// and the board proved it: sixteen frames sent at once came back as
	// one. So the test sends a frame, waits for it, and only then sends
	// the next.
	back := make([]int, count)
	padded, wrong, late := 0, 0, 0
	buf := make([]byte, 2048)
	for i := 0; i < count; i++ {
		out := build(iface.HardwareAddr, i, size)
		check(syscall.Sendto(fd, out, 0, addr))
		deadline := time.Now().Add(time.Duration(seconds) * time.Second)
		for time.Now().Before(deadline) {
			n, _, err := syscall.Recvfrom(fd, buf, 0)
			if err != nil {
				break
			}
			got := buf[:n]
			seq, ok := mine(got, count)
			if !ok {
				continue
			}
			if seq != i {
				late++
				continue
			}
			back[seq]++
			switch {
			case n == len(out) && bytes.Equal(got, out):
				// The frame came back as it went out.
			case n == 60 && len(out) < 60 && bytes.Equal(got[:len(out)], out):
				// The transmitter padded a short frame to the minimum.
				padded++
			default:
				wrong++
				fmt.Printf("[ethtest] frame %d came back as %d bytes, sent %d\n",
					seq, n, len(out))
			}
			break
		}
	}

	missing, twice := 0, 0
	for _, b := range back {
		switch {
		case b == 0:
			missing++
		case b > 1:
			twice++
		}
	}
	fmt.Printf("[ethtest] back %d of %d, padded %d, wrong %d, twice %d, late %d\n",
		count-missing, count, padded, wrong, twice, late)
	if missing != 0 || wrong != 0 || twice != 0 {
		fmt.Println("[ethtest] FAILED")
		os.Exit(1)
	}
	fmt.Println("[ethtest] every frame came back, unchanged")
}

// mine says whether a frame is one of this test's, and which one.
func mine(got []byte, count int) (int, bool) {
	if len(got) < 14+len(magic)+2 {
		return 0, false
	}
	if !bytes.Equal(got[6:12], source) ||
		binary.BigEndian.Uint16(got[12:14]) != ethType ||
		!bytes.Equal(got[14:14+len(magic)], magic) {
		return 0, false
	}
	seq := int(binary.BigEndian.Uint16(got[14+len(magic) : 16+len(magic)]))
	if seq < 0 || seq >= count {
		return 0, false
	}
	return seq, true
}

// build makes one frame: the adapter's address, this test's own source,
// the type, the magic, the frame's number, and a pattern that differs
// per frame so that two frames cannot be confused.
func build(dst net.HardwareAddr, seq, size int) []byte {
	frame := make([]byte, 0, 14+size)
	frame = append(frame, dst...)
	frame = append(frame, source...)
	frame = append(frame, byte(ethType>>8), byte(ethType&0xff))
	frame = append(frame, magic...)
	frame = append(frame, byte(seq>>8), byte(seq))
	for i := len(frame); i < 14+size; i++ {
		frame = append(frame, byte(seq*31+i))
	}
	return frame
}

func atoi(s string) int {
	n, err := strconv.Atoi(s)
	check(err)
	return n
}

func check(err error) {
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
