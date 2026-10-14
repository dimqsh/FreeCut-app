//! The LUT slot of a colour stack (CL-016, CL-017) and skin protection
//! (CL-030). Plain values: the file behind a [`LutRef`] is found, read and
//! parsed by other crates (the app's library and `freecut-media::lut`); the
//! timeline only names it.

/// A LUT file, named by the XXH3-128 hash of its bytes (CL-016's model
/// notes) plus the file name it had when it was chosen, for the missing
/// badge and relinking.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LutRef {
    pub hash: [u8; 16],
    pub file_name: String,
}

impl LutRef {
    pub fn new(hash: [u8; 16], file_name: impl Into<String>) -> LutRef {
        LutRef {
            hash,
            file_name: file_name.into(),
        }
    }

    /// The hash as 32 lower-case hex digits (how project files store it).
    pub fn hex(&self) -> String {
        hash_hex(&self.hash)
    }
}

/// `hash` as 32 lower-case hex digits.
pub fn hash_hex(hash: &[u8; 16]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(32);
    for b in hash {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0xf)]));
    }
    out
}

/// The hash written by [`hash_hex`]; `None` unless `s` is exactly 32 hex
/// digits (either case).
pub fn parse_hash_hex(s: &str) -> Option<[u8; 16]> {
    let bytes = s.as_bytes();
    if bytes.len() != 32 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let mut out = [0u8; 16];
    for (i, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        out[i] = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    Some(out)
}

/// How a LUT is sampled between its lattice points (CL-017; named apart
/// from the keyframe `Interp`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LutInterp {
    #[default]
    Tetrahedral,
    Trilinear,
}

impl LutInterp {
    pub const ALL: [LutInterp; 2] = [LutInterp::Tetrahedral, LutInterp::Trilinear];

    /// The stable name in project files.
    pub const fn as_str(self) -> &'static str {
        match self {
            LutInterp::Tetrahedral => "tetrahedral",
            LutInterp::Trilinear => "trilinear",
        }
    }

    pub fn parse(s: &str) -> Option<LutInterp> {
        LutInterp::ALL.into_iter().find(|i| i.as_str() == s)
    }
}

/// Where skin protection applies (CL-030): the Color section (white
/// balance, hue, saturation, vibrance and HSL) or the LUT section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkinScope {
    Color,
    Lut,
}

impl SkinScope {
    pub const ALL: [SkinScope; 2] = [SkinScope::Color, SkinScope::Lut];

    /// The stable name in project files.
    pub const fn as_str(self) -> &'static str {
        match self {
            SkinScope::Color => "color",
            SkinScope::Lut => "lut",
        }
    }
}

/// Skin protection's default Amount (CL-030, FreeCut default).
pub const SKIN_AMOUNT_DEFAULT: u8 = 70;

/// One scope's skin protection (CL-030): its switch (off by default) and
/// Amount, 0–100 %, 70 by default. The Amount is kept while the switch is
/// off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SkinProtect {
    pub on: bool,
    pub amount: u8,
}

impl Default for SkinProtect {
    fn default() -> Self {
        SkinProtect {
            on: false,
            amount: SKIN_AMOUNT_DEFAULT,
        }
    }
}

impl SkinProtect {
    /// Whether it protects anything: on with an Amount above 0.
    pub const fn is_active(self) -> bool {
        self.on && self.amount > 0
    }
}

/// The user LUT of a colour stack (CL-017): the file, its Intensity
/// (0–100 %, 100 by default, keyable), how it is sampled and its skin
/// protection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LutSlot {
    pub lut: LutRef,
    pub intensity: u8,
    pub interp: LutInterp,
    pub skin: SkinProtect,
}

impl LutSlot {
    /// `lut` at full intensity with the default sampling and no skin
    /// protection.
    pub fn new(lut: LutRef) -> LutSlot {
        LutSlot {
            lut,
            intensity: 100,
            interp: LutInterp::default(),
            skin: SkinProtect::default(),
        }
    }

    /// The slot with its settings back at their defaults and the same file
    /// (CL-017: a section reset keeps the LUT choice).
    #[must_use]
    pub fn reset(&self) -> LutSlot {
        LutSlot::new(self.lut.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_round_trip_through_hex() {
        let h: [u8; 16] = core::array::from_fn(|i| (i as u8).wrapping_mul(37) ^ 0xa5);
        let hex = hash_hex(&h);
        assert_eq!(hex.len(), 32);
        assert_eq!(parse_hash_hex(&hex), Some(h));
        assert_eq!(parse_hash_hex(&hex.to_uppercase()), Some(h));
        assert_eq!(parse_hash_hex(&hex[1..]), None);
        assert_eq!(parse_hash_hex(&format!("{}g", &hex[1..])), None);
        assert_eq!(LutRef::new(h, "a.cube").hex(), hex);
    }

    #[test]
    fn defaults_are_cl_017_and_cl_030() {
        let s = LutSlot::new(LutRef::new([1; 16], "x.cube"));
        assert_eq!((s.intensity, s.interp), (100, LutInterp::Tetrahedral));
        assert_eq!(
            s.skin,
            SkinProtect {
                on: false,
                amount: 70
            }
        );
        assert!(!s.skin.is_active());
        let changed = LutSlot {
            intensity: 30,
            interp: LutInterp::Trilinear,
            skin: SkinProtect {
                on: true,
                amount: 5,
            },
            ..s.clone()
        };
        assert!(changed.skin.is_active());
        assert_eq!(changed.reset(), s);
        for i in LutInterp::ALL {
            assert_eq!(LutInterp::parse(i.as_str()), Some(i));
        }
    }
}
