// SPDX-License-Identifier: Apache-2.0
//! Raw DEFLATE (RFC 1951) inflated into a slice, with no allocation and
//! no window of its own: the output is the window, so a back reference
//! reads what was written before it (issue 1229). The boot image's
//! inflater runs this on the core to unpack the kernel; the host's
//! tests hold it against `miniz_oxide`, which `bootimg` compresses with.
//!
//! The decoder is the canonical one of zlib's `puff`: a code's length
//! counts and its symbols in order, read a bit at a time. It is slower
//! than a table, and needs no table built per block.
#![no_std]

/// Why a stream did not inflate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input ended inside the stream.
    Truncated,
    /// A block of type 3, which DEFLATE does not have.
    BlockType,
    /// A stored block whose length and its complement disagree.
    StoredLength,
    /// A dynamic block's code lengths that make no code, or too many.
    Lengths,
    /// Bits that are no code's.
    Code,
    /// A back reference to before the output's start.
    Distance,
    /// More output than the slice holds.
    Overflow,
}

/// The input, a bit at a time, least significant bit first. Fewer than
/// eight bits are held between calls, so a stored block's start is the
/// next whole byte.
struct Bits<'a> {
    src: &'a [u8],
    at: usize,
    buf: u32,
    n: u32,
}

impl Bits<'_> {
    fn bits(&mut self, k: u32) -> Result<u32, Error> {
        while self.n < k {
            let b = *self.src.get(self.at).ok_or(Error::Truncated)?;
            self.at += 1;
            self.buf |= (b as u32) << self.n;
            self.n += 8;
        }
        let v = self.buf & ((1u32 << k) - 1);
        self.buf >>= k;
        self.n -= k;
        Ok(v)
    }
}

/// A canonical code: how many codes there are of each length, and the
/// symbols in the order of their codes.
struct Huff {
    count: [u16; 16],
    sym: [u16; 288],
}

impl Huff {
    /// The code of these lengths, one a symbol, nought for none. A code
    /// that is incomplete is taken, as `puff` takes it, and decoding
    /// fails only on bits it has no code for; one that is
    /// over-subscribed is refused.
    fn new(lens: &[u8]) -> Result<Huff, Error> {
        let mut h = Huff {
            count: [0; 16],
            sym: [0; 288],
        };
        for &l in lens {
            h.count[l as usize] += 1;
        }
        h.count[0] = 0;
        let mut left: i32 = 1;
        for len in 1..16 {
            left = (left << 1) - h.count[len] as i32;
            if left < 0 {
                return Err(Error::Lengths);
            }
        }
        let mut offs = [0u16; 16];
        for len in 1..15 {
            offs[len + 1] = offs[len] + h.count[len];
        }
        for (s, &l) in lens.iter().enumerate() {
            if l != 0 {
                h.sym[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(h)
    }

    /// The next symbol, its code read a bit at a time, most significant
    /// first.
    fn decode(&self, b: &mut Bits) -> Result<usize, Error> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return Ok(self.sym[(index + code - first) as usize] as usize);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(Error::Code)
    }
}

/// A length code's base and extra bits, for codes 257 to 285.
const LBASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59,
    67, 83, 99, 115, 131, 163, 195, 227, 258,
];
const LEXT: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5,
    5, 5, 5, 0,
];
/// A distance code's base and extra bits.
const DBASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513,
    769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DEXT: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10,
    11, 11, 12, 12, 13, 13,
];
/// The order a dynamic block's code length code lengths come in.
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// `src`, raw DEFLATE, inflated into `dst`: how many bytes it made.
pub fn inflate(src: &[u8], dst: &mut [u8]) -> Result<usize, Error> {
    let mut b = Bits {
        src,
        at: 0,
        buf: 0,
        n: 0,
    };
    let mut out = 0usize;
    loop {
        let last = b.bits(1)?;
        match b.bits(2)? {
            0 => out = stored(&mut b, dst, out)?,
            1 => {
                let mut lens = [0u8; 288 + 30];
                lens[..144].fill(8);
                lens[144..256].fill(9);
                lens[256..280].fill(7);
                lens[280..288].fill(8);
                lens[288..].fill(5);
                let (l, d) =
                    (Huff::new(&lens[..288])?, Huff::new(&lens[288..])?);
                out = codes(&mut b, dst, out, &l, &d)?;
            }
            2 => {
                let (l, d) = dynamic(&mut b)?;
                out = codes(&mut b, dst, out, &l, &d)?;
            }
            _ => return Err(Error::BlockType),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

/// A stored block: from the next whole byte, its length, the length's
/// complement, and that many bytes as they are.
fn stored(b: &mut Bits, dst: &mut [u8], out: usize) -> Result<usize, Error> {
    b.buf = 0;
    b.n = 0;
    let head = b.src.get(b.at..b.at + 4).ok_or(Error::Truncated)?;
    let len = u16::from_le_bytes([head[0], head[1]]) as usize;
    let nlen = u16::from_le_bytes([head[2], head[3]]) as usize;
    if len != !nlen & 0xffff {
        return Err(Error::StoredLength);
    }
    b.at += 4;
    let from = b.src.get(b.at..b.at + len).ok_or(Error::Truncated)?;
    let to = dst.get_mut(out..out + len).ok_or(Error::Overflow)?;
    to.copy_from_slice(from);
    b.at += len;
    Ok(out + len)
}

/// A dynamic block's two codes, from the code lengths it sends.
fn dynamic(b: &mut Bits) -> Result<(Huff, Huff), Error> {
    let nlen = b.bits(5)? as usize + 257;
    let ndist = b.bits(5)? as usize + 1;
    let ncode = b.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(Error::Lengths);
    }
    let mut cl = [0u8; 19];
    for &i in &ORDER[..ncode] {
        cl[i] = b.bits(3)? as u8;
    }
    let lencode = Huff::new(&cl)?;
    let mut lens = [0u8; 286 + 30];
    let mut i = 0;
    while i < nlen + ndist {
        let sym = lencode.decode(b)?;
        if sym < 16 {
            lens[i] = sym as u8;
            i += 1;
            continue;
        }
        let (value, times) = match sym {
            16 => {
                if i == 0 {
                    return Err(Error::Lengths);
                }
                (lens[i - 1], 3 + b.bits(2)? as usize)
            }
            17 => (0, 3 + b.bits(3)? as usize),
            _ => (0, 11 + b.bits(7)? as usize),
        };
        if i + times > nlen + ndist {
            return Err(Error::Lengths);
        }
        lens[i..i + times].fill(value);
        i += times;
    }
    // A block that cannot end is no block.
    if lens[256] == 0 {
        return Err(Error::Lengths);
    }
    Ok((
        Huff::new(&lens[..nlen])?,
        Huff::new(&lens[nlen..nlen + ndist])?,
    ))
}

/// A block's literals and back references, to its end code.
fn codes(
    b: &mut Bits,
    dst: &mut [u8],
    mut out: usize,
    l: &Huff,
    d: &Huff,
) -> Result<usize, Error> {
    loop {
        let sym = l.decode(b)?;
        if sym < 256 {
            *dst.get_mut(out).ok_or(Error::Overflow)? = sym as u8;
            out += 1;
            continue;
        }
        if sym == 256 {
            return Ok(out);
        }
        let s = sym - 257;
        if s >= 29 {
            return Err(Error::Code);
        }
        let len = LBASE[s] as usize + b.bits(LEXT[s] as u32)? as usize;
        let ds = d.decode(b)?;
        if ds >= 30 {
            return Err(Error::Code);
        }
        let dist = DBASE[ds] as usize + b.bits(DEXT[ds] as u32)? as usize;
        if dist > out {
            return Err(Error::Distance);
        }
        if out + len > dst.len() {
            return Err(Error::Overflow);
        }
        // A byte at a time: the copy may overlap what it is writing.
        for k in out..out + len {
            dst[k] = dst[k - dist];
        }
        out += len;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;
    use std::vec::Vec;

    /// Bytes that compress the way a kernel does, mostly: words drawn
    /// from a small vocabulary, runs, and stretches of noise.
    fn kernelish(n: usize, seed: u32) -> Vec<u8> {
        let mut x = seed | 1;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let words: [&[u8]; 6] = [
            b"\x13\x00\x00\x00",
            b"addi",
            b"\xef\xbe\xad\xde",
            b"ret\n",
            b"\0\0",
            b"linux",
        ];
        let mut v = Vec::new();
        while v.len() < n {
            match next() % 8 {
                0 => v.extend((0..next() % 64).map(|_| next() as u8)),
                1 => v.extend(vec![next() as u8; (next() % 300) as usize]),
                _ => v.extend_from_slice(words[(next() % 6) as usize]),
            }
        }
        v.truncate(n);
        v
    }

    fn round_trip(data: &[u8], level: u8) {
        let z = miniz_oxide::deflate::compress_to_vec(data, level);
        let mut out = vec![0u8; data.len()];
        let n = inflate(&z, &mut out).unwrap_or_else(|e| {
            panic!("level {level}, {} bytes: {e:?}", data.len())
        });
        assert_eq!(n, data.len(), "level {level}: the length");
        assert!(out == data, "level {level}: the bytes");
    }

    /// Every level miniz_oxide writes, stored blocks at level 0 and
    /// fixed and dynamic Huffman blocks above it, inflates to what was
    /// deflated, over sizes from nothing to past a stored block's 64 KiB.
    #[test]
    fn what_miniz_oxide_deflates_inflates_back() {
        for (k, n) in [0usize, 1, 2, 100, 4095, 65535, 65536, 70_001, 300_000]
            .into_iter()
            .enumerate()
        {
            let data = kernelish(n, 0x2545_f491 + k as u32);
            for level in 0..=10 {
                round_trip(&data, level);
            }
        }
        // Noise, which deflate leaves mostly as literals and stored blocks.
        let noise: Vec<u8> = kernelish(200_000, 7)
            .iter()
            .enumerate()
            .map(|(i, b)| b.wrapping_mul(31).wrapping_add(i as u8))
            .collect();
        for level in [0, 1, 6, 9] {
            round_trip(&noise, level);
        }
    }

    /// A stream cut short, a slice too small, a bad block type, a back
    /// reference before the start and a stored length whose complement
    /// disagrees are each refused, never answered with wrong bytes.
    #[test]
    fn a_bad_stream_is_refused() {
        let data = kernelish(50_000, 11);
        let z = miniz_oxide::deflate::compress_to_vec(&data, 9);
        let mut out = vec![0u8; data.len()];
        assert_eq!(inflate(&z[..z.len() / 2], &mut out), Err(Error::Truncated));
        let mut small = vec![0u8; data.len() - 1];
        assert_eq!(inflate(&z, &mut small), Err(Error::Overflow));
        // Last block, type 3.
        assert_eq!(inflate(&[0b111], &mut out), Err(Error::BlockType));
        // A fixed block whose first code is a length: nothing to copy.
        // After the three header bits, code 257, `0000001` read first bit
        // first, ends in the second byte's bit 1; then distance code 0,
        // five zeros.
        assert_eq!(inflate(&[0b011, 0b10, 0], &mut out), Err(Error::Distance));
        // A stored block of length 1 whose complement is not 0xfffe.
        assert_eq!(
            inflate(&[0b001, 1, 0, 0, 0, 0x55], &mut out),
            Err(Error::StoredLength)
        );
    }
}
