// SPDX-License-Identifier: Apache-2.0
//! The framebuffer, to be looked at: a PNG file, and colour on a
//! terminal.
//!
//! The PNG is written here rather than by a library, because the
//! build has no image tool and a PNG whose pixels are stored rather
//! than compressed is sixty lines. It is the demonstration's output
//! and the picture the document shows.

/// The CRC-32 of a byte string, as PNG defines it.
fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// The Adler-32 of a byte string, as zlib defines it.
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// One PNG chunk: length, type, data, CRC of the type and the data.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// A framebuffer of `w` by `h` pixels as a PNG, every pixel drawn as a
/// `scale` by `scale` block so the picture is crisp at any size.
pub fn png(fb: &[u32], w: usize, h: usize, scale: usize) -> Vec<u8> {
    let (pw, ph) = (w * scale, h * scale);
    // The raw image: a filter byte of zero, then three bytes a pixel.
    let mut raw = Vec::with_capacity(ph * (1 + pw * 3));
    for y in 0..ph {
        raw.push(0);
        for x in 0..pw {
            let p = fb[(y / scale) * w + x / scale];
            raw.push((p >> 16) as u8);
            raw.push((p >> 8) as u8);
            raw.push(p as u8);
        }
    }
    // A zlib stream of stored blocks: no compression, no table.
    let mut z = vec![0x78, 0x01];
    for (i, part) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(if last { 1 } else { 0 });
        z.extend_from_slice(&(part.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(part.len() as u16)).to_le_bytes());
        z.extend_from_slice(part);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(pw as u32).to_be_bytes());
    ihdr.extend_from_slice(&(ph as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// The framebuffer as colour on a terminal: one character per two
/// pixels, the upper pixel as the foreground of a half block and the
/// lower as the background, so the picture keeps its aspect.
pub fn ansi(fb: &[u32], w: usize, h: usize) -> String {
    let rgb = |p: u32| ((p >> 16) & 255, (p >> 8) & 255, p & 255);
    let mut s = String::new();
    let mut y = 0;
    while y < h {
        for x in 0..w {
            let (tr, tg, tb) = rgb(fb[y * w + x]);
            let (br, bg, bb) = if y + 1 < h {
                rgb(fb[(y + 1) * w + x])
            } else {
                (0, 0, 0)
            };
            s.push_str(&format!(
                "\x1b[38;2;{tr};{tg};{tb}m\x1b[48;2;{br};{bg};{bb}m\u{2580}"
            ));
        }
        s.push_str("\x1b[0m\n");
        y += 2;
    }
    s
}

/// The same picture where colour is not wanted: one character per
/// pixel, chosen by brightness, so a printout in a document shows
/// what was drawn.
pub fn ascii(fb: &[u32], w: usize, h: usize) -> String {
    let ramp = [' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];
    let mut s = String::new();
    for y in 0..h {
        for x in 0..w {
            let p = fb[y * w + x];
            let (r, g, b) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
            let lum = (r * 30 + g * 59 + b * 11) / 100;
            s.push(ramp[(lum as usize * (ramp.len() - 1)) / 255]);
        }
        s.push('\n');
    }
    s
}
