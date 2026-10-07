// SPDX-License-Identifier: Apache-2.0
//! Textures (#997): GL's texture objects, the room they live in, and
//! their upload into the layout Razboj reads (`razboj_tile::tex`).
//!
//! The room is the caller's, as a frame's is: words of DDR3, which this
//! module reaches through `mem` and Razboj at the bus address `bus`. Its
//! first `MAX_TEXTURES` times sixteen words are the descriptor table,
//! object `n`'s descriptor at index `n - 1`, and the texels follow,
//! handed out in order and never given back: deleting an object frees its
//! name and not its texels, which a program that loads its textures once
//! does not miss.
//!
//! Every texel is stored as RGBA in 32 bits, `0xAARRGGBB`, whatever the
//! format it was given in, and the object remembers which of GL's base
//! formats it was, since the environments read each differently.
use crate::gl;
use razboj_tile::tex::{self, Desc, DESC_WORDS, MAX_LEVELS};

/// How many texture objects there are room for.
pub const MAX_TEXTURES: usize = 64;

/// The words the descriptor table takes at the head of the room.
pub const TABLE_WORDS: usize = MAX_TEXTURES * DESC_WORDS;

/// A texture object's state as GL keeps it.
#[derive(Clone, Copy, Debug)]
pub struct Object {
    /// Whether its name is in use.
    pub live: bool,
    /// Its levels' sides, as their log2, once level 0 is given.
    pub size: Option<(u32, u32)>,
    /// Which levels have been given, a bit a level.
    pub defined: u32,
    /// GL's base format, `gl::RGBA` to `gl::LUMINANCE_ALPHA`.
    pub format: u32,
    /// Where its texels start in the room, a word index.
    pub at: usize,
    /// Its parameters, as GL's enumerants.
    pub min: u32,
    pub mag: u32,
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub generate: bool,
}

impl Object {
    /// A new object, in GL's initial state.
    const NEW: Object = Object {
        live: false,
        size: None,
        defined: 0,
        format: gl::RGBA,
        at: 0,
        min: gl::NEAREST_MIPMAP_LINEAR,
        mag: gl::LINEAR,
        wrap_s: gl::REPEAT,
        wrap_t: gl::REPEAT,
        generate: false,
    };
}

/// The room and the objects in it.
pub struct Store<'a> {
    mem: &'a mut [u32],
    bus: u32,
    used: usize,
    objects: [Object; MAX_TEXTURES],
}

/// How many levels a texture of these log2 sides has: down to one texel.
fn chain(w: u32, h: u32) -> u32 {
    w.max(h) + 1
}

/// The room a level takes, in words.
fn level_words(w: u32, h: u32, level: u32) -> usize {
    (tex::blocks(w, level) * tex::blocks(h, level) * 16) as usize
}

/// The log2 of `n`, if `n` is a power of two from one to the largest
/// side.
fn log2(n: u32) -> Option<u32> {
    (n.is_power_of_two() && n <= gl::MAX_TEXTURE_SIZE)
        .then(|| n.trailing_zeros())
}

/// One texel of `pixels` in `format` and `type_`, at byte `at`, as
/// `0xAARRGGBB`; channels narrower than a byte are widened by repeating
/// their top bits, as GL's conversion to fixed point does.
fn texel(format: u32, type_: u32, pixels: &[u8], at: usize) -> u32 {
    let b = |k: usize| pixels[at + k] as u32;
    let wide = |v: u32, bits: u32| {
        let v = v << (8 - bits);
        v | v >> bits
    };
    let rgba =
        |r: u32, g: u32, bl: u32, a: u32| a << 24 | r << 16 | g << 8 | bl;
    let short = || b(0) | b(1) << 8;
    match (format, type_) {
        (gl::RGBA, gl::UNSIGNED_SHORT_4_4_4_4) => {
            let s = short();
            let f = |k: u32| wide((s >> k) & 15, 4);
            rgba(f(12), f(8), f(4), f(0))
        }
        (gl::RGBA, gl::UNSIGNED_SHORT_5_5_5_1) => {
            let s = short();
            let f = |k: u32| wide((s >> k) & 31, 5);
            rgba(f(11), f(6), f(1), if s & 1 == 1 { 255 } else { 0 })
        }
        (gl::RGB, gl::UNSIGNED_SHORT_5_6_5) => {
            let s = short();
            rgba(
                wide(s >> 11, 5),
                wide((s >> 5) & 63, 6),
                wide(s & 31, 5),
                255,
            )
        }
        (gl::RGBA, _) => rgba(b(0), b(1), b(2), b(3)),
        (gl::RGB, _) => rgba(b(0), b(1), b(2), 255),
        (gl::LUMINANCE, _) => rgba(b(0), b(0), b(0), 255),
        (gl::LUMINANCE_ALPHA, _) => rgba(b(0), b(0), b(0), b(1)),
        _ => rgba(0, 0, 0, b(0)),
    }
}

/// The bytes a texel takes in `format` and `type_`, or `None` for a pair
/// GL ES does not have.
fn texel_bytes(format: u32, type_: u32) -> Option<usize> {
    match (format, type_) {
        (gl::RGBA, gl::UNSIGNED_BYTE) => Some(4),
        (gl::RGB, gl::UNSIGNED_BYTE) => Some(3),
        (gl::LUMINANCE_ALPHA, gl::UNSIGNED_BYTE) => Some(2),
        (gl::LUMINANCE | gl::ALPHA, gl::UNSIGNED_BYTE) => Some(1),
        (gl::RGBA, gl::UNSIGNED_SHORT_4_4_4_4 | gl::UNSIGNED_SHORT_5_5_5_1) => {
            Some(2)
        }
        (gl::RGB, gl::UNSIGNED_SHORT_5_6_5) => Some(2),
        _ => None,
    }
}

impl<'a> Store<'a> {
    /// A store over `mem`, which Razboj reads at `bus`, with no objects.
    pub fn new(mem: &'a mut [u32], bus: u32) -> Self {
        let used = TABLE_WORDS.min(mem.len());
        Store {
            mem,
            bus,
            used,
            objects: [Object::NEW; MAX_TEXTURES],
        }
    }

    /// The descriptor table's bus address, which Razboj is told.
    pub fn table(&self) -> u32 {
        self.bus
    }

    /// The bus address of object `name`'s descriptor, which a textured
    /// entry names its texture by.
    pub fn desc_at(&self, name: u32) -> u32 {
        self.bus + (name - 1) * (DESC_WORDS as u32) * 4
    }

    /// The room, read back: for a test that hands it to Razboj's model.
    pub fn words(&self) -> &[u32] {
        self.mem
    }

    /// The object called `name`, if it is live.
    pub fn object(&self, name: u32) -> Option<&Object> {
        let o = self.objects.get(name.checked_sub(1)? as usize)?;
        o.live.then_some(o)
    }

    fn object_mut(&mut self, name: u32) -> Option<&mut Object> {
        let o = self.objects.get_mut(name.checked_sub(1)? as usize)?;
        o.live.then_some(o)
    }

    /// `glGenTextures`: names not in use, written into `out`, each a new
    /// object; `None` when there is no room for that many.
    pub fn gen(&mut self, out: &mut [u32]) -> Option<()> {
        let free = self.objects.iter().filter(|o| !o.live).count();
        if free < out.len() {
            return None;
        }
        let mut k = 0;
        for slot in out.iter_mut() {
            while self.objects[k].live {
                k += 1;
            }
            self.objects[k] = Object {
                live: true,
                ..Object::NEW
            };
            *slot = k as u32 + 1;
        }
        Some(())
    }

    /// The texture called `name` made live if it is not, as `glBindTexture`
    /// makes a name not in use; `false` for a name past the room.
    pub fn ensure(&mut self, name: u32) -> bool {
        let Some(o) = name
            .checked_sub(1)
            .and_then(|k| self.objects.get_mut(k as usize))
        else {
            return false;
        };
        if !o.live {
            *o = Object {
                live: true,
                ..Object::NEW
            };
        }
        true
    }

    /// `glDeleteTextures`: the names given back. Nought and names not in
    /// use are passed over, as GL says.
    pub fn delete(&mut self, names: &[u32]) {
        for &n in names {
            if let Some(o) = self.object_mut(n) {
                *o = Object::NEW;
            }
        }
    }

    /// `glTexParameter`: one of the object's four parameters and the
    /// mipmap generation, or the error GL gives.
    pub fn parameter(
        &mut self,
        name: u32,
        pname: u32,
        value: u32,
    ) -> Result<(), u32> {
        let o = self.object_mut(name).ok_or(gl::INVALID_OPERATION)?;
        let wrap = matches!(value, gl::REPEAT | gl::CLAMP_TO_EDGE);
        match pname {
            gl::TEXTURE_MIN_FILTER
                if matches!(value, gl::NEAREST | gl::LINEAR)
                    || (gl::NEAREST_MIPMAP_NEAREST
                        ..=gl::LINEAR_MIPMAP_LINEAR)
                        .contains(&value) =>
            {
                o.min = value
            }
            gl::TEXTURE_MAG_FILTER
                if matches!(value, gl::NEAREST | gl::LINEAR) =>
            {
                o.mag = value
            }
            gl::TEXTURE_WRAP_S if wrap => o.wrap_s = value,
            gl::TEXTURE_WRAP_T if wrap => o.wrap_t = value,
            gl::GENERATE_MIPMAP => o.generate = value != 0,
            _ => return Err(gl::INVALID_ENUM),
        }
        self.publish(name);
        Ok(())
    }

    /// `glTexImage2D` of level `level`: `pixels`, `width` by `height` in
    /// `format` and `type_`, rows padded to `align` bytes, into the
    /// object `name`; or the error GL gives. Level 0 of a new size takes
    /// room for its whole chain of levels. With mipmap generation on,
    /// level 0 also writes every level below it.
    #[allow(clippy::too_many_arguments)] // GL's own arguments, in its order.
    pub fn image(
        &mut self,
        name: u32,
        level: u32,
        format: u32,
        width: u32,
        height: u32,
        type_: u32,
        pixels: &[u8],
        align: usize,
    ) -> Result<(), u32> {
        let bytes = texel_bytes(format, type_).ok_or(gl::INVALID_ENUM)?;
        let (lw, lh) = match (log2(width), log2(height)) {
            (Some(w), Some(h)) => (w, h),
            _ => return Err(gl::INVALID_VALUE),
        };
        let row = (width as usize * bytes).div_ceil(align) * align;
        if pixels.len() < row * (height as usize - 1) + width as usize * bytes {
            return Err(gl::INVALID_VALUE);
        }
        let o = *self.object(name).ok_or(gl::INVALID_OPERATION)?;
        // The base level's sides this level belongs to.
        let base = if level == 0 {
            (lw, lh)
        } else {
            let (bw, bh) = o.size.ok_or(gl::INVALID_OPERATION)?;
            if level >= chain(bw, bh)
                || tex::side(bw, level) != width
                || tex::side(bh, level) != height
            {
                return Err(gl::INVALID_VALUE);
            }
            (bw, bh)
        };
        let mut o = o;
        if level == 0 && (o.size != Some(base) || o.format != format) {
            let words: usize =
                (0..chain(lw, lh)).map(|l| level_words(lw, lh, l)).sum();
            if self.mem.len() - self.used < words {
                return Err(gl::OUT_OF_MEMORY);
            }
            (o.at, o.size, o.defined) = (self.used, Some(base), 0);
            self.used += words;
        }
        o.format = format;
        let d = self.desc_of(&o);
        for j in 0..height {
            for i in 0..width {
                let at = j as usize * row + i as usize * bytes;
                let t = texel(format, type_, pixels, at);
                self.put(&d, level, i, j, t);
            }
        }
        o.defined |= 1 << level;
        if level == 0 && o.generate {
            for l in 1..chain(base.0, base.1) {
                self.reduce(&d, l);
                o.defined |= 1 << l;
            }
        }
        self.objects[name as usize - 1] = o;
        self.publish(name);
        Ok(())
    }

    /// The texel `(i, j)` of a level of the texture `d` describes, written.
    fn put(&mut self, d: &Desc, level: u32, i: u32, j: u32, t: u32) {
        let at = tex::level_base(d, level) + tex::texel_offset(d, level, i, j);
        self.mem[((at - self.bus) / 4) as usize] = t;
    }

    /// The same, read.
    fn get(&self, d: &Desc, level: u32, i: u32, j: u32) -> u32 {
        let at = tex::level_base(d, level) + tex::texel_offset(d, level, i, j);
        self.mem[((at - self.bus) / 4) as usize]
    }

    /// Level `level` written from the one above it: each texel the mean
    /// of the two by two above it, or of the two where a side is already
    /// one, rounded, a channel at a time.
    fn reduce(&mut self, d: &Desc, level: u32) {
        let (w, h) = (tex::side(d.log_w, level), tex::side(d.log_h, level));
        let (up_w, up_h) =
            (tex::side(d.log_w, level - 1), tex::side(d.log_h, level - 1));
        for j in 0..h {
            for i in 0..w {
                let xs = if up_w > w { [2 * i, 2 * i + 1] } else { [i, i] };
                let ys = if up_h > h { [2 * j, 2 * j + 1] } else { [j, j] };
                let mut sum = [0u32; 4];
                for &y in &ys {
                    for &x in &xs {
                        let t = self.get(d, level - 1, x, y);
                        for (c, s) in sum.iter_mut().enumerate() {
                            *s += (t >> (8 * c)) & 0xff;
                        }
                    }
                }
                let t =
                    (0..4).fold(0, |a, c| a | ((sum[c] + 2) >> 2) << (8 * c));
                self.put(d, level, i, j, t);
            }
        }
    }

    /// Whether the object is complete, as GL's texturing needs: its base
    /// level given, and every level below it too when its minification
    /// filter reads them.
    pub fn complete(&self, name: u32) -> bool {
        let Some(o) = self.object(name) else {
            return false;
        };
        let Some((w, h)) = o.size else {
            return false;
        };
        let mips = !matches!(o.min, gl::NEAREST | gl::LINEAR);
        let need = if mips { (1 << chain(w, h)) - 1 } else { 1 };
        o.defined & need == need
    }

    /// The descriptor of an object, as Razboj reads it.
    fn desc_of(&self, o: &Object) -> Desc {
        let (w, h) = o.size.unwrap_or((0, 0));
        let filter = |f: u32| match f {
            gl::NEAREST => tex::NEAREST,
            gl::LINEAR => tex::LINEAR,
            f => f - gl::NEAREST_MIPMAP_NEAREST + tex::NEAREST_MIPMAP_NEAREST,
        };
        let class = match o.format {
            gl::RGB => tex::RGB,
            gl::ALPHA => tex::ALPHA,
            gl::LUMINANCE => tex::LUMINANCE,
            gl::LUMINANCE_ALPHA => tex::LUMINANCE_ALPHA,
            _ => tex::RGBA,
        };
        Desc {
            base: self.bus + 4 * o.at as u32,
            log_w: w,
            log_h: h,
            levels: chain(w, h).min(MAX_LEVELS as u32),
            clamp_s: o.wrap_s == gl::CLAMP_TO_EDGE,
            clamp_t: o.wrap_t == gl::CLAMP_TO_EDGE,
            min: filter(o.min),
            mag: filter(o.mag),
            class,
        }
    }

    /// The object's descriptor, written into the table for Razboj.
    fn publish(&mut self, name: u32) {
        let Some(o) = self.object(name).copied() else {
            return;
        };
        if o.size.is_none() || self.mem.len() < TABLE_WORDS {
            return;
        }
        let w = tex::encode(&self.desc_of(&o));
        let at = (name as usize - 1) * DESC_WORDS;
        self.mem[at..at + DESC_WORDS].copy_from_slice(&w);
    }

    /// The object's descriptor, as published, for the triangles' level of
    /// detail and for tests.
    pub fn desc(&self, name: u32) -> Option<Desc> {
        self.object(name)
            .filter(|o| o.size.is_some())
            .map(|o| self.desc_of(o))
    }
}
