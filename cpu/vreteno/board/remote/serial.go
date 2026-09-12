// SPDX-License-Identifier: Apache-2.0
// Runs on the machine the board is attached to, uploaded there as one
// static binary. Reads the serial port for a number of seconds and
// prints what comes, byte for byte as it arrives; once a whole line
// has come, types the reply, if any. The standard library alone: the
// port is set raw at its baud rate through termios.
//
//	serial PORT BAUD SECONDS [REPLY]
package main

import (
	"bytes"
	"fmt"
	"os"
	"strconv"
	"syscall"
	"time"
	"unsafe"
)

// The flush request, which the syscall package leaves out.
const tcflsh = 0x540B

// The baud rates termios knows by name.
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

func fail(err error) {
	fmt.Fprintln(os.Stderr, "[serial]", err)
	os.Exit(1)
}

func main() {
	if len(os.Args) < 4 {
		fail(fmt.Errorf("usage: serial PORT BAUD SECONDS [REPLY]"))
	}
	port := os.Args[1]
	baud, err := strconv.Atoi(os.Args[2])
	if err != nil {
		fail(err)
	}
	speed, ok := speeds[baud]
	if !ok {
		fail(fmt.Errorf("no such baud rate: %d", baud))
	}
	seconds, err := strconv.ParseFloat(os.Args[3], 64)
	if err != nil {
		fail(err)
	}
	reply := ""
	if len(os.Args) > 4 {
		reply = os.Args[4]
	}

	fd, err := syscall.Open(port, syscall.O_RDWR|syscall.O_NOCTTY|syscall.O_NONBLOCK, 0)
	if err != nil {
		fail(fmt.Errorf("open %s: %w", port, err))
	}
	defer syscall.Close(fd)
	// Raw, eight bits, no flow control, at the rate; a read returns
	// what is there after a tenth of a second at most, so the loop
	// keeps to its deadline.
	var t syscall.Termios
	if err := ioctl(fd, syscall.TCGETS, unsafe.Pointer(&t)); err != nil {
		fail(fmt.Errorf("termios of %s: %w", port, err))
	}
	t.Iflag = 0
	t.Oflag = 0
	t.Lflag = 0
	t.Cflag = syscall.CS8 | syscall.CREAD | syscall.CLOCAL | speed
	t.Ispeed = speed
	t.Ospeed = speed
	t.Cc[syscall.VMIN] = 0
	t.Cc[syscall.VTIME] = 1
	if err := ioctl(fd, syscall.TCSETS, unsafe.Pointer(&t)); err != nil {
		fail(fmt.Errorf("set %s: %w", port, err))
	}
	flush := uintptr(syscall.TCIFLUSH)
	_ = ioctl(fd, tcflsh, unsafe.Pointer(&flush))
	if err := syscall.SetNonblock(fd, false); err != nil {
		fail(err)
	}

	deadline := time.Now().Add(time.Duration(seconds * float64(time.Second)))
	var seen []byte
	replied := reply == ""
	buf := make([]byte, 256)
	for time.Now().Before(deadline) {
		n, err := syscall.Read(fd, buf)
		if err != nil && err != syscall.EINTR {
			fail(fmt.Errorf("read %s: %w", port, err))
		}
		if n > 0 {
			os.Stdout.Write(buf[:n])
			seen = append(seen, buf[:n]...)
		}
		if !replied && bytes.IndexByte(seen, '\n') >= 0 {
			if _, err := syscall.Write(fd, []byte(reply)); err != nil {
				fail(fmt.Errorf("write %s: %w", port, err))
			}
			replied = true
		}
	}
	fmt.Printf("\n[serial] %d bytes in %.0f s\n", len(seen), seconds)
}
