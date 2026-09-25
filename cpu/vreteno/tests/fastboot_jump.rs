// SPDX-License-Identifier: Apache-2.0
//! Fastboot's copy and jump, run on the core's model (issue 143).
//!
//! `boot` copies a program over the Zephyr that is running and jumps
//! to it, from a routine in `zephyr/fastboot/app/src/jump.S` that is
//! first moved to the end of the staging area. Nothing but the board
//! runs the whole of that, so this runs the part that can go wrong
//! quietly: the routine itself, assembled from that file, on the
//! model. The routine names no address of its own, so it is placed
//! where the model has memory, in the data memory, rather than at
//! 0x4800_0000; running it from a place it was not assembled for is
//! the property the board relies on.
use vreteno32::core::IMEM_BYTES;
use vreteno32::model::{Halt, Model, DATA_BASE, DATA_BYTES};

/// `addi x5, x0, 0x123`, which marks that the copy ran.
const MARK: u32 = 0x1230_0293;
/// `csrwi mhalt, 1`, which stops the model.
const HALT: u32 = 0x7c00_d073;

fn routine() -> Vec<u32> {
    let path = std::env::var("FB_JUMP").expect("FB_JUMP names the routine");
    let bytes = std::fs::read(path).expect("the assembled routine");
    assert!(bytes.len().is_multiple_of(4) && !bytes.is_empty());
    bytes
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Stage `program` at `src`, the routine at `at`, and run the routine
/// with `a0` = `src`, `a1` = `dst`, `a2` = the length, as `boot` calls
/// it. Returns the model once it stops.
fn run(program: &[u32], src: u32, dst: u32, at: u32) -> Model {
    let mut m = Model {
        mem: vec![0; (DATA_BYTES / 4) as usize],
        ..Model::default()
    };
    let word = |a: u32| ((a - DATA_BASE) / 4) as usize;
    for (i, w) in program.iter().enumerate() {
        m.mem[word(src) + i] = *w;
    }
    for (i, w) in routine().iter().enumerate() {
        m.mem[word(at) + i] = *w;
    }
    m.pc = at;
    m.x[10] = src;
    m.x[11] = dst;
    m.x[12] = 4 * program.len() as u32;
    let imem = vec![0u32; (IMEM_BYTES / 4) as usize];
    for _ in 0..10_000 {
        if m.halted.is_some() {
            break;
        }
        m.step(&imem, None);
    }
    m
}

#[test]
fn the_program_is_copied_and_run_from_where_it_was_put() {
    let program = [MARK, HALT];
    let (src, dst, at) =
        (DATA_BASE + 0x100, DATA_BASE + 0x400, DATA_BASE + 0x800);
    let m = run(&program, src, dst, at);
    assert!(
        matches!(m.halted, Some(Halt::Break)),
        "the program ran to its halt: {:?} at pc {:#x}",
        m.halted,
        m.pc
    );
    assert_eq!(m.x[5], 0x123, "the program's first word ran");
    let d = ((dst - DATA_BASE) / 4) as usize;
    assert_eq!(&m.mem[d..d + 2], &program, "the program is at dst");
}

/// A program of one word, which leaves the read-back fewer words than
/// it looks for: it must stop at the program rather than read below.
#[test]
fn a_program_of_one_word_runs() {
    let m = run(
        &[HALT],
        DATA_BASE + 0x100,
        DATA_BASE + 0x400,
        DATA_BASE + 0x800,
    );
    assert!(matches!(m.halted, Some(Halt::Break)), "{:?}", m.halted);
    assert_eq!(m.mem[(0x400 / 4) as usize], HALT);
}

/// The routine moved elsewhere runs the same, which is what lets it
/// run from the staging area's last page rather than from the image
/// it was linked into.
#[test]
fn the_routine_runs_wherever_it_is_put() {
    let program = [MARK, HALT];
    for at in [DATA_BASE + 0x600, DATA_BASE + 0xa00, DATA_BASE + 0xe00] {
        let m = run(&program, DATA_BASE + 0x100, DATA_BASE + 0x400, at);
        assert!(matches!(m.halted, Some(Halt::Break)), "at {at:#x}");
        assert_eq!(m.x[5], 0x123, "at {at:#x}");
    }
}
