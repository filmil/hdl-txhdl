// SPDX-License-Identifier: Apache-2.0
// Runs on the machine the board is attached to, uploaded there as one
// static binary. Sends a program to the loader in the board's boot
// memory and then watches the serial port, so that changing the
// software on this machine is a second rather than a place and route.
//
//	load PORT BAUD ADDRESS IMAGE SECONDS
//
// The stream is what `cpu/vreteno/rust/boot.rs` reads: the magic word
// `TXLD`, the address, the length, the words, and their sum, every
// number four bytes least significant first. The loader answers `K`
// when it has taken the header and after every word, and this waits
// for each one, which is what keeps a fast line from overrunning a
// port that buffers eight bytes.
//
// The standard library alone, and the same termios handling the serial
// watcher beside this file uses.
package main

import (
	"encoding/binary"
	"fmt"
	"os"
	"strconv"
	"syscall"
	"time"
	"unsafe"
)

// The flush request, which the syscall package leaves out.
const tcflsh = 0x540B

// What the loader says when it is ready for the next word.
const ack = 'K'

var speeds = map[int]uint32{
	9600:   syscall.B9600,
	19200:  syscall.B19200,
	38400:  syscall.B38400,
	57600:  syscall.B57600,
	115200: syscall.B115200,
	230400: syscall.B230400,
}

func ioctl(fd int, req uint, arg unsafe.Pointer) error {
	_, _, e := syscall.Syscall(syscall.SYS_IOCTL, uintptr(fd), uintptr(req), uintptr(arg))
	if e != 0 {
		return e
	}
	return nil
}

// raw puts the port in the state a byte stream wants: no echo, no
// translation, no flow control, and a read that returns what is there.
func raw(fd int, baud int) error {
	speed, ok := speeds[baud]
	if !ok {
		return fmt.Errorf("no such baud rate: %d", baud)
	}
	var t syscall.Termios
	if err := ioctl(fd, syscall.TCGETS, unsafe.Pointer(&t)); err != nil {
		return err
	}
	t.Iflag = 0
	t.Oflag = 0
	t.Lflag = 0
	t.Cflag = syscall.CS8 | syscall.CREAD | syscall.CLOCAL | speed
	t.Cc[syscall.VMIN] = 0
	t.Cc[syscall.VTIME] = 1
	if err := ioctl(fd, syscall.TCSETS, unsafe.Pointer(&t)); err != nil {
		return err
	}
	return ioctl(fd, tcflsh, unsafe.Pointer(uintptr(2)))
}

func main() {
	if len(os.Args) != 6 {
		fmt.Fprintln(os.Stderr,
			"usage: load PORT BAUD ADDRESS IMAGE SECONDS")
		os.Exit(2)
	}
	port, baudText, addrText, image, secondsText :=
		os.Args[1], os.Args[2], os.Args[3], os.Args[4], os.Args[5]
	baud := atoi(baudText)
	seconds := atoi(secondsText)
	addr64, err := strconv.ParseUint(addrText, 0, 32)
	check(err)
	addr := uint32(addr64)

	blob, err := os.ReadFile(image)
	check(err)
	if len(blob)%4 != 0 {
		blob = append(blob, make([]byte, 4-len(blob)%4)...)
	}

	fd, err := syscall.Open(port, syscall.O_RDWR|syscall.O_NOCTTY, 0)
	check(err)
	defer syscall.Close(fd)
	check(raw(fd, baud))

	// What comes back, printed as it arrives, so a refusal and a
	// program that crashed after loading do not look alike.
	said := make(chan byte, 4096)
	go func() {
		buf := make([]byte, 256)
		for {
			n, err := syscall.Read(fd, buf)
			if err != nil {
				return
			}
			for _, b := range buf[:n] {
				said <- b
			}
		}
	}()

	deadline := time.Now().Add(time.Duration(seconds) * time.Second)
	// The loader says `boot` when it starts and then waits, so on a
	// board that was configured a while ago that word has long gone
	// past. Listen briefly in case it is there, and send anyway: the
	// loader is waiting for the magic word whether or not anybody
	// heard it say so, and the acknowledgements say whether it is
	// listening.
	fmt.Fprintf(os.Stderr, "[load] listening on %s\n", port)
	if waitFor(said, "boot", time.Now().Add(2*time.Second)) {
		fmt.Fprintln(os.Stderr, "[load] the loader is waiting")
	} else {
		fmt.Fprintln(os.Stderr, "[load] no greeting; sending anyway")
	}

	words := len(blob) / 4
	fmt.Fprintf(os.Stderr, "[load] %d words to %#08x\n", words, addr)
	header := make([]byte, 0, 12)
	header = binary.LittleEndian.AppendUint32(header, 0x444c5854)
	header = binary.LittleEndian.AppendUint32(header, addr)
	header = binary.LittleEndian.AppendUint32(header, uint32(len(blob)))
	write(fd, header)

	var sum uint32
	for i := 0; i < words; i++ {
		// One word per acknowledgement: the loader writes each into
		// memory, which takes longer than a word takes to arrive.
		if !waitByte(said, ack, deadline) {
			fmt.Fprintf(os.Stderr, "[load] no acknowledgement at word %d\n", i)
			os.Exit(1)
		}
		word := binary.LittleEndian.Uint32(blob[i*4 : i*4+4])
		sum += word
		write(fd, blob[i*4:i*4+4])
	}
	if !waitByte(said, ack, deadline) {
		fmt.Fprintln(os.Stderr, "[load] no acknowledgement for the last word")
		os.Exit(1)
	}
	write(fd, binary.LittleEndian.AppendUint32(nil, sum))

	// Whatever the loader and then the program have to say.
	for time.Now().Before(deadline) {
		select {
		case b := <-said:
			os.Stdout.Write([]byte{b})
		case <-time.After(200 * time.Millisecond):
		}
	}
	fmt.Fprintln(os.Stderr)
}

// waitFor waits for a word to appear in what the board says.
func waitFor(said chan byte, text string, deadline time.Time) bool {
	seen := make([]byte, 0, 64)
	for time.Now().Before(deadline) {
		select {
		case b := <-said:
			os.Stdout.Write([]byte{b})
			seen = append(seen, b)
			if len(seen) > 64 {
				seen = seen[1:]
			}
			if containsText(seen, text) {
				return true
			}
		case <-time.After(100 * time.Millisecond):
		}
	}
	return false
}

// waitByte waits for one byte, printing anything else that arrives so
// that a refusal is visible rather than silently swallowed.
func waitByte(said chan byte, want byte, deadline time.Time) bool {
	for time.Now().Before(deadline) {
		select {
		case b := <-said:
			if b == want {
				return true
			}
			os.Stdout.Write([]byte{b})
		case <-time.After(100 * time.Millisecond):
		}
	}
	return false
}

func containsText(haystack []byte, needle string) bool {
	n := len(needle)
	for i := 0; i+n <= len(haystack); i++ {
		if string(haystack[i:i+n]) == needle {
			return true
		}
	}
	return false
}

func write(fd int, bytes []byte) {
	for len(bytes) > 0 {
		n, err := syscall.Write(fd, bytes)
		check(err)
		bytes = bytes[n:]
	}
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
