//! Service credits as fixed-point thousandths (architecture §7.4).
//!
//! The service answers cost checks with JSON numbers (`0.25`, `35`). They are
//! converted from their decimal text with integer math only; amounts finer
//! than a thousandth round **up**, so an estimate is never understated.

use std::fmt;

use serde_json::Value;

/// An amount of the service's credits, in thousandths.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Credits(i64);

/// Why a credit amount could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CreditsError {
    /// Not a non-negative decimal number.
    #[error("not a credit amount: {0:?}")]
    NotANumber(String),
    /// Larger than FreeCut can hold.
    #[error("credit amount out of range: {0}")]
    OutOfRange(String),
}

impl Credits {
    /// Zero credits.
    pub const ZERO: Credits = Credits(0);

    /// An amount in thousandths of a credit.
    pub const fn from_millis(millis: i64) -> Credits {
        Credits(millis)
    }

    /// The amount in thousandths of a credit.
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// Parses non-negative decimal text: `"24"`, `"0.25"`, `"1e-6"`.
    ///
    /// # Errors
    /// [`CreditsError`] for anything else, or amounts beyond `i64` thousandths.
    pub fn parse_decimal(text: &str) -> Result<Credits, CreditsError> {
        let bad = || CreditsError::NotANumber(text.to_owned());
        let (mantissa, exponent) = match text.find(['e', 'E']) {
            Some(i) => {
                let exp: i32 = text[i + 1..].parse().map_err(|_| bad())?;
                (&text[..i], exp)
            }
            None => (text, 0),
        };
        let (int_part, frac_part) = match mantissa.split_once('.') {
            Some((i, f)) => (i, f),
            None => (mantissa, ""),
        };
        if (int_part.is_empty() && frac_part.is_empty())
            || !int_part.bytes().all(|b| b.is_ascii_digit())
            || !frac_part.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(bad());
        }
        let digits = format!("{int_part}{frac_part}");
        let digits = digits.trim_start_matches('0');
        // value = digits × 10^(exponent − frac_len); thousandths shift by 3.
        let frac_len = i32::try_from(frac_part.len()).map_err(|_| bad())?;
        let shift = exponent
            .checked_add(3)
            .and_then(|s| s.checked_sub(frac_len))
            .ok_or_else(|| CreditsError::OutOfRange(text.to_owned()))?;
        if digits.is_empty() {
            return Ok(Credits::ZERO);
        }
        let out_of_range = || CreditsError::OutOfRange(text.to_owned());
        if digits.len() > 36 {
            return Err(out_of_range());
        }
        let n: i128 = digits.parse().map_err(|_| bad())?;
        let millis = if shift >= 0 {
            let factor = 10i128
                .checked_pow(u32::try_from(shift).map_err(|_| out_of_range())?)
                .ok_or_else(out_of_range)?;
            n.checked_mul(factor).ok_or_else(out_of_range)?
        } else {
            let divisor = 10i128.checked_pow(shift.unsigned_abs());
            match divisor {
                // Anything smaller than 10^-38 of a credit rounds up to 1.
                None => 1,
                Some(d) => n.div_euclid(d) + i128::from(n.rem_euclid(d) != 0),
            }
        };
        i64::try_from(millis)
            .map(Credits)
            .map_err(|_| out_of_range())
    }

    /// Reads a JSON number or decimal string.
    ///
    /// # Errors
    /// [`CreditsError`] for other JSON values or unreadable amounts.
    pub fn from_json(value: &Value) -> Result<Credits, CreditsError> {
        match value {
            // serde_json prints the shortest text that reads back as the
            // same number, which is the service's own decimal text.
            Value::Number(n) => Credits::parse_decimal(&n.to_string()),
            Value::String(s) => Credits::parse_decimal(s.trim()),
            other => Err(CreditsError::NotANumber(other.to_string())),
        }
    }

    /// The canonical text with three decimals, as the recipe stores it
    /// (`"24.000"`).
    pub fn to_fixed3(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        format!("{sign}{}.{:03}", abs / 1000, abs % 1000)
    }

    /// Sum, or `None` on overflow.
    pub fn checked_add(self, other: Credits) -> Option<Credits> {
        self.0.checked_add(other.0).map(Credits)
    }
}

/// Shortest text: `24`, `0.25`, `0.001`.
impl fmt::Display for Credits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fixed = self.to_fixed3();
        let trimmed = fixed.trim_end_matches('0').trim_end_matches('.');
        f.write_str(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recorded_cost_answers_parse_exactly() {
        // Recorded 2026-10-14: an image check answered 0.25, a video check 35.
        assert_eq!(
            Credits::from_json(&json!(0.25)),
            Ok(Credits::from_millis(250))
        );
        assert_eq!(
            Credits::from_json(&json!(35)),
            Ok(Credits::from_millis(35_000))
        );
    }

    #[test]
    fn decimal_text_forms() {
        let p = |s: &str| Credits::parse_decimal(s).map(Credits::millis);
        assert_eq!(p("0"), Ok(0));
        assert_eq!(p("24.000"), Ok(24_000));
        assert_eq!(p(".5"), Ok(500));
        assert_eq!(p("7."), Ok(7_000));
        assert_eq!(p("1e3"), Ok(1_000_000));
        assert_eq!(p("2.5E-1"), Ok(250));
        assert_eq!(p("1e-6"), Ok(1));
        assert!(p("").is_err());
        assert!(p("-1").is_err());
        assert!(p("1.2.3").is_err());
        assert!(p("abc").is_err());
        assert!(p("1e400").is_err());
    }

    #[test]
    fn finer_than_a_thousandth_rounds_up() {
        assert_eq!(Credits::parse_decimal("0.0625").unwrap().millis(), 63);
        assert_eq!(Credits::parse_decimal("0.0001").unwrap().millis(), 1);
        assert_eq!(Credits::parse_decimal("1.0000").unwrap().millis(), 1_000);
    }

    #[test]
    fn text_forms() {
        assert_eq!(Credits::from_millis(24_000).to_fixed3(), "24.000");
        assert_eq!(Credits::from_millis(24_000).to_string(), "24");
        assert_eq!(Credits::from_millis(250).to_string(), "0.25");
        assert_eq!(Credits::from_millis(1).to_string(), "0.001");
        assert_eq!(Credits::ZERO.to_string(), "0");
    }

    #[test]
    fn other_json_is_refused() {
        assert!(Credits::from_json(&json!(true)).is_err());
        assert!(Credits::from_json(&json!(-3)).is_err());
        assert_eq!(
            Credits::from_json(&json!("12.5")),
            Ok(Credits::from_millis(12_500))
        );
    }
}
