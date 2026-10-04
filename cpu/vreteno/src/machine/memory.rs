// SPDX-License-Identifier: Apache-2.0
//! The DDR3, as plain memory: a gigabyte from `0x4000_0000`, kept in
//! pages made when first written, so a model that touches a few
//! megabytes holds a few megabytes (issue 1016).
use std::collections::HashMap;

/// Words in a page: 4 KiB.
const PAGE_WORDS: usize = 1024;

/// Memory of `len` bytes at `base`, read as zero until written.
#[derive(Clone, Debug)]
pub struct Memory {
    pub base: u32,
    pub len: u32,
    pages: HashMap<u32, Box<[u32; PAGE_WORDS]>>,
}

impl Memory {
    pub fn new(base: u32, len: u32) -> Self {
        Memory {
            base,
            len,
            pages: HashMap::new(),
        }
    }

    /// Whether `addr` is inside.
    pub fn holds(&self, addr: u32) -> bool {
        addr.wrapping_sub(self.base) < self.len
    }

    fn at(addr: u32, base: u32) -> (u32, usize) {
        let w = (addr.wrapping_sub(base) / 4) as usize;
        ((w / PAGE_WORDS) as u32, w % PAGE_WORDS)
    }

    /// The word at `addr`, which `holds`.
    pub fn load(&self, addr: u32) -> u32 {
        let (p, i) = Self::at(addr, self.base);
        self.pages.get(&p).map_or(0, |page| page[i])
    }

    /// The word at `addr`, written.
    pub fn store(&mut self, addr: u32, v: u32) {
        let (p, i) = Self::at(addr, self.base);
        self.pages
            .entry(p)
            .or_insert_with(|| Box::new([0; PAGE_WORDS]))[i] = v;
    }

    /// Bytes laid down from `addr`, little-endian, as a loader does.
    pub fn put(&mut self, addr: u32, bytes: &[u8]) {
        for (k, chunk) in bytes.chunks(4).enumerate() {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            let at = addr + 4 * k as u32;
            let mut v = u32::from_le_bytes(w);
            if chunk.len() < 4 {
                let keep = !((1u64 << (8 * chunk.len())) - 1) as u32;
                v |= self.load(at) & keep;
            }
            self.store(at, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_read_back_and_untouched_memory_reads_zero() {
        let mut m = Memory::new(0x4000_0000, 0x4000_0000);
        assert!(m.holds(0x4000_0000) && m.holds(0x7fff_fffc));
        assert!(!m.holds(0x8000_0000) && !m.holds(0x3fff_fffc));
        assert_eq!(m.load(0x5000_0000), 0);
        m.store(0x5000_0000, 0x1234_5678);
        assert_eq!(m.load(0x5000_0000), 0x1234_5678);
        m.put(0x4000_0000, &[1, 2, 3, 4, 5]);
        assert_eq!(m.load(0x4000_0000), 0x0403_0201);
        assert_eq!(m.load(0x4000_0004), 5, "a short last word");
    }
}
