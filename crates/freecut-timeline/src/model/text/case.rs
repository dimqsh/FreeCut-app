//! Case mapping (TX-008), shared by the display transform (the renderer maps
//! the content before layout) and **Change case** (which rewrites stored
//! content, see `edit::change_case`).
//!
//! The mappings are Unicode's default full case mappings (so `ß` becomes
//! `SS` and lengths may change), with these additions:
//! - Turkish and Azerbaijani (`tr`, `az` language tags): dotted and dotless
//!   i map as `i ↔ İ` and `ı ↔ I`.
//! - Greek capital sigma lowercases to final `ς` at the end of a word.
//! - Title and Sentence case use Unicode's titlecase mapping where it
//!   differs from the uppercase one (`ǆ` → `ǅ`, `ﬁ` → `Fi`, `ᾀ` → `ᾈ`,
//!   Georgian letters stay as they are).
//!
//! Words and sentences (FreeCut defaults):
//! - A *word* starts at a letter that follows anything but a letter, digit
//!   or combining mark; an apostrophe between letters stays inside the word
//!   ("don't" → "Don't"). Title case capitalizes every word start, with no
//!   small-word exceptions, and lowercases the rest.
//! - A *sentence* ends at `.`, `!`, `?`, `…`, `‽`, `。`, `！` or `？`,
//!   optionally followed by closing quotes or brackets, then whitespace.
//!   Sentence case capitalizes the first letter of the text and the first
//!   letter after each sentence end, and lowercases the rest. Abbreviations
//!   ("e.g. this") count as sentence ends. Scripts without case are
//!   unchanged by every mapping.

use std::borrow::Cow;
use std::ops::Range;

use super::LangTag;
use super::style::CaseTransform;

/// `text` as displayed under `case` (TX-008), borrowed when nothing changes.
pub fn display<'a>(text: &'a str, case: CaseTransform, lang: Option<&LangTag>) -> Cow<'a, str> {
    if case == CaseTransform::AsTyped {
        return Cow::Borrowed(text);
    }
    let mapped = map_range(text, 0..text.len(), case, lang);
    if mapped == text {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(mapped)
    }
}

/// `text` with only the characters in the byte range `range` mapped to
/// `case`; word and sentence starts are judged in the whole text. The range
/// is widened to character boundaries and clipped to the text. An empty
/// range and `AsTyped` change nothing.
pub fn change_range(
    text: &str,
    range: Range<usize>,
    case: CaseTransform,
    lang: Option<&LangTag>,
) -> String {
    if range.start >= range.end {
        return text.to_owned();
    }
    let mut start = range.start.min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = range.end.clamp(start, text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    map_range(text, start..end, case, lang)
}

fn map_range(
    text: &str,
    range: Range<usize>,
    case: CaseTransform,
    lang: Option<&LangTag>,
) -> String {
    let turkic = lang.is_some_and(LangTag::uses_turkic_casing);
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = String::with_capacity(text.len());
    let mut sentence = SentenceState::Start;
    for (i, &(at, c)) in chars.iter().enumerate() {
        let start_case = match case {
            CaseTransform::AsTyped | CaseTransform::Upper | CaseTransform::Lower => false,
            CaseTransform::Title => c.is_alphabetic() && is_word_start(&chars[..i]),
            CaseTransform::Sentence => sentence == SentenceState::Start && c.is_alphabetic(),
        };
        if case == CaseTransform::Sentence {
            sentence = sentence.next(c);
        }
        if !range.contains(&at) {
            out.push(c);
            continue;
        }
        match case {
            CaseTransform::AsTyped => out.push(c),
            CaseTransform::Upper => push_upper(&mut out, c, turkic),
            CaseTransform::Lower => push_lower(&mut out, &chars, i, turkic),
            CaseTransform::Title | CaseTransform::Sentence => {
                if start_case {
                    push_title(&mut out, c, turkic);
                } else {
                    push_lower(&mut out, &chars, i, turkic);
                }
            }
        }
    }
    out
}

/// Where sentence case is in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SentenceState {
    /// The next letter starts a sentence.
    Start,
    /// Inside a sentence.
    Inside,
    /// After a sentence-ending mark (and closing punctuation): whitespace
    /// starts the next sentence, anything else continues this one.
    Ended,
}

impl SentenceState {
    fn next(self, c: char) -> SentenceState {
        let ends = matches!(c, '.' | '!' | '?' | '…' | '‽' | '。' | '！' | '？');
        match self {
            SentenceState::Start if c.is_alphanumeric() => SentenceState::Inside,
            SentenceState::Start => SentenceState::Start,
            _ if ends => SentenceState::Ended,
            SentenceState::Ended if c.is_whitespace() => SentenceState::Start,
            SentenceState::Ended if is_closing(c) => SentenceState::Ended,
            _ => SentenceState::Inside,
        }
    }
}

fn is_closing(c: char) -> bool {
    matches!(
        c,
        '"' | '\'' | ')' | ']' | '}' | '»' | '”' | '’' | '」' | '』' | '）'
    )
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '’')
}

/// Combining marks of the common blocks: part of the word they follow.
fn is_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_mark(c)
}

/// Whether a letter after `before` starts a word. Before an apostrophe,
/// combining marks are skipped, so decomposed text (`e` + U+0301) keeps
/// the apostrophe inside the word as the composed form does.
fn is_word_start(before: &[(usize, char)]) -> bool {
    let mut before = before.iter().rev().map(|&(_, c)| c);
    match before.next() {
        None => true,
        Some(p) if is_word_char(p) => false,
        Some(p) if is_apostrophe(p) => !before
            .find(|&c| !is_mark(c))
            .is_some_and(char::is_alphabetic),
        Some(_) => true,
    }
}

fn push_upper(out: &mut String, c: char, turkic: bool) {
    match c {
        'i' if turkic => out.push('İ'),
        _ => out.extend(c.to_uppercase()),
    }
}

/// The titlecase form: the uppercase one, except for the characters whose
/// Unicode titlecase mapping (UnicodeData.txt and SpecialCasing.txt)
/// differs from it.
fn push_title(out: &mut String, c: char, turkic: bool) {
    match c {
        // Latin digraphs: the mixed-case letter.
        '\u{01C4}'..='\u{01C6}' => out.push('\u{01C5}'),
        '\u{01C7}'..='\u{01C9}' => out.push('\u{01C8}'),
        '\u{01CA}'..='\u{01CC}' => out.push('\u{01CB}'),
        '\u{01F1}'..='\u{01F3}' => out.push('\u{01F2}'),
        'ß' => out.push_str("Ss"),
        // Latin and Armenian ligatures: capital first letter only.
        '\u{FB00}' => out.push_str("Ff"),
        '\u{FB01}' => out.push_str("Fi"),
        '\u{FB02}' => out.push_str("Fl"),
        '\u{FB03}' => out.push_str("Ffi"),
        '\u{FB04}' => out.push_str("Ffl"),
        '\u{FB05}' | '\u{FB06}' => out.push_str("St"),
        '\u{0587}' => out.push_str("\u{0535}\u{0582}"),
        '\u{FB13}' => out.push_str("\u{0544}\u{0576}"),
        '\u{FB14}' => out.push_str("\u{0544}\u{0565}"),
        '\u{FB15}' => out.push_str("\u{0544}\u{056B}"),
        '\u{FB16}' => out.push_str("\u{054E}\u{0576}"),
        '\u{FB17}' => out.push_str("\u{0544}\u{056D}"),
        // Georgian Mkhedruli has no titlecase: Mtavruli is only its
        // uppercase (all-caps) form.
        '\u{10D0}'..='\u{10FA}' | '\u{10FD}'..='\u{10FF}' => out.push(c),
        // Greek with ypogegrammeni: the capital with prosgegrammeni
        // (`ᾀ` → `ᾈ`), where uppercase spells the iota out (`ἈΙ`). In
        // U+1F80..=U+1FAF the capital is the small letter's code + 8.
        '\u{1F80}'..='\u{1FAF}' => {
            out.push(char::from_u32(u32::from(c) | 0x8).unwrap_or(c));
        }
        '\u{1FB3}' | '\u{1FBC}' => out.push('\u{1FBC}'),
        '\u{1FC3}' | '\u{1FCC}' => out.push('\u{1FCC}'),
        '\u{1FF3}' | '\u{1FFC}' => out.push('\u{1FFC}'),
        '\u{1FB2}' => out.push_str("\u{1FBA}\u{0345}"),
        '\u{1FB4}' => out.push_str("\u{0386}\u{0345}"),
        '\u{1FB7}' => out.push_str("\u{0391}\u{0342}\u{0345}"),
        '\u{1FC2}' => out.push_str("\u{1FCA}\u{0345}"),
        '\u{1FC4}' => out.push_str("\u{0389}\u{0345}"),
        '\u{1FC7}' => out.push_str("\u{0397}\u{0342}\u{0345}"),
        '\u{1FF2}' => out.push_str("\u{1FFA}\u{0345}"),
        '\u{1FF4}' => out.push_str("\u{038F}\u{0345}"),
        '\u{1FF7}' => out.push_str("\u{03A9}\u{0342}\u{0345}"),
        _ => push_upper(out, c, turkic),
    }
}

fn push_lower(out: &mut String, chars: &[(usize, char)], i: usize, turkic: bool) {
    let c = chars[i].1;
    match c {
        'I' if turkic => out.push('ı'),
        'İ' if turkic => out.push('i'),
        'Σ' if is_final_sigma(chars, i) => out.push('ς'),
        _ => out.extend(c.to_lowercase()),
    }
}

/// Unicode's Final_Sigma condition, approximated: a letter before (skipping
/// combining marks) and none after.
fn is_final_sigma(chars: &[(usize, char)], i: usize) -> bool {
    let before = chars[..i]
        .iter()
        .rev()
        .map(|&(_, c)| c)
        .find(|&c| !is_mark(c))
        .is_some_and(char::is_alphabetic);
    let after = chars[i + 1..]
        .iter()
        .map(|&(_, c)| c)
        .find(|&c| !is_mark(c))
        .is_some_and(char::is_alphabetic);
    before && !after
}

#[cfg(test)]
mod tests {
    use super::*;
    use CaseTransform::*;

    fn d(text: &str, case: CaseTransform) -> String {
        display(text, case, None).into_owned()
    }

    fn tr(text: &str, case: CaseTransform) -> String {
        display(text, case, Some(&LangTag::new("tr-TR"))).into_owned()
    }

    #[test]
    fn as_typed_and_unchanged_text_borrow() {
        assert!(matches!(
            display("Hello", AsTyped, None),
            Cow::Borrowed("Hello")
        ));
        assert!(matches!(display("ABC 12", Upper, None), Cow::Borrowed(_)));
        assert!(matches!(display("مرحبا", Lower, None), Cow::Borrowed(_)));
    }

    #[test]
    fn the_case_table() {
        let table: &[(&str, CaseTransform, &str)] = &[
            ("Straße", Upper, "STRASSE"),
            ("ǆungla", Upper, "ǄUNGLA"),
            ("ﬁne", Upper, "FINE"),
            ("HELLO World", Lower, "hello world"),
            ("ΟΔΟΣ ΣΑΣ", Lower, "οδος σας"),
            ("İstanbul", Lower, "i\u{307}stanbul"),
            ("the lord of the rings", Title, "The Lord Of The Rings"),
            ("don't STOP-me now", Title, "Don't Stop-Me Now"),
            ("3rd place, ǆungla", Title, "3rd Place, ǅungla"),
            ("ßa", Title, "Ssa"),
            ("'quoted' words", Title, "'Quoted' Words"),
            (
                "hello WORLD. how are you? fine!yes… \"ok.\" next",
                Sentence,
                "Hello world. How are you? Fine!yes… \"Ok.\" Next",
            ),
            ("  — first.\nsecond", Sentence, "  — First.\nSecond"),
            ("v3.2 is out", Sentence, "V3.2 is out"),
            ("中文。中文", Upper, "中文。中文"),
            // Titlecase differs from uppercase.
            ("\u{FB01}ne", Title, "Fine"),
            ("\u{FB02}ow", Sentence, "Flow"),
            ("\u{0587}", Title, "\u{0535}\u{0582}"),
            ("\u{1F80}", Title, "\u{1F88}"),
            ("\u{1F80}", Upper, "\u{1F08}\u{0399}"),
            ("\u{1FB7}", Title, "\u{0391}\u{0342}\u{0345}"),
            ("გამარჯობა", Title, "გამარჯობა"),
        ];
        for (input, case, want) in table {
            assert_eq!(&d(input, *case), want, "{input:?} as {case:?}");
        }
    }

    #[test]
    fn turkish_dotted_and_dotless_i() {
        assert_eq!(tr("istanbul ılık", Upper), "İSTANBUL ILIK");
        assert_eq!(tr("İSTANBUL ILIK", Lower), "istanbul ılık");
        assert_eq!(tr("izmir", Title), "İzmir");
        assert_eq!(display("izmir", Upper, Some(&LangTag::new("az"))), "İZMİR");
        // Not Turkish: the default mappings.
        assert_eq!(d("istanbul", Upper), "ISTANBUL");
        assert_eq!(display("ILIK", Lower, Some(&LangTag::new("en-US"))), "ilik");
    }

    #[test]
    fn length_changing_mappings_in_a_range() {
        // Only the selection changes; its context still decides word starts.
        let text = "groß und klein";
        let all = change_range(text, 0..text.len(), Upper, None);
        assert_eq!(all, "GROSS UND KLEIN");
        assert_eq!(change_range(text, 0..5, Upper, None), "GROSS und klein");
        assert_eq!(change_range(text, 10..15, Title, None), "groß und Klein");
        assert_eq!(change_range(text, 11..15, Title, None), "groß und klein");
        assert_eq!(change_range(text, 10..11, Title, None), "groß und Klein");
        // Inside a multi-byte character: widened to its boundaries.
        assert_eq!(change_range(text, 4..4, Upper, None), "groß und klein");
        assert_eq!(change_range(text, 3..4, Upper, None), "groSS und klein");
        assert_eq!(change_range(text, 9..999, Upper, None), "groß und KLEIN");
        assert_eq!(change_range(text, 99..999, Upper, None), text);
        assert_eq!(change_range(text, 0..99, AsTyped, None), text);
    }

    #[test]
    fn decomposed_letters_keep_the_apostrophe_inside_the_word() {
        assert_eq!(d("jos\u{E9}'s car", Title), "Jos\u{E9}'s Car");
        assert_eq!(d("jose\u{301}'s car", Title), "Jose\u{301}'s Car");
        assert_eq!(d("e\u{301}\u{300}'x", Title), "E\u{301}\u{300}'x");
    }

    #[test]
    fn every_character_titlecases_idempotently() {
        // Title(Title(c)) == Title(c) for every scalar value: the uppercase
        // fallback is not idempotent where Unicode's titlecase differs
        // (`ᾀ` → `ἈΙ` → `Ἀι`).
        let mut one = String::new();
        let mut unstable = Vec::new();
        for c in (0..=u32::from(char::MAX)).filter_map(char::from_u32) {
            one.clear();
            one.push(c);
            let once = d(&one, Title);
            if d(&once, Title) != once {
                unstable.push(format!("U+{:04X}", u32::from(c)));
            }
        }
        // The one exception: deprecated `ŉ` titlecases to `ʼN` as Unicode
        // says, and the modifier letter `ʼ` then starts the word, so a
        // second pass lowercases the `N`.
        assert_eq!(unstable, ["U+0149"]);
    }

    proptest::proptest! {
        // `ŉ` is left out: the one exception above.
        #[test]
        fn upper_lower_and_title_are_idempotent(text in "[\\PC&&[^\\x{149}]]{0,12}") {
            for case in [Upper, Lower, Title] {
                let once = d(&text, case);
                proptest::prop_assert_eq!(d(&once, case), once.clone(), "{:?}", case);
            }
        }
    }
}
