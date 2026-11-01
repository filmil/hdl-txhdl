// SPDX-License-Identifier: Apache-2.0
// The sender against a loader played by the test, on a pseudo-terminal
// rather than the board's serial port, so that the transfer is checked
// without a board (HDL/txhdl#784).
package main

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
	"unsafe"
)

// The test binary is also the sender, when it is asked to be, so that
// a test can stop a whole sender from outside as a timeout does.
func TestMain(m *testing.M) {
	if args := os.Getenv("LOAD_AS_SENDER"); args != "" {
		os.Args = append([]string{"load"}, strings.Split(args, " ")...)
		main()
		os.Exit(0)
	}
	os.Exit(m.Run())
}

// pty opens a pseudo-terminal: the loader's end and the sender's.
func pty(t *testing.T) (*os.File, int) {
	m, fd, _ := ptyNamed(t)
	return m, fd
}

// ptyNamed is pty, and the name of the sender's end.
func ptyNamed(t *testing.T) (*os.File, int, string) {
	t.Helper()
	m, err := os.OpenFile("/dev/ptmx", os.O_RDWR|syscall.O_NOCTTY, 0)
	if err != nil {
		t.Skipf("no pseudo-terminal here: %v", err)
	}
	var unlock int32
	if err := ioctl(int(m.Fd()), syscall.TIOCSPTLCK, unsafe.Pointer(&unlock)); err != nil {
		t.Fatal(err)
	}
	var n uint32
	if err := ioctl(int(m.Fd()), syscall.TIOCGPTN, unsafe.Pointer(&n)); err != nil {
		t.Fatal(err)
	}
	name := fmt.Sprintf("/dev/pts/%d", n)
	fd, err := syscall.Open(name, syscall.O_RDWR|syscall.O_NOCTTY, 0)
	if err != nil {
		t.Fatal(err)
	}
	if err := raw(fd, 115200); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { m.Close(); syscall.Close(fd) })
	return m, fd, name
}

// loader plays boot.rs: the header, then a word per acknowledgement,
// then the sum. It answers the first `answers` times and then goes
// quiet, as a loader that has stopped does. What it took comes back on
// the channel.
func loader(m *os.File, answers int) chan []uint32 {
	return slowLoader(m, answers, 0)
}

// slowLoader is loader, taking `pause` over each word.
func slowLoader(m *os.File, answers int, pause time.Duration) chan []uint32 {
	got := make(chan []uint32, 1)
	go func() {
		var header [12]byte
		if _, err := io.ReadFull(m, header[:]); err != nil {
			got <- nil
			return
		}
		n := int(binary.LittleEndian.Uint32(header[8:])) / 4
		var words []uint32
		for i := 0; i <= n; i++ {
			if answers == 0 {
				got <- words
				return
			}
			answers--
			time.Sleep(pause)
			m.Write([]byte{ack})
			var w [4]byte
			if _, err := io.ReadFull(m, w[:]); err != nil {
				got <- words
				return
			}
			words = append(words, binary.LittleEndian.Uint32(w[:]))
		}
		got <- words
	}()
	return got
}

func image(words int) []byte {
	blob := make([]byte, 0, 4*words)
	for i := 0; i < words; i++ {
		blob = binary.LittleEndian.AppendUint32(blob, uint32(i*2654435761))
	}
	return blob
}

// A large image goes through whole, however long it takes, and says
// how far it has got on the way.
func TestALargeImageGoesThroughWhole(t *testing.T) {
	m, fd := pty(t)
	const words = 9000
	blob := image(words)
	got := loader(m, words+1)
	var sent atomic.Int64
	var log bytes.Buffer
	if err := transfer(fd, listen(fd), blob, 0x40000000, &sent, &log); err != nil {
		t.Fatalf("%v\n%s", err, log.String())
	}
	took := <-got
	if len(took) != words+1 {
		t.Fatalf("the loader took %d words and a sum, want %d", len(took), words+1)
	}
	var sum uint32
	for i := 0; i < words; i++ {
		w := binary.LittleEndian.Uint32(blob[4*i:])
		if took[i] != w {
			t.Fatalf("word %d: %#x, want %#x", i, took[i], w)
		}
		sum += w
	}
	if took[words] != sum {
		t.Fatalf("sum %#x, want %#x", took[words], sum)
	}
	if sent.Load() != words {
		t.Fatalf("counted %d words sent, want %d", sent.Load(), words)
	}
	for _, want := range []string{"4096 of 9000 words", "8192 of 9000 words", "9000 words sent"} {
		if !strings.Contains(log.String(), want) {
			t.Errorf("no %q in what it said:\n%s", want, log.String())
		}
	}
}

// A loader that stops answering stops the transfer with an error that
// names the word, rather than nothing.
func TestALoaderThatStopsIsNamed(t *testing.T) {
	m, fd := pty(t)
	loader(m, 101)
	var sent atomic.Int64
	var log bytes.Buffer
	err := transfer(fd, listen(fd), image(500), 0x40000000, &sent, &log)
	if err == nil || !strings.Contains(err.Error(), "at word 101 of 500") {
		t.Fatalf("error %v, want one at word 101 of 500", err)
	}
	if sent.Load() != 100 {
		t.Fatalf("counted %d words taken, want 100", sent.Load())
	}
}

// A sender stopped from outside partway, as the timeout round it stops
// it, says at which word and fails, where it used to end with nothing
// printed after its first line and leave a half-loaded image looking
// like a quiet program. It says so on stdout, where the console goes,
// and nothing of the loader's is left on stderr: behind ssh the two
// streams travel apart, and a `[load]` line on stderr can land in the
// middle of a console line (HDL/txhdl#2045).
func TestASenderCutOffSaysWhere(t *testing.T) {
	m, _, name := ptyNamed(t)
	img := filepath.Join(t.TempDir(), "image.bin")
	if err := os.WriteFile(img, image(2000), 0o644); err != nil {
		t.Fatal(err)
	}
	slowLoader(m, 2001, 2*time.Millisecond)
	cmd := exec.Command(os.Args[0])
	cmd.Env = append(os.Environ(),
		"LOAD_AS_SENDER="+strings.Join([]string{name, "115200", "0x40000000", img, "1"}, " "))
	var out, errs bytes.Buffer
	cmd.Stdout = &out
	cmd.Stderr = &errs
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	// The greeting is waited for for two seconds, then the words go.
	time.Sleep(2500 * time.Millisecond)
	cmd.Process.Signal(syscall.SIGTERM)
	err := cmd.Wait()
	code := -1
	if e, ok := err.(*exec.ExitError); ok {
		code = e.ExitCode()
	}
	if code != 1 {
		t.Fatalf("exit %v (%d), want 1\n%s%s", err, code, out.String(), errs.String())
	}
	said := out.String()
	if !strings.Contains(said, "terminated: cut off at word ") || !strings.Contains(said, " of 2000") {
		t.Fatalf("no word named where it was cut off on stdout:\n%s", said)
	}
	for _, want := range []string{"[load] listening on", "[load] 2000 words to"} {
		if !strings.Contains(said, want) {
			t.Fatalf("no %q on stdout:\n%s", want, said)
		}
	}
	if strings.Contains(errs.String(), "[load]") {
		t.Fatalf("a [load] line on stderr, apart from the console:\n%s", errs.String())
	}
}

// With `stamp`, every line the sender prints starts with the board
// server's UTC time, the form the nightly reads, so the nightly times
// the board and not the network to it (HDL/txhdl#2046). The stamp is
// taken at a line's first byte, and the cut-off line is stamped too.
func TestStampedLinesStartWithTheServersTime(t *testing.T) {
	m, _, name := ptyNamed(t)
	img := filepath.Join(t.TempDir(), "image.bin")
	if err := os.WriteFile(img, image(2000), 0o644); err != nil {
		t.Fatal(err)
	}
	slowLoader(m, 2001, 2*time.Millisecond)
	cmd := exec.Command(os.Args[0])
	cmd.Env = append(os.Environ(),
		"LOAD_AS_SENDER="+strings.Join([]string{name, "115200", "0x40000000", img, "1", "stamp"}, " "))
	var out, errs bytes.Buffer
	cmd.Stdout = &out
	cmd.Stderr = &errs
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	time.Sleep(2500 * time.Millisecond)
	cmd.Process.Signal(syscall.SIGTERM)
	cmd.Wait()
	if errs.Len() != 0 {
		t.Fatalf("stderr is not empty, so not all of it is stamped:\n%s", errs.String())
	}
	// A stamp and something after it: an empty stamped line is a line
	// the console made up.
	stamped := regexp.MustCompile(`^\d\d:\d\d:\d\d\.\d\d\dZ \S`)
	lines := strings.Split(strings.TrimRight(out.String(), "\n"), "\n")
	if len(lines) < 3 {
		t.Fatalf("too few lines:\n%s", out.String())
	}
	for _, l := range lines {
		if !stamped.MatchString(l) {
			t.Fatalf("a line without the server's stamp: %q\n%s", l, out.String())
		}
	}
	for _, want := range []string{"Z [load] 2000 words to", "Z [load] terminated: cut off at word "} {
		if !strings.Contains(out.String(), want) {
			t.Fatalf("no stamped %q:\n%s", want, out.String())
		}
	}
}

// A console stamps each line once, with the time of its first byte,
// however the line's bytes are split across writes; the clock moves on
// a millisecond at each write, so a stamp taken later would show. A
// `[load]` line said partway through a line of the board's starts on a
// line of its own, and ending a line at the start of one adds nothing.
func TestAConsoleStampsEachLineAtItsFirstByte(t *testing.T) {
	var b bytes.Buffer
	at := time.Date(2026, 11, 1, 12, 9, 20, 848_000_000, time.UTC)
	c := &console{w: &b, stamp: true, clock: func() time.Time { return at }}
	for _, p := range []string{"ab", "c\nd", "e\n", "f"} {
		c.Write([]byte(p))
		at = at.Add(time.Millisecond)
	}
	say(c, "[load] cut off\n")
	c.endLine()
	want := "12:09:20.848Z abc\n" +
		"12:09:20.849Z de\n" +
		"12:09:20.851Z f\n" +
		"12:09:20.852Z [load] cut off\n"
	if b.String() != want {
		t.Fatalf("got %q, want %q", b.String(), want)
	}
}
