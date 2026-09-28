// SPDX-License-Identifier: Apache-2.0
// Runs on the machine the board is attached to, uploaded there as one
// static binary. Sends a program to the loader in the board's boot
// memory and then watches the serial port, so that changing the
// software on this machine is a second rather than a place and route.
//
//	load PORT BAUD ADDRESS IMAGE SECONDS [reset]
//
// With `reset`, the line is held low first, which the board's top
// turns into a reset of the core, so that a program that has taken the
// core gives it back to the loader without anybody reprogramming the
// part. The low is a zero byte sent at 300 baud: nine bit times of low,
// 30 ms, past the top's 20 ms threshold. It is not a break, because a
// break sent through this board's CP2102N, as Linux's cp210x driver
// drives it, never reached the pin, while the slow zero did; a zero at
// a slow rate is a low any adapter can make. The loader's greeting goes
// out while the port is still at 300 baud and is not readable, so this
// does not wait for it: the acknowledgement of the header says whether
// the loader is there.
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
	"io"
	"os"
	"os/signal"
	"strconv"
	"sync/atomic"
	"syscall"
	"time"
	"unsafe"
)

// The flush request, which the syscall package leaves out.
const tcflsh = 0x540B

// The rate at which one zero byte holds the line low for 30 ms.
const slowBaud = 300

// What the loader says when it is ready for the next word.
const ack = 'K'

// How long the sender waits for each acknowledgement. The loader
// writes a word into memory and answers, which takes a few
// milliseconds; a loader that has stopped answering is known in this
// long, and a long program is not cut short by the seconds meant for
// watching it run afterwards, which once bounded the transfer too and
// stopped a 2496-word image at word 710 (HDL/txhdl#402).
const ackPatience = 2 * time.Second

// How often the transfer says how far it has got, in words, so that a
// transfer stopped from outside is seen to have been moving and where
// (HDL/txhdl#784).
const progressEvery = 4096

var speeds = map[int]uint32{
	300:    syscall.B300,
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
	if len(os.Args) != 6 && len(os.Args) != 7 {
		fmt.Fprintln(os.Stderr,
			"usage: load PORT BAUD ADDRESS IMAGE SECONDS [reset]")
		os.Exit(2)
	}
	reset := len(os.Args) == 7 && os.Args[6] == "reset"
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
	said := listen(fd)

	// The loader says `boot` when it starts and then waits, so on a
	// board that was configured a while ago that word has long gone
	// past. Listen briefly in case it is there, and send anyway: the
	// loader is waiting for the magic word whether or not anybody
	// heard it say so, and the acknowledgements say whether it is
	// listening.
	if reset {
		// The slow zero, then the rate back, then a moment for the
		// core to come out of reset and the loader to start listening.
		fmt.Fprintf(os.Stderr, "[load] holding %s low to reset the core\n", port)
		check(raw(fd, slowBaud))
		write(fd, []byte{0})
		time.Sleep(100 * time.Millisecond)
		check(raw(fd, baud))
		time.Sleep(50 * time.Millisecond)
		// Whatever the greeting became at the wrong rate is not it.
		for len(said) > 0 {
			<-said
		}
	} else {
		fmt.Fprintf(os.Stderr, "[load] listening on %s\n", port)
		if waitFor(said, "boot", time.Now().Add(2*time.Second)) {
			fmt.Fprintln(os.Stderr, "[load] the loader is waiting")
		} else {
			fmt.Fprintln(os.Stderr, "[load] no greeting; sending anyway")
		}
	}

	// A transfer stopped from outside, by the timeout the script on
	// the other end of ssh puts round this, says how far it got and
	// fails, rather than ending with nothing printed (HDL/txhdl#784).
	words := len(blob) / 4
	var sent atomic.Int64
	var done atomic.Bool
	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGTERM, syscall.SIGINT, syscall.SIGHUP)
	go func() {
		s := <-stop
		if done.Load() {
			fmt.Fprintf(os.Stderr, "\n[load] %v after the transfer\n", s)
		} else {
			fmt.Fprintf(os.Stderr, "\n[load] %v: cut off at word %d of %d\n", s, sent.Load(), words)
		}
		os.Exit(1)
	}()
	if err := transfer(fd, said, blob, addr, &sent, os.Stderr); err != nil {
		fmt.Fprintf(os.Stderr, "[load] %v\n", err)
		os.Exit(1)
	}
	done.Store(true)

	// Whatever the loader and then the program have to say: the seconds
	// given are for this, counted from the end of the transfer, so that
	// a long program is watched as long as a short one.
	deadline := time.Now().Add(time.Duration(seconds) * time.Second)
	for time.Now().Before(deadline) {
		select {
		case b := <-said:
			os.Stdout.Write([]byte{b})
		case <-time.After(200 * time.Millisecond):
		}
	}
	fmt.Fprintln(os.Stderr)
}

// listen hands on every byte the port says, as it arrives.
func listen(fd int) chan byte {
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
	return said
}

// transfer sends the header, the words and their sum, one word per
// acknowledgement, and counts in sent the words the loader has taken.
// A loader that stops answering is an error naming the word, and the
// silence it waits through is bounded by ackPatience per word rather
// than by any total, so a long image is never cut short while it moves.
func transfer(fd int, said chan byte, blob []byte, addr uint32, sent *atomic.Int64, log io.Writer) error {
	words := len(blob) / 4
	fmt.Fprintf(log, "[load] %d words to %#08x\n", words, addr)
	header := make([]byte, 0, 12)
	header = binary.LittleEndian.AppendUint32(header, 0x444c5854)
	header = binary.LittleEndian.AppendUint32(header, addr)
	header = binary.LittleEndian.AppendUint32(header, uint32(len(blob)))
	write(fd, header)

	start := time.Now()
	var sum uint32
	for i := 0; i < words; i++ {
		// One word per acknowledgement: the loader writes each into
		// memory, which takes longer than a word takes to arrive.
		if !waitByte(said, ack, time.Now().Add(ackPatience)) {
			return fmt.Errorf("no acknowledgement within %v at word %d of %d", ackPatience, i, words)
		}
		sent.Store(int64(i))
		if i > 0 && i%progressEvery == 0 {
			fmt.Fprintf(log, "[load] %d of %d words, %.0f s\n", i, words, time.Since(start).Seconds())
		}
		word := binary.LittleEndian.Uint32(blob[i*4 : i*4+4])
		sum += word
		write(fd, blob[i*4:i*4+4])
	}
	if !waitByte(said, ack, time.Now().Add(ackPatience)) {
		return fmt.Errorf("no acknowledgement within %v for the last word of %d", ackPatience, words)
	}
	sent.Store(int64(words))
	write(fd, binary.LittleEndian.AppendUint32(nil, sum))
	fmt.Fprintf(log, "[load] %d words sent in %.0f s\n", words, time.Since(start).Seconds())
	return nil
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
