//! The text rules both formats share (CL-016, *Text*): UTF-8 or Latin-1,
//! an optional UTF-8 BOM, LF or CRLF, `#` comments, surrounding
//! whitespace, and numbers with `.` as the decimal mark.

use super::error::{LutError, LutErrorKind, Malformed};

/// Longest piece of file text quoted back in an error.
const QUOTE_CHARS: usize = 32;

/// A LUT file's bytes, split into lines.
pub(crate) struct Text<'a> {
    bytes: &'a [u8],
    utf8: bool,
}

impl<'a> Text<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Text<'a> {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        Text {
            bytes,
            utf8: std::str::from_utf8(bytes).is_ok(),
        }
    }

    /// Every line as `(1-based number, content)`, the content without its
    /// line end, comment and surrounding whitespace (possibly empty).
    pub(crate) fn lines(&self) -> impl Iterator<Item = (u32, &'a [u8])> + 'a {
        // Callers refuse files over `MAX_FILE_BYTES` (600 MiB) first, so the
        // line count stays far below `u32::MAX` and the counter cannot
        // overflow.
        self.bytes
            .split(|&b| b == b'\n')
            .zip(1u32..)
            .map(|(raw, number)| {
                let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
                (number, strip_comment(raw).trim_ascii())
            })
    }

    /// Decodes a piece of the file: as UTF-8 when the whole file is UTF-8,
    /// otherwise as Latin-1 (every byte one character).
    pub(crate) fn decode(&self, piece: &[u8]) -> String {
        if self.utf8 {
            String::from_utf8_lossy(piece).into_owned()
        } else {
            piece.iter().map(|&b| char::from(b)).collect()
        }
    }
}

/// Cuts the line at the first `#` outside double quotes (so a `TITLE` may
/// contain one).
fn strip_comment(line: &[u8]) -> &[u8] {
    let mut quoted = false;
    for (i, &b) in line.iter().enumerate() {
        match b {
            b'"' => quoted = !quoted,
            b'#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

/// The whitespace-separated words of a line.
pub(crate) fn tokens(content: &[u8]) -> impl Iterator<Item = &[u8]> {
    content
        .split(|b| b.is_ascii_whitespace())
        .filter(|t| !t.is_empty())
}

/// A piece of file text for an error message, shortened.
pub(crate) fn quote(token: &[u8]) -> String {
    let text = String::from_utf8_lossy(token);
    let mut out: String = text.chars().take(QUOTE_CHARS).collect();
    if text.chars().nth(QUOTE_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// Whether the token is a number written with `,` as its decimal mark:
/// every comma sits between two digits, and with points instead it is a
/// number.
fn is_decimal_comma(token: &[u8]) -> bool {
    let mut any = false;
    for (i, &b) in token.iter().enumerate() {
        if b == b',' {
            let digit_before = i > 0 && token[i - 1].is_ascii_digit();
            let digit_after = token.get(i + 1).is_some_and(u8::is_ascii_digit);
            if !(digit_before && digit_after) {
                return false;
            }
            any = true;
        }
    }
    if !any {
        return false;
    }
    let pointed: Vec<u8> = token
        .iter()
        .map(|&b| if b == b',' { b'.' } else { b })
        .collect();
    std::str::from_utf8(&pointed).is_ok_and(|s| s.parse::<f32>().is_ok())
}

/// Parses a finite `f32` (optional sign, `.` decimal mark, optional
/// exponent).
pub(crate) fn parse_f32(token: &[u8], line: u32) -> Result<f32, LutError> {
    let parsed = std::str::from_utf8(token)
        .ok()
        .and_then(|s| s.parse::<f32>().ok());
    match parsed {
        Some(v) if v.is_finite() => Ok(v),
        Some(_) => Err(LutError::malformed(
            Some(line),
            Malformed::NotFinite {
                token: quote(token),
            },
        )),
        None => Err(not_a_number(token, line)),
    }
}

/// Parses a non-negative whole number written as plain digits.
pub(crate) fn parse_uint(token: &[u8], line: u32) -> Result<u64, LutError> {
    if token.is_empty() || !token.iter().all(u8::is_ascii_digit) {
        if is_decimal_comma(token) {
            return Err(decimal_comma(line));
        }
        return Err(LutError::malformed(
            Some(line),
            Malformed::NotAnInteger {
                token: quote(token),
            },
        ));
    }
    // Plain digits that overflow `u64` are far beyond any limit; saturate so
    // the caller's range check rejects them.
    Ok(std::str::from_utf8(token)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(u64::MAX))
}

/// The error for a token that is not a number, or a decimal-comma one.
pub(crate) fn not_a_number(token: &[u8], line: u32) -> LutError {
    if is_decimal_comma(token) {
        decimal_comma(line)
    } else {
        LutError::malformed(
            Some(line),
            Malformed::NotANumber {
                token: quote(token),
            },
        )
    }
}

fn decimal_comma(line: u32) -> LutError {
    LutError {
        line: Some(line),
        kind: LutErrorKind::DecimalComma,
    }
}

/// Parses exactly three finite numbers.
pub(crate) fn parse_triple<'t>(
    mut words: impl Iterator<Item = &'t [u8]>,
    line: u32,
) -> Result<[f32; 3], LutError> {
    let mut out = [0.0f32; 3];
    let mut found = 0usize;
    for word in words.by_ref() {
        if found < 3 {
            out[found] = parse_f32(word, line)?;
        }
        found += 1;
    }
    if found == 3 {
        Ok(out)
    } else {
        Err(LutError::malformed(
            Some(line),
            Malformed::ValueCount { found },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_strip_bom_crlf_comments_and_whitespace() {
        let text = Text::new(b"\xEF\xBB\xBFTITLE \"a # b\" # note\r\n\t0 0 0  \r\n\r\n# only\n");
        let lines: Vec<(u32, &[u8])> = text.lines().collect();
        assert_eq!(
            lines,
            vec![
                (1, &b"TITLE \"a # b\""[..]),
                (2, &b"0 0 0"[..]),
                (3, &b""[..]),
                (4, &b""[..]),
                (5, &b""[..]),
            ]
        );
    }

    #[test]
    fn latin1_decodes_byte_per_char() {
        let text = Text::new(b"Caf\xE9");
        assert_eq!(text.decode(b"Caf\xE9"), "Café");
        let text = Text::new("Café".as_bytes());
        assert_eq!(text.decode("Café".as_bytes()), "Café");
    }

    #[test]
    fn numbers() {
        for (s, v) in [
            ("0", 0.0),
            ("-0.5", -0.5),
            ("+1.25", 1.25),
            (".5", 0.5),
            ("5.", 5.0),
            ("1e-3", 0.001),
            ("2.5E+2", 250.0),
        ] {
            assert_eq!(parse_f32(s.as_bytes(), 1).unwrap(), v, "{s}");
        }
        for s in ["nan", "NaN", "inf", "-INF", "infinity", "1e39"] {
            let e = parse_f32(s.as_bytes(), 3).unwrap_err();
            assert!(
                matches!(e.kind, LutErrorKind::Malformed(Malformed::NotFinite { .. })),
                "{s}: {e:?}"
            );
            assert_eq!(e.line, Some(3));
        }
        for s in ["abc", "0.5,", "1,0,0", "0x10", "1..2", ","] {
            let e = parse_f32(s.as_bytes(), 1).unwrap_err();
            assert!(
                matches!(
                    e.kind,
                    LutErrorKind::Malformed(Malformed::NotANumber { .. })
                ),
                "{s}: {e:?}"
            );
        }
        for s in ["0,5", "-0,25", "1,0e-3"] {
            let e = parse_f32(s.as_bytes(), 1).unwrap_err();
            assert_eq!(e.kind, LutErrorKind::DecimalComma, "{s}");
        }
    }

    #[test]
    fn whole_numbers() {
        assert_eq!(parse_uint(b"1023", 1).unwrap(), 1023);
        assert_eq!(parse_uint(b"99999999999999999999999", 1).unwrap(), u64::MAX);
        for s in ["-1", "1.0", "1e3", "+4", ""] {
            let e = parse_uint(s.as_bytes(), 1).unwrap_err();
            assert!(
                matches!(
                    e.kind,
                    LutErrorKind::Malformed(Malformed::NotAnInteger { .. })
                ),
                "{s}: {e:?}"
            );
        }
        assert_eq!(
            parse_uint(b"0,5", 1).unwrap_err().kind,
            LutErrorKind::DecimalComma
        );
    }

    #[test]
    fn quotes_are_shortened() {
        assert_eq!(quote(b"abc"), "abc");
        let long = "x".repeat(40);
        assert_eq!(quote(long.as_bytes()), format!("{}…", "x".repeat(32)));
    }
}
