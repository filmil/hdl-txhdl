// SPDX-License-Identifier: Apache-2.0
//! `glDrawElements` keeps the vertices it has worked out (#1624): a
//! vertex several triangles name is transformed and lit once while it
//! stays in the cache. What it writes must be what drawing each
//! triangle's corners one by one writes, word for word, whatever the
//! order the indices come in and however they collide in the cache.
use gles::fixed::{Fx, ONE};
use gles::{gl, Gl, Vertex};

/// A grid of `n + 1` by `n + 1` points over a bump, its normals leaning
/// out, and the indices of its `2 n n` triangles, square by square.
fn grid(n: usize) -> (Vec<[Fx; 4]>, Vec<[Fx; 3]>, Vec<[Fx; 4]>, Vec<u16>) {
    let (mut p, mut nor, mut col, mut idx) = (vec![], vec![], vec![], vec![]);
    for r in 0..=n {
        for c in 0..=n {
            let (x, y) = (
                c as Fx * ONE / n as Fx - ONE / 2,
                r as Fx * ONE / n as Fx - ONE / 2,
            );
            let bump = (x as i64 * x as i64 + y as i64 * y as i64) / ONE as i64;
            p.push([x, y, -(bump / 2) as Fx - 2 * ONE, ONE]);
            nor.push([x / 2, y / 2, 7 * ONE / 8]);
            col.push([
                ONE,
                (r * 7 % 16) as Fx * ONE / 16,
                (c * 5 % 16) as Fx * ONE / 16,
                ONE - r as Fx * ONE / (2 * n as Fx),
            ]);
        }
    }
    let k = |r: usize, c: usize| (r * (n + 1) + c) as u16;
    for r in 0..n {
        for c in 0..n {
            idx.extend([k(r, c), k(r + 1, c), k(r, c + 1)]);
            idx.extend([k(r + 1, c), k(r + 1, c + 1), k(r, c + 1)]);
        }
    }
    (p, nor, col, idx)
}

/// A context lit from the eye, smooth shaded, depth tested and
/// blending, so that every part of a vertex's working reaches the list.
fn context(frame: &mut [[u32; 16]]) -> Gl<'_> {
    let mut g = Gl::new(frame, 128, 128);
    g.matrix_mode(gl::PROJECTION);
    g.frustum(-ONE / 2, ONE / 2, -ONE / 2, ONE / 2, ONE, 8 * ONE);
    g.matrix_mode(gl::MODELVIEW);
    g.light(gl::LIGHT0, gl::POSITION, &[0, 0, ONE, 0]);
    g.light(gl::LIGHT0, gl::DIFFUSE, &[ONE, ONE, ONE, ONE]);
    g.light(gl::LIGHT0, gl::SPECULAR, &[ONE, ONE, ONE, ONE]);
    g.material(
        gl::FRONT_AND_BACK,
        gl::SPECULAR,
        &[ONE / 2, ONE / 2, ONE / 2, ONE],
    );
    g.material(gl::FRONT_AND_BACK, gl::SHININESS, &[16 * ONE]);
    g.enable(gl::LIGHTING);
    g.enable(gl::LIGHT0);
    g.enable(gl::COLOR_MATERIAL);
    g.enable(gl::DEPTH_TEST);
    g.enable(gl::BLEND);
    g.blend_func(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
    g.shade_model(gl::SMOOTH);
    g.rotate(30 * ONE, ONE, ONE, 0);
    g
}

/// The list `draw_elements` writes for `idx`, and the one drawing each
/// index's vertex in turn writes.
fn both(n: usize, idx: &[u16]) -> (Vec<[u32; 16]>, Vec<[u32; 16]>) {
    let (p, nor, col, _) = grid(n);
    let mut a = vec![[0u32; 16]; 8192];
    let mut b = vec![[0u32; 16]; 8192];
    let mut g = context(&mut a);
    g.draw_elements(gl::TRIANGLES, idx, &p, Some(&col), Some(&nor));
    assert_eq!(g.get_error(), gl::NO_ERROR);
    let elements = g.frame().to_vec();
    let mut g = context(&mut b);
    g.draw_vertices(gl::TRIANGLES, idx.len(), |k| {
        let i = idx[k] as usize;
        Vertex {
            position: p[i],
            colour: Some(col[i]),
            normal: Some(nor[i]),
            tex: None,
        }
    });
    let one_by_one = g.frame().to_vec();
    (elements, one_by_one)
}

#[test]
fn an_indexed_grid_draws_as_its_corners_one_by_one() {
    let n = 6;
    let (_, _, _, idx) = grid(n);
    let (elements, one_by_one) = both(n, &idx);
    assert!(elements.len() > 2 * n * n, "the triangles were drawn");
    assert_eq!(elements, one_by_one);
}

#[test]
fn indices_that_collide_in_the_cache_draw_the_same() {
    // Indices sixteen apart share a slot, and an index comes back after
    // its slot was taken, so a vertex is worked out again rather than
    // read from a slot another one holds.
    let n = 6;
    let idx = [0u16, 16, 32, 16, 0, 33, 1, 17, 0, 48, 32, 1];
    let (elements, one_by_one) = both(n, &idx);
    assert_eq!(elements, one_by_one);
}

#[test]
fn a_call_forgets_what_the_call_before_kept() {
    // The same indices into other arrays: the second call must work its
    // vertices out afresh.
    let n = 4;
    let (p, nor, col, idx) = grid(n);
    let shifted: Vec<[Fx; 4]> = p
        .iter()
        .map(|v| [v[0] + ONE / 8, v[1], v[2], v[3]])
        .collect();
    let mut a = vec![[0u32; 16]; 4096];
    let mut g = context(&mut a);
    g.draw_elements(gl::TRIANGLES, &idx, &p, Some(&col), Some(&nor));
    let first = g.frame().len();
    g.draw_elements(gl::TRIANGLES, &idx, &shifted, Some(&col), Some(&nor));
    let second = g.frame()[first..].to_vec();
    let mut b = vec![[0u32; 16]; 4096];
    let mut g = context(&mut b);
    g.draw_elements(gl::TRIANGLES, &idx, &shifted, Some(&col), Some(&nor));
    assert_eq!(second, g.frame().to_vec());
}
