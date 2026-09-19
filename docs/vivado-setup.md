<!-- SPDX-License-Identifier: Apache-2.0 -->
# Vivado for this repository, from nothing

Status: instructions, September 19, 2026.
Author: automated coding assistant, with human supervision.

This is for someone who has cloned this repository, has not used Bazel
much, and wants to know what to do about Vivado.
It says what to do, in order, and why each step is there.
Everything about the ruleset behind it is in its own documents, which
the last section points at.

## The short answer: you probably need nothing

Vivado is needed for synthesis, for place and route, and for the two
simulations that use Vivado's own simulator.
Every one of those is a target the build will not run unless you ask
for it by name: all twenty-eight of them carry Bazel's `manual` tag.

So this works with nothing installed but Bazelisk, and does not want
Vivado:

```sh
bazel build //...
bazel test //...
```

That is the language, every example, every document, the core, the
lowered netlists and their simulations under `nvc` and Verilator.
If that is all you are doing, stop reading here.

You need Vivado when you want one of these:

```sh
bazel build //cpu/vreteno:vreteno_synth     # the core through synthesis
bazel build //cpu/vreteno:vreteno_board_pnr # and to a bitstream
bazel test  //cpu/vreteno/board/sim:board_test
```

## How Vivado gets here

There are two ways, and this repository is set up for the first.

* **Hermetic.** You hand Bazel the AMD installer archive once, and
  Bazel installs Vivado itself, into a directory it manages, with only
  the device families this project needs.
  Nothing has to be on your `PATH`, and everyone who builds gets the
  same Vivado.
  `.bazelrc` chooses it for everybody:

  ```
  build --@rules_vivado//:vivado_mode=hermetic
  ```

* **Host.** You install Vivado yourself, as you always have, and tell
  Bazel where it is.
  Nothing here is set up for it, but you can switch your own checkout
  over in one file, which the next section is about.

The hermetic installation is configured in `MODULE.bazel`, and those
lines are committed because they are the project's rather than yours:

```python
vivado = use_extension("@rules_vivado//:extensions.bzl", "vivado")
vivado.install(
    install_cache = "/data/cache/vivado-install",
    urls = ["file:///data/tools/archives/FPGAs_AdaptiveSoCs_Unified_SDI_2025.2_1114_2157_1.tar"],
    modules = ["Artix-7"],
)
```

Read as: install Vivado 2025.2 from that archive, keep the
installation in that directory, and install the Artix-7 device family
and nothing else, because the board is an Artix-7 part.

## `user.bazelrc`: the file that is yours

`.bazelrc` is the project's settings and is committed.
Its last line is

```
try-import %workspace%/user.bazelrc
```

`try-import` means Bazel reads that file if it exists and says nothing
if it does not, so you create it only when you want it.
It is in `.gitignore`, so it never reaches a commit.
Put anything in it that is true of your machine rather than of the
project.

It is read last on purpose: **a setting in `user.bazelrc` overrides the
same setting in `.bazelrc`**, with one catch worth knowing before it
wastes an afternoon.
Bazel takes the last setting of a flag, but it ranks a command-specific
line above a general one first.
So a `build` line here beats a `build` line in `.bazelrc`, while a
`common` line does not, however late it comes.
Checked on this tree, with `--symlink_prefix` as the visible flag:

| `.bazelrc` | `user.bazelrc` | Which wins |
|---|---|---|
| `build --symlink_prefix=zzz-` | `common --symlink_prefix=yyy-` | `zzz-`, the one in `.bazelrc` |
| `build --symlink_prefix=zzz-` | `build --symlink_prefix=yyy-` | `yyy-`, the one in `user.bazelrc` |

The rule that follows: **write your line with the same word the line
you mean to beat uses.** The Vivado lines in `.bazelrc` say `build`,
so yours should say `build`.

## If you already have Vivado installed

This is the cheapest way in, and it skips the hundred gigabytes
entirely.
Create `user.bazelrc` with:

```
# Use the Vivado I installed myself, not a Bazel-installed one.
build --@rules_vivado//:vivado_mode=host
build --@rules_vivado//internal:vivado_version=2025.2
build --@rules_vivado//internal:vivado_path=/tools/Xilinx/2025.2/Vivado
```

Point `vivado_path` at the directory that has `bin/vivado` in it, and
set `vivado_version` to match.
If you leave `vivado_path` out it is guessed as
`/opt/Xilinx/<version>/Vivado`.

Nothing else changes: the same targets build, and the rest of the
repository does not know or care which Vivado ran.

## If you want the hermetic one

You need the archive.
It is one `.tar` from AMD's download page, about 96 GiB, named like
`FPGAs_AdaptiveSoCs_Unified_SDI_2025.2_1114_2157_1.tar`, and AMD makes
you log in for it.
Bazel will not fetch it from AMD for you.

You also need room: about 300 GB free while it runs, of which 51 GB
stays.
The first build takes roughly two hours on spinning disks, of which the
install itself is thirteen minutes; the rest is copying a hundred
gigabyte file three times.
It happens once, and survives `bazel clean --expunge`, because the
installation lives in the cache directory rather than in the build
tree.
Bazel 9.2.0 or later is needed, because earlier versions crash partway
through fetching a `file://` URL.

Then there is one thing this repository cannot make easy, and it is
better to say it plainly than to let it surprise you.
**The two paths in `MODULE.bazel` are absolute, and a module file
cannot be overridden per user.** On the machines this project is
developed on, `/data/tools/archives/...` and `/data/cache/vivado-install`
both exist already, and a new checkout there needs no setup at all.
Anywhere else you have two choices:

* **Put the archive where the file says**, at
  `/data/tools/archives/FPGAs_AdaptiveSoCs_Unified_SDI_2025.2_1114_2157_1.tar`,
  and make `/data/cache/vivado-install` a directory you can write to.
  A symbolic link is enough for either.
  This is the tidy option: `MODULE.bazel` stays as committed, and
  `git status` stays clean.

* **Edit the two lines** to paths that suit your machine, and keep the
  edit out of your commits.
  It works, at the cost of a modified file you have to remember.

Why those lines are absolute at all, rather than in `user.bazelrc`
where they would belong: the URL and the component selection are what
name the installation in the cache, so two workspaces that state them
identically share one installation and a workspace that states them
differently gets its own copy.
Keeping them in the committed file is what makes every checkout on a
machine share the fifty gigabytes rather than each installing its own.
`install_cache` in the same block wins over the `RULES_VIVADO_CACHE`
environment variable, so that variable will not move it for you.

### Checking that it worked

The cache directory holds one subdirectory per installation, and a
finished one has a `COMPLETE` file in it:

```
/data/cache/vivado-install/
└── 2025.2-18cd19a4392e25e8/
    ├── COMPLETE
    └── install/
```

`bazel build //cpu/vreteno:vreteno_synth` is a small enough thing to
try it on: minutes once Vivado is there, hours the first time because
it installs first.

## When it goes wrong

* **`No space left on device`, partway through the first build.**
  The peak is around 300 GB, not the 51 GB that remains.
  Three copies of the archive exist at once before it is unpacked.
* **A `NullPointerException` about `URI.getHost()`.**
  Bazel 9.1.0 with a `file://` URL.
  Use 9.2.0 or later.
* **It starts a two-hour install when you expected it to be there
  already.**
  The archive URL, the module list or the cache directory is not what
  the existing installation was made with, so Bazel is making a second
  one.
  Compare your `MODULE.bazel` with the committed one.
* **A Vivado target fails and nothing says why.**
  Read the log the action wrote, `bazel-bin/<package>/<name>.log`.
  Vivado reports most failures there rather than on the console.
* **`user.bazelrc` seems to do nothing.**
  Check the word at the start of the line, as the table above explains,
  and check the file is at the workspace root beside `.bazelrc`.
  `bazel info --announce_rc` prints every line Bazel actually read, in
  the order it read them.

## Where the rest is written

The ruleset is `bazel_rules_vivado`, at
`github.com/filmil/bazel_rules_vivado`, and its own documents cover
what this page skips.
`docs/vivado-toolchain.md` there is the reference: every attribute of
`vivado.install`, the full module menu of the 2025.2 installer, the
other modes, the licensing note, and the install cache in detail.
A gentler first page for any project rather than this one,
`docs/hermetic-quickstart.md`, is proposed there in pull request 131.

In this repository, `README.md` says how to build everything else, and
`docs/README.md` lists the notes in this directory.
