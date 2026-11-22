//! `tlc2.util.FP64`, per tool instead of static. Primitive `Extend` operations only; which
//! bytes a value contributes is decided in `value::fingerprint` and nowhere else.

use crate::fp64_polys::POLYS;

/// A 64-bit fingerprint. Full 64 bits are kept for the "actual fingerprints" line and for
/// the RandomElement seed; the FPSet stores [`FpKey`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fp(u64);

/// What every FPSet stores: `fp & 0x7FFF_FFFF_FFFF_FFFF` (`DiskFPSet.java:65,775`, bit 63 is
/// the "flushed" flag). Only constructible `From<Fp>`, so no set can be keyed by an unmasked
/// value.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct FpKey(u64);

impl From<Fp> for FpKey {
    fn from(fp: Fp) -> Self {
        FpKey(fp.0 & 0x7FFF_FFFF_FFFF_FFFF)
    }
}

impl FpKey {
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl Fp {
    pub fn raw(self) -> u64 {
        self.0
    }
    /// `FP64.Zero`, which `IValue.initialize()` fingerprints against: the result is thrown
    /// away, the point being the `convertAndCache` a lazy value runs on the way.
    pub(crate) fn zero() -> Fp {
        Fp(0)
    }
    /// `(int) fp & 0x7FFFFFFF`, the `SetOfStates` slot hash.
    pub fn slot_hash(self) -> u32 {
        (self.0 as u32) & 0x7FFF_FFFF
    }
}

/// Number of `-fp` choices (`FP64.Polys.length`).
pub(crate) const POLY_COUNT: usize = POLYS.len();

pub(crate) struct Fp64 {
    irred_poly: u64,
    /// `ByteModTable_7`.
    byte_mod_table: Box<[u64; 256]>,
}

impl Fp64 {
    /// `FP64.Init(Polys[index])`. `-fp N` picks the index (validated by the CLI to
    /// `0..POLY_COUNT`); TLC defaults to a random one.
    pub(crate) fn new(poly_index: usize) -> Self {
        let irred_poly = POLYS[poly_index];
        // `t = t * x` in GF(2^64), bit 63 of the Java long being the constant term.
        const ONE: u64 = 0x8000_0000_0000_0000;
        let mut power = [0u64; 72];
        let mut t = ONE;
        for p in &mut power {
            *p = t;
            let mask = if t & 1 != 0 { irred_poly } else { 0 };
            t = (t >> 1) ^ mask;
        }
        let mut byte_mod_table = Box::new([0u64; 256]);
        for (j, slot) in byte_mod_table.iter_mut().enumerate() {
            for k in 0..8 {
                if j & (1 << k) != 0 {
                    *slot ^= power[127 - 7 * 8 - k];
                }
            }
        }
        Fp64 { irred_poly, byte_mod_table }
    }

    /// `FP64.getIrredPoly()`, which `TLCGet("config").fingerprint` prints as a Java long.
    pub(crate) fn irred_poly(&self) -> i64 {
        self.irred_poly as i64
    }

    /// `FP64.New()`.
    pub(crate) fn new_fp(&self) -> Fp {
        Fp(self.irred_poly)
    }

    /// `fp = (fp >>> 8) ^ T7[(b ^ (int) fp) & 0xFF]`.
    pub(crate) fn extend_byte(&self, fp: Fp, b: u8) -> Fp {
        Fp((fp.0 >> 8) ^ self.byte_mod_table[usize::from(b ^ fp.0 as u8)])
    }

    /// 4 bytes, little-endian.
    pub(crate) fn extend_i32(&self, fp: Fp, x: i32) -> Fp {
        x.to_le_bytes().into_iter().fold(fp, |fp, b| self.extend_byte(fp, b))
    }

    /// 8 bytes, little-endian.
    pub(crate) fn extend_i64(&self, fp: Fp, x: i64) -> Fp {
        x.to_le_bytes().into_iter().fold(fp, |fp, b| self.extend_byte(fp, b))
    }

    /// Only the low byte of each UTF-16 unit counts (`(int) c ^ (int) fp`, masked).
    pub(crate) fn extend_str(&self, fp: Fp, s: &str) -> Fp {
        s.encode_utf16().fold(fp, |fp, unit| self.extend_byte(fp, unit as u8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replays `tests/golden/fp64.txt`, produced by TLC's `FP64.java` through
    /// `tools/golden/Fp64Golden.java`.
    #[test]
    fn matches_tlc_fp64() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/fp64.txt");
        let text = std::fs::read_to_string(path).expect("golden file");
        let mut current: Option<(usize, Fp64, Fp)> = None;
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let index: usize = f[0].parse().unwrap();
            let expected = u64::from_str_radix(f[3], 16).unwrap();
            let (_, fp64, fp) = current.get_or_insert_with(|| {
                let fp64 = Fp64::new(index);
                let fp = fp64.new_fp();
                (index, fp64, fp)
            });
            *fp = match f[1] {
                "new" => {
                    *fp64 = Fp64::new(index);
                    fp64.new_fp()
                }
                "byte" => fp64.extend_byte(*fp, f[2].parse::<i8>().unwrap() as u8),
                "int" => fp64.extend_i32(*fp, f[2].parse().unwrap()),
                "long" => fp64.extend_i64(*fp, f[2].parse().unwrap()),
                "str" => {
                    let units: Vec<u16> = f[2].split(',').filter(|u| !u.is_empty()).map(|u| u.parse().unwrap()).collect();
                    fp64.extend_str(*fp, &String::from_utf16(&units).unwrap())
                }
                op => panic!("unknown op {op}"),
            };
            assert_eq!(fp.raw(), expected, "{line}");
        }
        assert_eq!(POLY_COUNT, 131);
    }

    #[test]
    fn fp_key_masks_bit_63() {
        assert_eq!(FpKey::from(Fp(u64::MAX)).raw(), 0x7FFF_FFFF_FFFF_FFFF);
    }
}
