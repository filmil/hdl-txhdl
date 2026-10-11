# The canonical demos

After each bigger milestone the same demos are run on the board again and their videos kept, so that one milestone can be set beside the next (issue 1592).
This page says what they are, how a run is made, and where its videos go.

## What is shown

`docs/demos.tsv` is the list, in the order a run shows it.
Each line names a demo, its image's Bazel target, how it is loaded, how long it is shown, and what ends it early.

* **The icosahedron series**: one image for each GPU feature, oldest first, from the flat icosahedron the core draws itself to the textured and mipmapped ones Razboj draws.
  A new feature adds a line at the end of the series.
* **The teapot**: the Utah teapot, so that every milestone draws the same well-known model.
* **Linux on the console**: the board boots Linux and runs a fixed list of commands, each printed before its output, until `txhdl: demo done`.
* **Doom**: the first level, from Freedoom's WAD.

Each demo is shown for a fixed number of seconds, so two runs line up frame for frame.
A line that starts with `#` is skipped; that is where a demo waits until its image exists.

## Making a run

Hold the board token, then, from a checkout of the commit the run is for:

```sh
bazel run //cpu/vreteno/board/remote:demos -- --out=$HOME/txhdl-demos
```

The tool:

1. builds every image in the list and the fastboot app at that commit, and the flagship under the Vivado lock;
2. programs the flagship;
3. for each demo in order:
   * stops if anything else holds the board's serial port, since a second reader there takes the loader's acknowledgements;
   * loads the demo with the serial loader or with fastboot;
   * keeps its console for its seconds, every line stamped in UTC;
   * stops its own reader on the board server;
4. writes the run's directory.

A demo after a fastboot one starts from a reprogram, since Linux does not answer the serial line's reset.
The tool refuses to run across 07:05 to 07:15 UTC, when the board is switched off every day.
`--dry-run` builds the images and prints the plan without the flagship or the board.

## What a run leaves

A run's directory, `<start>-<sha>`, is named by its start in Pacific time, in ISO 8601's basic format with the offset (`<YYYYMMDD>T<HHMM><offset>`), then main's short sha, which follows the last hyphen: for example `20261010T1830-0700-94d3c50d`.
The basic format has no colons, so the name is safe on every filesystem.
The times inside the directory are UTC, so an evening run's stage times fall on the next day's date.
The directory holds:

* `<demo>.log`, each demo's console with its UTC stamps;
* `manifest.tsv`, each demo's target, image sha256, start and end in UTC, and the console's last line;
* `README.md`, with main's commit, the flagship's sha256 and the run's time in UTC.

The board server records the screen throughout.
Its owner cuts `<demo>.mp4` from each demo's start to its end in `manifest.tsv`, and puts the clips, the logs and the manifest in `TxHDL/demos/<start>-<sha>/` on the shared drive.
