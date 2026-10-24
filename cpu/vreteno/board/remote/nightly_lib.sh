# SPDX-License-Identifier: Apache-2.0
# The nightly's readers (issue 1854): each takes a log the run kept,
# every line stamped `HH:MM:SS.mmmZ` as the board said it, and prints
# rows of results.tsv, tab-separated:
#
#   check  metric  value  unit  status
#
# status is pass, fail or skipped. A check that finds nothing to read
# says so as a fail rather than printing nothing, since a row that is
# not there is not noticed. Sourced by nightly.sh and by its test.

# The seconds between two stamps, the second after the first; across
# midnight, a day is added.
_secs='sub ts { my ($h, $m, $x) = $_[0] =~ /^(\d\d):(\d\d):(\d\d\.\d+)Z/ or return; $h*3600 + $m*60 + $x }
sub gap { my $g = $_[1] - $_[0]; $g += 86400 if $g < 0; $g }'

row() { printf '%s\t%s\t%s\t%s\t%s\n' "$@"; }

# Linux through fastboot: `booting` to OpenSBI's banner (#1457's gap,
# which passes under 2 s), and `booting` to `Run /init` (recorded).
read_linux() {
  local check="$1" log="$2"
  perl -e "$_secs"'
    my ($c) = @ARGV; my ($b, $o, $i);
    open my $f, "<", $ARGV[1] or die;
    while (<$f>) {
      $b //= ts($_) if /fastboot: booting/;
      $o //= ts($_) if defined $b && /OpenSBI v/;
      $i //= ts($_) if defined $b && /Run \/init as init process/;
    }
    if (!defined $b) { print "$c\tbooting\t-\t-\tfail\n"; exit }
    printf "$c\tbooting to OpenSBI\t%s\ts\t%s\n", defined $o ? sprintf("%.2f", gap($b, $o)) : "-",
      defined $o && gap($b, $o) < 2 ? "pass" : "fail";
    printf "$c\tbooting to /init\t%s\ts\t%s\n", defined $i ? sprintf("%.2f", gap($b, $i)) : "-",
      defined $i ? "pass" : "fail";
  ' "$check" "$log"
}

# A demo: its last line, as demos.sh keeps it in manifest.tsv, and its
# own `demo ... tx_dropped N` line where it prints one, which is 0.
read_demo() {
  local name="$1" log="$2" last="$3"
  if [[ -z "$last" ]]; then
    row "demo $name" ran - - fail
  else
    row "demo $name" ran yes - pass
  fi
  local d
  d="$(grep -aoE 'tx_dropped [0-9]+' "$log" 2>/dev/null | tail -1 | cut -d' ' -f2 || true)"
  if [[ -n "$d" ]]; then
    row "demo $name" tx_dropped "$d" frames "$([[ "$d" == 0 ]] && echo pass || echo fail)"
  fi
}

# Runs a binary Bazel built, as it is and outside this script's own
# runfiles. The nightly runs under `bazel run`, which exports
# RUNFILES_DIR, RUNFILES_MANIFEST_FILE and JAVA_RUNFILES for the
# nightly, and runfiles.bash reads all three; a wrapper
# started with them looks for its own files in the nightly's runfiles,
# finds none, and goes on with an empty path, as //eth/hil:run did with
# its tester (#1898). Without them it finds its own, beside itself.
bare() {
  env -u RUNFILES_DIR -u RUNFILES_MANIFEST_FILE -u JAVA_RUNFILES "$@"
}

# The PHY's registers as phyregs prints them, `rNN a b c d`: the link up
# (r1 bit 2, from `r01 again` where the log has it: the bit latches
# low, so the dump's read is low if the link dropped at any time since
# the last read, as it does while the part is programmed, #1897),
# and a 1000BASE-T full-duplex link: both sides advertise
# 1000 full (r9 bit 9, r10 bit 11) and both receivers are OK (r10 bits
# 13 and 12).
read_phy() {
  local log="$1"
  perl -e '
    my (%r, $again);
    open my $f, "<", $ARGV[0] or die;
    while (<$f>) {
      $again = hex $1 if /\br01 again ([0-9a-f]{4})/;
      next unless /\br(\d\d) ((?:[0-9a-f]{4} ?){4})/;
      my @w = split " ", $2;
      $r{$1 + $_} = hex $w[$_] for 0 .. 3;
    }
    if (!defined $r{1} || !defined $r{10}) { print "ethernet\tphy registers\t-\t-\tfail\n"; exit }
    my $up = ((defined $again ? $again : $r{1}) >> 2) & 1;
    my $g = ($r{9} >> 9 & 1) && ($r{10} >> 11 & 1) && ($r{10} >> 13 & 1) && ($r{10} >> 12 & 1);
    printf "ethernet\tlink\t%s\t-\t%s\n", $up ? "up" : "down", $up ? "pass" : "fail";
    printf "ethernet\t1000 full\t%s\t-\t%s\n", $g ? "yes" : "no", $g ? "pass" : "fail";
  ' "$log"
}

# ping's summary: the packets lost, which are none.
read_ping() {
  local log="$1"
  perl -e '
    open my $f, "<", $ARGV[0] or die;
    while (<$f>) {
      if (/(\d+) packets transmitted, (\d+) (?:packets )?received/) {
        my $lost = $1 - $2;
        printf "ethernet\tping lost\t%d\tof %d\t%s\n", $lost, $1, $lost == 0 && $1 > 0 ? "pass" : "fail";
        exit;
      }
    }
    print "ethernet\tping lost\t-\t-\tfail\n";
  ' "$log"
}

# ethperf: each transmit run's frames a second, recorded, and each
# receive run's frames got of those sent, which are all of them.
read_ethperf() {
  local log="$1"
  perl -e '
    my $n = 0;
    open my $f, "<", $ARGV[0] or die;
    while (<$f>) {
      if (/\btx (dma|copy) (\d+) frames \d+ ticks \d+ frames\/s (\d+)/) {
        print "ethernet\ttx $1 $2\t$3\tframes/s\tpass\n"; $n++;
      } elsif (/\brx (dma|copy) (\d+) got (\d+) of (\d+)/) {
        printf "ethernet\trx %s %s got\t%d\tof %d\t%s\n", $1, $2, $3, $4, $4 > 0 && $3 == $4 ? "pass" : "fail"; $n++;
      }
    }
    print "ethernet\tethperf\t-\t-\tfail\n" unless $n;
  ' "$log"
}

# The fastboot app: the seconds from its load to `listening on port`,
# and whether the host's getvar was answered.
read_fastboot() {
  local log="$1" answered="$2"
  perl -e "$_secs"'
    my ($s, $l);
    open my $f, "<", $ARGV[0] or die;
    while (<$f>) {
      $s //= ts($_) if /\bok 40000000\b/ || /\[load\] \d+ words to/;
      $l //= ts($_) if /fastboot: listening on port/;
    }
    printf "fastboot\tload to listening\t%s\ts\t%s\n",
      defined $s && defined $l ? sprintf("%.2f", gap($s, $l)) : "-",
      defined $s && defined $l ? "pass" : "fail";
  ' "$log"
  row fastboot "getvar answered" "$answered" - "$([[ "$answered" == yes ]] && echo pass || echo fail)"
}

# The costs image: each `costs NAME N`, and each figure of an `icache`
# line, `icache cold N warm N ..`, recorded as it is.
read_costs() {
  local log="$1"
  perl -e '
    my $n = 0;
    open my $f, "<", $ARGV[0] or die;
    while (<$f>) {
      s/^\S+Z //;
      if (/^costs (\S+) (\d+)\s*$/) { print "costs\t$1\t$2\tcycles\tpass\n"; $n++; next }
      if (/^(icache\S*)((?: \S+ \d+)+)\s*$/) {
        my ($k, $rest) = ($1, $2);
        while ($rest =~ / (\S+) (\d+)/g) { print "costs\t$k $1\t$2\tcycles\tpass\n"; $n++ }
      }
    }
    print "costs\tfigures\t-\t-\tfail\n" unless $n;
  ' "$log"
}

# results.tsv against the night before's: a row per metric in either,
# with both values, the change where both are numbers, and `moved`
# where the status is not the one it was.
diff_results() {
  local prev="$1" now="$2"
  perl -e '
    sub read_tsv { my %h; my @k; open my $f, "<", $_[0] or return ({}, []);
      while (<$f>) { chomp; next if /^check\t/; my @c = split /\t/;
        my $k = "$c[0]\t$c[1]"; push @k, $k unless exists $h{$k}; $h{$k} = [$c[2], $c[3], $c[4]] }
      (\%h, \@k) }
    my ($p, $pk) = read_tsv($ARGV[0]);
    my ($n, $nk) = read_tsv($ARGV[1]);
    print "check\tmetric\tbefore\tnow\tchange\tstatus\n";
    my %seen;
    for my $k (@$nk, @$pk) {
      next if $seen{$k}++;
      my ($a, $b) = ($p->{$k}, $n->{$k});
      my $va = $a ? $a->[0] : "-";
      my $vb = $b ? $b->[0] : "-";
      my $num = qr/^-?\d+(?:\.\d+)?$/;
      # Whole numbers as whole, others to the hundredth, never in
      # exponent form.
      my $ch = "-";
      if ($va =~ $num && $vb =~ $num) {
        $ch = ($va =~ /^-?\d+$/ && $vb =~ /^-?\d+$/)
          ? sprintf("%+d", $vb - $va) : sprintf("%+.2f", $vb - $va);
      }
      my $st = !$a ? "new, $b->[2]"
        : !$b ? "gone"
        : $a->[2] eq $b->[2] ? $b->[2]
        : "moved $a->[2] to $b->[2]";
      print "$k\t$va\t$vb\t$ch\t$st\n";
    }
  ' "$prev" "$now"
}
