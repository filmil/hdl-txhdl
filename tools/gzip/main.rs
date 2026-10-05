// SPDX-License-Identifier: Apache-2.0
//! A file gzipped the same way every time (issue 1201): the boot image's
//! initramfs, which the kernel unpacks itself.
//!
//! `gzip IN OUT`. The member has no name, no timestamp and an unknown
//! system, so the output depends on the input alone, and its body is
//! `miniz_oxide`'s DEFLATE at level 9, the crate already pinned under
//! the waveform writer. A tool here rather than the host's gzip, which
//! the build does not pin, or a Python step, which new build steps do
//! not use.

/// The CRC-32 gzip's trailer carries: reflected, polynomial
/// `0xEDB88320`, all ones in and out.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// `data` as one gzip member.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    // Magic, DEFLATE, no flags, no time, the slowest compression, and
    // an unknown system.
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 2, 0xff];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 9));
    out.extend(crc32(data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("usage: gzip IN OUT");
        std::process::exit(2)
    };
    let data = std::fs::read(input).unwrap_or_else(|e| panic!("{input}: {e}"));
    std::fs::write(output, gzip(&data))
        .unwrap_or_else(|e| panic!("{output}: {e}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The check value every CRC-32 is held to.
    #[test]
    fn the_crc_is_gzips() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    /// A member's body inflates back to the input, its trailer is the
    /// input's CRC and length, and the header says nothing that varies.
    #[test]
    fn a_member_unpacks_to_what_went_in() {
        let data: Vec<u8> =
            (0..100_000u32).map(|i| (i * 7 / 13) as u8).collect();
        let z = gzip(&data);
        assert_eq!(&z[..10], &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 2, 0xff]);
        let body = &z[10..z.len() - 8];
        let back = miniz_oxide::inflate::decompress_to_vec(body).unwrap();
        assert_eq!(back, data);
        let tail = &z[z.len() - 8..];
        assert_eq!(tail[..4], crc32(&data).to_le_bytes());
        assert_eq!(tail[4..], (data.len() as u32).to_le_bytes());
        assert!(z.len() < data.len() / 4, "it compresses");
        assert_eq!(gzip(&data), z, "the same every time");
    }
}
