//! `.cube` (CL-016, *`.cube` keywords* and *`.cube` data*).

use super::error::{LutError, Malformed, Unsupported};
use super::text::{Text, parse_f32, parse_triple, parse_uint, quote, tokens};
use super::{
    Lut, Lut1d, Lut3d, LutNotice, MAX_1D_SIZE, MAX_CUBE_3D_SIZE, MAX_KEPT_3D_SIZE, MAX_TITLE_CHARS,
    RESAMPLED_3D_SIZE, check_len,
};

const TITLE: &str = "TITLE";
const LUT_1D_SIZE: &str = "LUT_1D_SIZE";
const LUT_3D_SIZE: &str = "LUT_3D_SIZE";
const DOMAIN_MIN: &str = "DOMAIN_MIN";
const DOMAIN_MAX: &str = "DOMAIN_MAX";
const LUT_1D_INPUT_RANGE: &str = "LUT_1D_INPUT_RANGE";
const LUT_3D_INPUT_RANGE: &str = "LUT_3D_INPUT_RANGE";

const CHANNELS: [&str; 3] = ["red", "green", "blue"];

/// Parses a `.cube` file: a 1D table, a 3D table, or a 1D shaper followed
/// by a 3D table.
///
/// Every keyword comes before the first data line; unknown upper-case
/// keywords are skipped and counted. Data is red-fastest, as stored. 3D
/// tables larger than [`MAX_KEPT_3D_SIZE`](super::MAX_KEPT_3D_SIZE) are
/// resampled to [`RESAMPLED_3D_SIZE`](super::RESAMPLED_3D_SIZE) with a
/// [`LutNotice::Resampled`].
///
/// # Errors
/// A [`LutError`] naming the line and the rule the file breaks.
pub fn parse_cube(bytes: &[u8]) -> Result<Lut, LutError> {
    check_len(bytes.len())?;
    let text = Text::new(bytes);
    let mut header = Header::default();
    let mut layout: Option<Layout> = None;
    let mut rows: Vec<[f32; 3]> = Vec::new();

    for (line, content) in text.lines() {
        let Some(first) = tokens(content).next() else {
            continue;
        };
        match classify(first) {
            LineKind::Data => {
                let layout = match &layout {
                    Some(layout) => layout,
                    None => {
                        let new = header.finish(Some(line))?;
                        // Bound the reservation by the file: a data line
                        // takes at least 6 bytes ("0 0 0\n").
                        let cap = new.rows.min((bytes.len() / 6) as u64);
                        rows.reserve(usize::try_from(cap).unwrap_or(0));
                        layout.insert(new)
                    }
                };
                if rows.len() as u64 == layout.rows {
                    return Err(LutError::malformed(
                        Some(line),
                        Malformed::TooManyRows {
                            expected: layout.rows,
                        },
                    ));
                }
                rows.push(parse_triple(tokens(content), line)?);
            }
            LineKind::Keyword => {
                if layout.is_some() {
                    return Err(LutError::malformed(
                        Some(line),
                        Malformed::KeywordAfterData {
                            keyword: quote(first),
                        },
                    ));
                }
                header.keyword(first, content, line, &text)?;
            }
            LineKind::Text => {
                return Err(LutError::malformed(
                    Some(line),
                    Malformed::UnexpectedText {
                        text: quote(content),
                    },
                ));
            }
        }
    }

    let layout = match layout {
        Some(layout) => layout,
        None => header.finish(None)?,
    };
    if (rows.len() as u64) < layout.rows {
        return Err(LutError::malformed(
            Some(layout.size_line),
            Malformed::TooFewRows {
                expected: layout.rows,
                found: rows.len() as u64,
            },
        ));
    }
    build(header.title, layout, rows)
}

/// What a non-empty line is, from its first word.
enum LineKind {
    Data,
    Keyword,
    Text,
}

fn classify(first: &[u8]) -> LineKind {
    let starts_number =
        |b: u8| b.is_ascii_digit() || b == b'-' || b == b'+' || b == b'.' || b == b',';
    let is_keyword = first[0].is_ascii_uppercase()
        && first
            .iter()
            .all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
    // `NAN`, `INF` and friends are (non-finite) numbers, not keywords.
    let parses = std::str::from_utf8(first).is_ok_and(|s| s.parse::<f32>().is_ok());
    if parses || starts_number(first[0]) {
        LineKind::Data
    } else if is_keyword {
        LineKind::Keyword
    } else {
        LineKind::Text
    }
}

/// A keyword value and the line it was on.
#[derive(Clone, Copy)]
struct At<T> {
    value: T,
    line: u32,
}

#[derive(Default)]
struct Header {
    title: Option<String>,
    title_line: Option<u32>,
    size_1d: Option<At<u32>>,
    size_3d: Option<At<u32>>,
    domain_min: Option<At<[f32; 3]>>,
    domain_max: Option<At<[f32; 3]>>,
    range_1d: Option<At<[f32; 2]>>,
    range_3d: Option<At<[f32; 2]>>,
    skipped: u32,
}

/// The table shape and domain, fixed by the keywords.
struct Layout {
    size_1d: u32,
    size_3d: u32,
    /// `size_1d + size_3d³`.
    rows: u64,
    /// The size keyword a short table is blamed on (3D if present).
    size_line: u32,
    domain_min: [f32; 3],
    domain_max: [f32; 3],
    skipped: u32,
}

impl Header {
    fn keyword(
        &mut self,
        word: &[u8],
        content: &[u8],
        line: u32,
        text: &Text<'_>,
    ) -> Result<(), LutError> {
        let args = || tokens(content).skip(1);
        match word {
            b"TITLE" => {
                once(self.title_line.is_some(), TITLE, line)?;
                self.title_line = Some(line);
                self.title = parse_title(&content[TITLE.len()..], line, text)?;
            }
            b"LUT_1D_SIZE" => {
                once(self.size_1d.is_some(), LUT_1D_SIZE, line)?;
                let value = parse_size(args(), LUT_1D_SIZE, MAX_1D_SIZE, line)?;
                self.size_1d = Some(At { value, line });
            }
            b"LUT_3D_SIZE" => {
                once(self.size_3d.is_some(), LUT_3D_SIZE, line)?;
                let value = parse_size(args(), LUT_3D_SIZE, MAX_CUBE_3D_SIZE, line)?;
                self.size_3d = Some(At { value, line });
            }
            b"DOMAIN_MIN" | b"DOMAIN_MAX" => {
                let is_min = word == b"DOMAIN_MIN";
                let keyword = if is_min { DOMAIN_MIN } else { DOMAIN_MAX };
                let slot = if is_min {
                    &self.domain_min
                } else {
                    &self.domain_max
                };
                once(slot.is_some(), keyword, line)?;
                if self.range_1d.is_some() || self.range_3d.is_some() {
                    return Err(mixed(line));
                }
                let value = parse_values::<3>(args(), keyword, line)?;
                let at = Some(At { value, line });
                if is_min {
                    self.domain_min = at;
                } else {
                    self.domain_max = at;
                }
            }
            b"LUT_1D_INPUT_RANGE" | b"LUT_3D_INPUT_RANGE" => {
                let is_1d = word == b"LUT_1D_INPUT_RANGE";
                let keyword = if is_1d {
                    LUT_1D_INPUT_RANGE
                } else {
                    LUT_3D_INPUT_RANGE
                };
                let slot = if is_1d {
                    &self.range_1d
                } else {
                    &self.range_3d
                };
                once(slot.is_some(), keyword, line)?;
                if self.domain_min.is_some() || self.domain_max.is_some() {
                    return Err(mixed(line));
                }
                let value = parse_values::<2>(args(), keyword, line)?;
                if value[1] <= value[0] {
                    return Err(LutError::malformed(
                        Some(line),
                        Malformed::EmptyDomain {
                            keyword,
                            channel: None,
                        },
                    ));
                }
                let at = Some(At { value, line });
                if is_1d {
                    self.range_1d = at;
                } else {
                    self.range_3d = at;
                }
            }
            _ => self.skipped = self.skipped.saturating_add(1),
        }
        Ok(())
    }

    /// Checks the keywords together once the data starts (or the file
    /// ends, `first_data` = `None`).
    fn finish(&self, first_data: Option<u32>) -> Result<Layout, LutError> {
        let size_1d = self.size_1d.map_or(0, |s| s.value);
        let size_3d = self.size_3d.map_or(0, |s| s.value);
        let Some(size_line) = self.size_3d.or(self.size_1d).map(|s| s.line) else {
            return Err(LutError::malformed(first_data, Malformed::MissingSize));
        };
        for (range, keyword, has_table, table) in [
            (self.range_1d, LUT_1D_INPUT_RANGE, size_1d > 0, "1D"),
            (self.range_3d, LUT_3D_INPUT_RANGE, size_3d > 0, "3D"),
        ] {
            if let Some(range) = range
                && !has_table
            {
                return Err(LutError::malformed(
                    Some(range.line),
                    Malformed::RangeWithoutTable { keyword, table },
                ));
            }
        }

        let (domain_min, domain_max) = if self.range_1d.is_some() || self.range_3d.is_some() {
            // Input-range form: the shaper's range is the file's domain; a
            // 3D table after a shaper must take the shaper's 0–1 output.
            let range = if size_1d > 0 {
                if let Some(r3) = self.range_3d
                    && size_3d > 0
                    && r3.value != [0.0, 1.0]
                {
                    return Err(LutError::unsupported(
                        Some(r3.line),
                        Unsupported::ShaperInputRange,
                    ));
                }
                self.range_1d
            } else {
                self.range_3d
            };
            let [lo, hi] = range.map_or([0.0, 1.0], |r| r.value);
            ([lo; 3], [hi; 3])
        } else {
            let lo = self.domain_min.map_or([0.0; 3], |d| d.value);
            let hi = self.domain_max.map_or([1.0; 3], |d| d.value);
            if let Some(c) = (0..3).find(|&c| hi[c] <= lo[c]) {
                // Blame the later of the two lines (the one present, when
                // only one is given).
                let (keyword, line) = match (self.domain_min, self.domain_max) {
                    (Some(min), Some(max)) if min.line > max.line => (DOMAIN_MIN, min.line),
                    (_, Some(max)) => (DOMAIN_MAX, max.line),
                    (Some(min), None) => (DOMAIN_MIN, min.line),
                    (None, None) => (DOMAIN_MAX, size_line),
                };
                return Err(LutError::malformed(
                    Some(line),
                    Malformed::EmptyDomain {
                        keyword,
                        channel: Some(CHANNELS[c]),
                    },
                ));
            }
            (lo, hi)
        };

        let n3 = u64::from(size_3d);
        Ok(Layout {
            size_1d,
            size_3d,
            rows: u64::from(size_1d) + n3 * n3 * n3,
            size_line,
            domain_min,
            domain_max,
            skipped: self.skipped,
        })
    }
}

/// Splits the rows into the shaper and the 3D table, resampling a large one.
fn build(title: Option<String>, layout: Layout, mut rows: Vec<[f32; 3]>) -> Result<Lut, LutError> {
    let cube_rows = rows.split_off(layout.size_1d as usize);
    // Sizes are range-checked by `parse_size` and the row count by the
    // caller, so the constructors cannot refuse; an impossible refusal maps
    // to "too few rows" rather than a panic.
    let short = || {
        LutError::malformed(
            Some(layout.size_line),
            Malformed::TooFewRows {
                expected: layout.rows,
                found: 0,
            },
        )
    };
    let shaper = match layout.size_1d {
        0 => None,
        _ => Some(Lut1d::new(rows).ok_or_else(short)?),
    };
    let mut notices = Vec::new();
    let cube = match layout.size_3d {
        0 => None,
        n => {
            let table = Lut3d::new(n, cube_rows).ok_or_else(short)?;
            if n > MAX_KEPT_3D_SIZE {
                notices.push(LutNotice::Resampled {
                    from: n,
                    to: RESAMPLED_3D_SIZE,
                });
                Some(table.resampled(RESAMPLED_3D_SIZE).ok_or_else(short)?)
            } else {
                Some(table)
            }
        }
    };
    Ok(Lut {
        title,
        shaper,
        cube,
        domain_min: layout.domain_min,
        domain_max: layout.domain_max,
        skipped_keywords: layout.skipped,
        notices,
    })
}

fn once(seen: bool, keyword: &'static str, line: u32) -> Result<(), LutError> {
    if seen {
        Err(LutError::malformed(
            Some(line),
            Malformed::DuplicateKeyword { keyword },
        ))
    } else {
        Ok(())
    }
}

fn mixed(line: u32) -> LutError {
    LutError::unsupported(Some(line), Unsupported::MixedDomainKeywords)
}

/// `TITLE "…"`: the text between the first and last quote, at most
/// [`MAX_TITLE_CHARS`] characters; blank becomes `None`.
fn parse_title(rest: &[u8], line: u32, text: &Text<'_>) -> Result<Option<String>, LutError> {
    let rest = rest.trim_ascii();
    let quoted = rest.len() >= 2 && rest.starts_with(b"\"") && rest.ends_with(b"\"");
    if !quoted {
        return Err(LutError::malformed(Some(line), Malformed::TitleNotQuoted));
    }
    let title = text.decode(&rest[1..rest.len() - 1]);
    let chars = title.chars().count();
    if chars > MAX_TITLE_CHARS {
        return Err(LutError::malformed(
            Some(line),
            Malformed::TitleTooLong { chars },
        ));
    }
    let title = title.trim();
    Ok((!title.is_empty()).then(|| title.to_owned()))
}

fn parse_size<'t>(
    args: impl Iterator<Item = &'t [u8]>,
    keyword: &'static str,
    max: u32,
    line: u32,
) -> Result<u32, LutError> {
    let args: Vec<&[u8]> = args.collect();
    let &[token] = args.as_slice() else {
        return Err(LutError::malformed(
            Some(line),
            Malformed::ArgumentCount {
                keyword,
                expected: 1,
                found: args.len(),
            },
        ));
    };
    let size = parse_uint(token, line).map_err(|e| match e.kind {
        super::LutErrorKind::Malformed(Malformed::NotAnInteger { token }) => {
            LutError::malformed(Some(line), Malformed::SizeNotInteger { keyword, token })
        }
        _ => e,
    })?;
    match u32::try_from(size) {
        Ok(size) if (2..=max).contains(&size) => Ok(size),
        _ => Err(LutError::malformed(
            Some(line),
            Malformed::SizeOutOfRange {
                keyword,
                size,
                min: 2,
                max,
            },
        )),
    }
}

fn parse_values<'t, const N: usize>(
    args: impl Iterator<Item = &'t [u8]>,
    keyword: &'static str,
    line: u32,
) -> Result<[f32; N], LutError> {
    let args: Vec<&[u8]> = args.collect();
    if args.len() != N {
        return Err(LutError::malformed(
            Some(line),
            Malformed::ArgumentCount {
                keyword,
                expected: N,
                found: args.len(),
            },
        ));
    }
    let mut out = [0.0; N];
    for (slot, token) in out.iter_mut().zip(args) {
        *slot = parse_f32(token, line)?;
    }
    Ok(out)
}
