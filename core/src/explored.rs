//! Versioned, bounded explored-map tiles. RGBA is lossless; RLE runs include
//! alpha, and incompressible tiles use literal bytes. No terrain is retained.
pub const SIDE: u32 = 64;
pub const BYTES: usize = (SIDE * SIDE * 4) as usize;

/// Incremental encoder: each step scans at most `pixels` texels. A literal
/// fallback prevents random colours expanding the disk footprint.
pub struct Encoder {
    raw: Vec<u8>,
    rle: Vec<u8>,
    at: usize,
}
impl Encoder {
    pub fn new(raw: Vec<u8>) -> Self {
        assert_eq!(raw.len(), BYTES);
        Self {
            raw,
            rle: vec![1],
            at: 0,
        }
    }
    pub fn step(&mut self, pixels: usize) -> bool {
        let end = (self.at + pixels * 4).min(self.raw.len());
        while self.at < end {
            let start = self.at;
            self.at += 4;
            while self.at < end && self.raw[self.at..self.at + 4] == self.raw[start..start + 4] {
                self.at += 4;
            }
            self.rle
                .extend_from_slice(&(((self.at - start) / 4) as u16).to_le_bytes());
            self.rle.extend_from_slice(&self.raw[start..start + 4]);
        }
        self.at == self.raw.len()
    }
    pub fn finish(self) -> Vec<u8> {
        assert_eq!(self.at, BYTES);
        if self.rle.len() <= BYTES {
            self.rle
        } else {
            let mut out = Vec::with_capacity(BYTES + 1);
            out.push(0);
            out.extend_from_slice(&self.raw);
            out
        }
    }
}

pub fn decode(data: &[u8]) -> Option<Vec<u8>> {
    if data.first() == Some(&0) {
        return (data.len() == BYTES + 1).then(|| data[1..].to_vec());
    }
    if data.first() != Some(&1) || !(data.len() - 1).is_multiple_of(6) {
        return None;
    }
    let mut out = Vec::with_capacity(BYTES);
    for run in data[1..].as_chunks::<6>().0 {
        let n = u16::from_le_bytes([run[0], run[1]]) as usize * 4;
        if n == 0 || out.len() + n > BYTES {
            return None;
        }
        // Grow runs through native slice copies, not one guest loop per texel.
        let start = out.len();
        out.extend_from_slice(&run[2..]);
        let mut filled = 4;
        while filled < n {
            let count = filled.min(n - filled);
            out.extend_from_within(start..start + count);
            filled += count;
        }
    }
    (out.len() == BYTES).then_some(out)
}

/// Stable 128-bit namespace digest. Length delimiters distinguish arbitrary
/// world/dimension strings; only hexadecimal reaches the storage key.
pub fn namespace(
    world: &str,
    dimension: &str,
    layer: Option<i32>,
    tpb: u32,
    flat: bool,
) -> [u8; 16] {
    let mut a = 0xcbf29ce484222325u64;
    let mut b = 0x84222325cbf29ce4u64;
    for part in [
        world.as_bytes(),
        dimension.as_bytes(),
        &layer.unwrap_or(i32::MAX).to_le_bytes(),
        &tpb.to_le_bytes(),
        &[flat as u8],
    ] {
        for c in (part.len() as u64)
            .to_le_bytes()
            .into_iter()
            .chain(part.iter().copied())
        {
            a = (a ^ c as u64).wrapping_mul(0x100000001b3);
            b = (b ^ c as u64).wrapping_mul(0x100000001e7);
        }
    }
    let mut id = [0; 16];
    id[..8].copy_from_slice(&a.to_le_bytes());
    id[8..].copy_from_slice(&b.to_le_bytes());
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    fn roundtrip(raw: Vec<u8>) -> Vec<u8> {
        let mut e = Encoder::new(raw.clone());
        while !e.step(256) {}
        let encoded = e.finish();
        assert_eq!(decode(&encoded), Some(raw));
        encoded
    }
    #[test]
    fn lossless_transparency_runs_and_literals() {
        assert!(roundtrip(vec![0; BYTES]).len() < 128);
        let raw: Vec<_> = (0..BYTES)
            .map(|i| (i.wrapping_mul(37) ^ (i >> 8)) as u8)
            .collect();
        assert_eq!(roundtrip(raw).len(), BYTES + 1);
        let mut raw = vec![0; BYTES];
        raw[20..24].copy_from_slice(&[1, 2, 3, 255]);
        roundtrip(raw);
    }
    #[test]
    fn rejects_corrupt_or_oversized_tiles() {
        for data in [
            vec![],
            vec![2],
            vec![0, 1],
            vec![1, 0, 0, 0, 0, 0, 0],
            vec![1, 255, 255, 1, 2, 3, 255],
        ] {
            assert_eq!(decode(&data), None);
        }
    }
    #[test]
    fn namespaces_separate_world_dimension_layer_resolution_and_style() {
        let base = namespace("sp:A", "overworld", None, 1, false);
        for id in [
            namespace("sp:B", "overworld", None, 1, false),
            namespace("sp:A", "nether", None, 1, false),
            namespace("sp:A", "overworld", Some(0), 1, false),
            namespace("sp:A", "overworld", None, 2, false),
            namespace("sp:A", "overworld", None, 1, true),
        ] {
            assert_ne!(id, base);
        }
    }
}
