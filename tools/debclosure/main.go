// SPDX-License-Identifier: Apache-2.0
//
// The Debian dependency closure of a set of packages, as a lock file.
//
// OpenROAD and Yosys are published as Debian packages, and a Debian
// package names the shared libraries it needs but does not carry them.
// This reads a suite's package index, walks the closure from the
// packages named on the command line, and writes one line per package
// with the version, the checksum and two URLs to fetch it from. The
// build then fetches exactly those bytes and unpacks them into a tree,
// so the two tools are pinned the way every other tool in this
// repository is pinned.
//
// Run by hand when the pin is to move, and commit what it writes:
//
//	bazel run //tools/debclosure -- -suite=trixie \
//	    -skip=libc6,libgcc-s1 yosys > third_party/yosys/trixie.lock.tsv
//
// The first URL is the live archive, which is fast and which forgets a
// package as soon as a newer one replaces it. The second is
// snapshot.debian.org, which is slow and never forgets. A fetcher that
// tries them in order is fast while the pin is current and still
// correct years later.
package main

import (
	"bufio"
	"compress/gzip"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"sort"
	"strings"
)

// One package as the index states it: what it is called, which version
// this is, where in the pool it lies, and the checksum of those bytes.
type pkg struct {
	name     string
	version  string
	filename string
	sha256   string
	size     string
	depends  []string
	provides []string
}

const (
	live     = "https://deb.debian.org/debian/"
	snapshot = "https://snapshot.debian.org/archive/debian/"
)

func main() {
	suite := flag.String("suite", "trixie", "the Debian suite to read")
	stamp := flag.String("snapshot", "20250901T000000Z",
		"the snapshot.debian.org timestamp of the fallback URL")
	skip := flag.String("skip", "",
		"packages to leave out of the closure, comma separated")
	flag.Parse()
	if flag.NArg() == 0 {
		fmt.Fprintln(os.Stderr, "usage: debclosure [flags] package...")
		os.Exit(2)
	}

	index, err := fetchIndex(*suite)
	if err != nil {
		fmt.Fprintln(os.Stderr, "reading the index:", err)
		os.Exit(1)
	}
	byName, provided := parse(index)

	left := map[string]bool{}
	for _, s := range strings.Split(*skip, ",") {
		if s != "" {
			left[s] = true
		}
	}

	// The closure, breadth first from the packages named, following
	// Depends and taking the first alternative of a choice, which is
	// the one Debian itself prefers. A dependency may name a virtual
	// package, so what is found is filed under the name of the real
	// package that answered it, and a package reached twice under two
	// names is one package.
	found := map[string]*pkg{}
	missing := map[string]bool{}
	queue := append([]string{}, flag.Args()...)
	for len(queue) > 0 {
		name := queue[0]
		queue = queue[1:]
		if left[name] {
			continue
		}
		p := byName[name]
		if p == nil {
			p = provided[name]
		}
		if p == nil {
			missing[name] = true
			continue
		}
		if left[p.name] || found[p.name] != nil {
			continue
		}
		found[p.name] = p
		queue = append(queue, p.depends...)
	}

	names := make([]string, 0, len(found))
	for n := range found {
		names = append(names, n)
	}
	sort.Strings(names)

	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	fmt.Fprintf(w, "# suite\t%s\n", *suite)
	fmt.Fprintf(w, "# snapshot\t%s\n", *stamp)
	fmt.Fprintf(w, "# name\tversion\tsize\tsha256\turl\tfallback\n")
	for _, n := range names {
		p := found[n]
		fmt.Fprintf(w, "%s\t%s\t%s\t%s\t%s%s\t%s%s/%s\n",
			p.name, p.version, p.size, p.sha256,
			live, p.filename, snapshot, *stamp, p.filename)
	}

	for n := range missing {
		fmt.Fprintln(os.Stderr, "not in the index, left out:", n)
	}
}

// The suite's binary package index for amd64, decompressed.
func fetchIndex(suite string) (io.Reader, error) {
	url := live + "dists/" + suite + "/main/binary-amd64/Packages.gz"
	resp, err := http.Get(url)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("%s: %s", url, resp.Status)
	}
	gz, err := gzip.NewReader(resp.Body)
	if err != nil {
		return nil, err
	}
	all, err := io.ReadAll(gz)
	if err != nil {
		return nil, err
	}
	return strings.NewReader(string(all)), nil
}

// The index as two maps: packages by their own name, and packages by
// each virtual name they provide. The first entry for a name wins,
// which is the one the index lists first.
func parse(r io.Reader) (map[string]*pkg, map[string]*pkg) {
	byName := map[string]*pkg{}
	provided := map[string]*pkg{}
	sc := bufio.NewScanner(r)
	sc.Buffer(make([]byte, 1<<20), 1<<20)
	cur := &pkg{}
	flush := func() {
		if cur.name != "" {
			if _, seen := byName[cur.name]; !seen {
				byName[cur.name] = cur
			}
			for _, v := range cur.provides {
				if _, seen := provided[v]; !seen {
					provided[v] = byName[cur.name]
				}
			}
		}
		cur = &pkg{}
	}
	for sc.Scan() {
		line := sc.Text()
		if line == "" {
			flush()
			continue
		}
		key, value, ok := strings.Cut(line, ": ")
		if !ok {
			continue
		}
		switch key {
		case "Package":
			cur.name = value
		case "Version":
			cur.version = value
		case "Filename":
			cur.filename = value
		case "SHA256":
			cur.sha256 = value
		case "Size":
			cur.size = value
		case "Depends":
			cur.depends = relations(value)
		case "Provides":
			cur.provides = relations(value)
		}
	}
	flush()
	return byName, provided
}

// The package names in a relation field, with the version constraints
// dropped and the first alternative of each choice taken.
func relations(s string) []string {
	var out []string
	for _, term := range strings.Split(s, ",") {
		first := strings.TrimSpace(strings.Split(term, "|")[0])
		if name, _, ok := strings.Cut(first, " "); ok {
			first = name
		}
		first = strings.TrimSpace(first)
		// A dependency written "python3:any" names an architecture
		// qualifier that the index does not repeat on the package
		// itself.
		if name, _, ok := strings.Cut(first, ":"); ok {
			first = name
		}
		if first != "" {
			out = append(out, first)
		}
	}
	return out
}
