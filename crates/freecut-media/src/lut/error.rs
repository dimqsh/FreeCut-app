//! Why a LUT file was rejected (CL-016, *Errors*): one line number and one
//! reason, with every validation rule its own variant so callers and tests
//! can match on it and the app can word it.

use std::fmt;

use super::{MAX_3DL_MESH, MAX_FILE_BYTES, MAX_TITLE_CHARS};

/// A rejected LUT file: where (1-based line, when one line is to blame) and
/// why.
///
/// Displays as `line 12: expected 3 numbers, found 2`, or just the reason
/// when no single line is at fault.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}{kind}", LinePrefix(*line))]
pub struct LutError {
    /// The 1-based line of the file the problem was found on (a byte-order
    /// mark is part of line 1). `None` for whole-file problems such as a
    /// file that is too large or holds no table at all.
    pub line: Option<u32>,
    /// The reason.
    pub kind: LutErrorKind,
}

impl LutError {
    pub(crate) fn malformed(line: Option<u32>, reason: Malformed) -> LutError {
        LutError {
            line,
            kind: LutErrorKind::Malformed(reason),
        }
    }

    pub(crate) fn unsupported(line: Option<u32>, reason: Unsupported) -> LutError {
        LutError {
            line,
            kind: LutErrorKind::Unsupported(reason),
        }
    }
}

/// The class of a rejection. Malformed and unsupported files are reported
/// separately (CL-016): a malformed file breaks the format's rules, an
/// unsupported one is valid but uses something FreeCut does not handle.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LutErrorKind {
    /// The file breaks a rule of the format.
    #[error("{0}")]
    Malformed(Malformed),
    /// The file is valid but uses a feature FreeCut does not support.
    #[error("unsupported LUT: {0}")]
    Unsupported(Unsupported),
    /// The file is over [`MAX_FILE_BYTES`] and was refused before parsing.
    #[error(
        "the file is larger than {} MB, the most FreeCut reads as a LUT",
        MAX_FILE_BYTES / (1024 * 1024)
    )]
    TooLarge,
    /// A number uses `,` as its decimal mark (`0,5`). FreeCut never guesses
    /// the intended value.
    #[error("numbers use a comma as the decimal mark; LUT files need a point (0.5, not 0,5)")]
    DecimalComma,
}

/// A rule of the `.cube` or `.3dl` format that the file breaks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Malformed {
    /// A `.cube` line that is neither a keyword nor a row of numbers (for
    /// example lower-case words).
    #[error("expected a keyword or 3 numbers, found \"{text}\"")]
    UnexpectedText {
        /// The start of the line.
        text: String,
    },
    /// A value that is not a number.
    #[error("\"{token}\" is not a number")]
    NotANumber {
        /// The offending text.
        token: String,
    },
    /// A value that is NaN or infinite (also: a number too large for 32-bit
    /// floats).
    #[error("\"{token}\" is not a finite number")]
    NotFinite {
        /// The offending text.
        token: String,
    },
    /// A `.3dl` mesh or data value that is not a non-negative whole number.
    #[error("\"{token}\" is not a whole number")]
    NotAnInteger {
        /// The offending text.
        token: String,
    },
    /// A data line without exactly three values.
    #[error("expected 3 numbers, found {found}")]
    ValueCount {
        /// How many values the line holds.
        found: usize,
    },
    /// A `.cube` keyword after the first data line.
    #[error("the keyword {keyword} comes after the table data; keywords must come first")]
    KeywordAfterData {
        /// The keyword as written.
        keyword: String,
    },
    /// A `.cube` keyword given twice.
    #[error("{keyword} is given more than once")]
    DuplicateKeyword {
        /// The keyword.
        keyword: &'static str,
    },
    /// A `.cube` keyword with the wrong number of values after it.
    #[error("{keyword} takes {expected} values, found {found}")]
    ArgumentCount {
        /// The keyword.
        keyword: &'static str,
        /// How many values it takes.
        expected: usize,
        /// How many the line holds.
        found: usize,
    },
    /// `TITLE` without its text in double quotes.
    #[error("TITLE needs its text in double quotes")]
    TitleNotQuoted,
    /// A `TITLE` longer than [`MAX_TITLE_CHARS`] characters.
    #[error("TITLE is {chars} characters long; at most {MAX_TITLE_CHARS} are allowed")]
    TitleTooLong {
        /// The title's length in characters.
        chars: usize,
    },
    /// A size keyword whose value is not a whole number.
    #[error("{keyword} needs a whole number, found \"{token}\"")]
    SizeNotInteger {
        /// `LUT_1D_SIZE` or `LUT_3D_SIZE`.
        keyword: &'static str,
        /// The offending text.
        token: String,
    },
    /// A size outside `LUT_1D_SIZE` 2..=[`MAX_1D_SIZE`](super::MAX_1D_SIZE)
    /// or `LUT_3D_SIZE` 2..=[`MAX_CUBE_3D_SIZE`](super::MAX_CUBE_3D_SIZE).
    #[error("{keyword} {size} is outside {min}–{max}")]
    SizeOutOfRange {
        /// `LUT_1D_SIZE` or `LUT_3D_SIZE`.
        keyword: &'static str,
        /// The size given.
        size: u64,
        /// Smallest allowed size.
        min: u32,
        /// Largest allowed size.
        max: u32,
    },
    /// Table data before any `LUT_1D_SIZE` or `LUT_3D_SIZE`, or a file with
    /// neither keyword.
    #[error("no LUT_1D_SIZE or LUT_3D_SIZE before the table data")]
    MissingSize,
    /// A domain whose maximum does not exceed its minimum.
    #[error("{keyword}: the maximum must be above the minimum{}", ChannelSuffix(*channel))]
    EmptyDomain {
        /// `DOMAIN_MAX` (or `DOMAIN_MIN` when only it is given),
        /// `LUT_1D_INPUT_RANGE` or `LUT_3D_INPUT_RANGE`.
        keyword: &'static str,
        /// `"red"`, `"green"` or `"blue"` for `DOMAIN_*`; `None` for the
        /// input-range keywords, which cover all three.
        channel: Option<&'static str>,
    },
    /// An input-range keyword for a table the file does not have.
    #[error("{keyword} is given but the file has no {table} table")]
    RangeWithoutTable {
        /// `LUT_1D_INPUT_RANGE` or `LUT_3D_INPUT_RANGE`.
        keyword: &'static str,
        /// `"1D"` or `"3D"`.
        table: &'static str,
    },
    /// Fewer `.cube` data lines than the sizes declare.
    #[error("{} data lines expected, {} found", Grouped(*expected), Grouped(*found))]
    TooFewRows {
        /// `LUT_1D_SIZE + LUT_3D_SIZE³`.
        expected: u64,
        /// Data lines in the file.
        found: u64,
    },
    /// More `.cube` data lines than the sizes declare; the line is the
    /// first one too many.
    #[error("more data lines than the {} the sizes declare", Grouped(*expected))]
    TooManyRows {
        /// `LUT_1D_SIZE + LUT_3D_SIZE³`.
        expected: u64,
    },
    /// A `.3dl` file with no mesh line (empty, or only comments).
    #[error("no input mesh line")]
    MissingMesh,
    /// A `.3dl` mesh with fewer than 2 or more than [`MAX_3DL_MESH`] values.
    #[error("the input mesh has {found} values; 2–{MAX_3DL_MESH} are allowed")]
    MeshSize {
        /// Values on the mesh line.
        found: usize,
    },
    /// A `.3dl` mesh that does not start at 0.
    #[error("the input mesh starts at {first}; it must start at 0")]
    MeshStart {
        /// The first mesh value.
        first: u64,
    },
    /// A `.3dl` mesh that is not strictly increasing.
    #[error("the input mesh is not increasing at value {position}")]
    MeshNotIncreasing {
        /// 1-based position of the first value not above the one before it.
        position: usize,
    },
    /// A `.3dl` integer value above 65535 (the 16-bit maximum).
    #[error("{value} is above 65535, the largest 16-bit value")]
    ValueTooLarge {
        /// The value.
        value: u64,
    },
}

/// A valid feature of a LUT file that FreeCut does not support.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unsupported {
    /// A `.cube` file mixing `DOMAIN_MIN`/`DOMAIN_MAX` with the older
    /// `LUT_1D_INPUT_RANGE`/`LUT_3D_INPUT_RANGE`.
    #[error("DOMAIN_MIN/DOMAIN_MAX mixed with an *_INPUT_RANGE keyword")]
    MixedDomainKeywords,
    /// A shaper `.cube` (1D then 3D) whose `LUT_3D_INPUT_RANGE` is not
    /// 0–1, so the 3D table would need its own domain.
    #[error("a LUT_3D_INPUT_RANGE other than 0 1 after a 1D shaper")]
    ShaperInputRange,
    /// A `.3dl` header keyword line (such as `3DMESH`) where the mesh line
    /// belongs.
    #[error("the .3dl header \"{keyword}\"; FreeCut reads files that start with the input mesh")]
    MeshHeader {
        /// The first word of the line.
        keyword: String,
    },
    /// A `.3dl` mesh whose last value is not 1023, 4095 or 65535.
    #[error("an input mesh ending at {last}; it must end at 1023, 4095 or 65535")]
    MeshDepth {
        /// The last mesh value.
        last: u64,
    },
    /// A `.3dl` file whose data does not match its mesh (`points³` lines).
    #[error(
        "the input mesh has {points} points, so {} data lines are expected; {} found",
        Grouped(*expected),
        Grouped(*found)
    )]
    MeshMismatch {
        /// Values on the mesh line (the cube size).
        points: u32,
        /// `points³`.
        expected: u64,
        /// Data lines in the file.
        found: u64,
    },
}

/// `"line 12: "` or nothing.
struct LinePrefix(Option<u32>);

impl fmt::Display for LinePrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(line) => write!(f, "line {line}: "),
            None => Ok(()),
        }
    }
}

/// `" (green)"` or nothing.
struct ChannelSuffix(Option<&'static str>);

impl fmt::Display for ChannelSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(channel) => write!(f, " on the {channel} channel"),
            None => Ok(()),
        }
    }
}

/// A count with thin groups of three digits: `35 937`.
struct Grouped(u64);

impl fmt::Display for Grouped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits = self.0.to_string();
        let first = digits.len() % 3;
        let mut out = String::with_capacity(digits.len() + digits.len() / 3);
        for (i, c) in digits.chars().enumerate() {
            if i != 0 && (i + 3 - first).is_multiple_of(3) {
                out.push(' ');
            }
            out.push(c);
        }
        f.write_str(&out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_digits_in_threes() {
        assert_eq!(Grouped(0).to_string(), "0");
        assert_eq!(Grouped(999).to_string(), "999");
        assert_eq!(Grouped(4913).to_string(), "4 913");
        assert_eq!(Grouped(35_937).to_string(), "35 937");
        assert_eq!(Grouped(274_625).to_string(), "274 625");
        assert_eq!(Grouped(16_777_216).to_string(), "16 777 216");
    }

    #[test]
    fn display_names_line_and_reason() {
        let e = LutError::malformed(Some(12), Malformed::ValueCount { found: 2 });
        assert_eq!(e.to_string(), "line 12: expected 3 numbers, found 2");
        let e = LutError::malformed(
            None,
            Malformed::TooFewRows {
                expected: 35_937,
                found: 35_936,
            },
        );
        assert_eq!(e.to_string(), "35 937 data lines expected, 35 936 found");
        let e = LutError::malformed(
            Some(4),
            Malformed::EmptyDomain {
                keyword: "DOMAIN_MAX",
                channel: Some("green"),
            },
        );
        assert_eq!(
            e.to_string(),
            "line 4: DOMAIN_MAX: the maximum must be above the minimum on the green channel"
        );
        let e = LutError::unsupported(Some(1), Unsupported::MeshDepth { last: 255 });
        assert_eq!(
            e.to_string(),
            "line 1: unsupported LUT: an input mesh ending at 255; it must end at 1023, 4095 or 65535"
        );
    }
}
