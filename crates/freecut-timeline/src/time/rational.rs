//! Exact dimensionless ratios (speed factors, aspect ratios).

use super::TimeError;

/// A reduced fraction with a positive denominator.
///
/// There is deliberately no constructor from `f64`: ratios come from integers
/// or exact decimal strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rational {
    num: i64,
    den: i64,
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Rational {
    pub const ONE: Rational = Rational { num: 1, den: 1 };

    pub fn new(num: i64, den: i64) -> Result<Rational, TimeError> {
        if den == 0 || num == i64::MIN || den == i64::MIN {
            return Err(TimeError::InvalidRational { num, den });
        }
        let g = gcd(num.unsigned_abs(), den.unsigned_abs());
        // g divides both magnitudes and both fit in i64 (MIN excluded above).
        let g = i64::try_from(g).map_err(|_| TimeError::InvalidRational { num, den })?;
        let (mut num, mut den) = (num / g, den / g);
        if den < 0 {
            (num, den) = (-num, -den);
        }
        Ok(Rational { num, den })
    }

    pub fn from_int(v: i64) -> Result<Rational, TimeError> {
        Rational::new(v, 1)
    }

    /// Parses an exact decimal such as `"1.37"` or `"-0.5"` (no exponent).
    pub fn parse_decimal(text: &str) -> Result<Rational, TimeError> {
        let invalid = || TimeError::InvalidRational { num: 0, den: 0 };
        let text = text.trim();
        let (neg, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let (int_part, frac_part) = digits.split_once('.').unwrap_or((digits, ""));
        if int_part.is_empty() && frac_part.is_empty() {
            return Err(invalid());
        }
        if !int_part.bytes().all(|b| b.is_ascii_digit())
            || !frac_part.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid());
        }
        let scale = 10_i64
            .checked_pow(u32::try_from(frac_part.len()).map_err(|_| invalid())?)
            .ok_or(TimeError::Overflow)?;
        let int_val: i64 = if int_part.is_empty() {
            0
        } else {
            int_part.parse().map_err(|_| TimeError::Overflow)?
        };
        let frac_val: i64 = if frac_part.is_empty() {
            0
        } else {
            frac_part.parse().map_err(|_| TimeError::Overflow)?
        };
        let mag = int_val
            .checked_mul(scale)
            .and_then(|v| v.checked_add(frac_val))
            .ok_or(TimeError::Overflow)?;
        Rational::new(if neg { -mag } else { mag }, scale)
    }

    pub const fn num(self) -> i64 {
        self.num
    }

    pub const fn den(self) -> i64 {
        self.den
    }

    pub const fn is_positive(self) -> bool {
        self.num > 0
    }

    pub fn recip(self) -> Result<Rational, TimeError> {
        Rational::new(self.den, self.num)
    }

    pub fn checked_mul(self, other: Rational) -> Result<Rational, TimeError> {
        let num = i128::from(self.num) * i128::from(other.num);
        let den = i128::from(self.den) * i128::from(other.den);
        Self::from_i128(num, den)
    }

    pub fn checked_div(self, other: Rational) -> Result<Rational, TimeError> {
        self.checked_mul(other.recip()?)
    }

    fn from_i128(num: i128, den: i128) -> Result<Rational, TimeError> {
        let g = {
            let (mut a, mut b) = (num.unsigned_abs(), den.unsigned_abs());
            while b != 0 {
                (a, b) = (b, a % b);
            }
            a.max(1)
        };
        let g = i128::try_from(g).map_err(|_| TimeError::Overflow)?;
        let num = i64::try_from(num / g).map_err(|_| TimeError::Overflow)?;
        let den = i64::try_from(den / g).map_err(|_| TimeError::Overflow)?;
        Rational::new(num, den)
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    /// Cross-multiplied in `i128`; denominators are positive so the order holds.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        (i128::from(self.num) * i128::from(other.den))
            .cmp(&(i128::from(other.num) * i128::from(self.den)))
    }
}

impl core::fmt::Display for Rational {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_reduces_and_normalizes_sign() {
        let r = Rational::new(6, -4).unwrap();
        assert_eq!((r.num(), r.den()), (-3, 2));
        assert!(Rational::new(1, 0).is_err());
        assert!(Rational::new(i64::MIN, 1).is_err());
    }

    #[test]
    fn parse_decimal_is_exact() {
        assert_eq!(
            Rational::parse_decimal("1.37"),
            Ok(Rational::new(137, 100).unwrap())
        );
        assert_eq!(
            Rational::parse_decimal("29.97"),
            Ok(Rational::new(2997, 100).unwrap())
        );
        assert_eq!(
            Rational::parse_decimal("-0.5"),
            Ok(Rational::new(-1, 2).unwrap())
        );
        assert_eq!(
            Rational::parse_decimal("2"),
            Ok(Rational::ONE
                .checked_mul(Rational::from_int(2).unwrap())
                .unwrap())
        );
        assert!(Rational::parse_decimal("").is_err());
        assert!(Rational::parse_decimal("1e3").is_err());
        assert!(Rational::parse_decimal(".").is_err());
    }

    #[test]
    fn ordering_uses_cross_multiplication() {
        let a = Rational::new(1, 3).unwrap();
        let b = Rational::new(333_333, 1_000_000).unwrap();
        assert!(a > b);
        assert!(Rational::new(-1, 2).unwrap() < Rational::new(1, 3).unwrap());
    }

    #[test]
    fn mul_and_div_reduce() {
        let a = Rational::new(2, 3).unwrap();
        let b = Rational::new(3, 4).unwrap();
        assert_eq!(a.checked_mul(b), Ok(Rational::new(1, 2).unwrap()));
        assert_eq!(a.checked_div(b), Ok(Rational::new(8, 9).unwrap()));
        assert!(Rational::new(0, 1).unwrap().recip().is_err());
    }
}
