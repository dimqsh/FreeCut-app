use unicode_segmentation::UnicodeSegmentation;

use super::*;
use crate::shape::Rect;
use crate::text::{GlyphFont, LineInfo, PlacedGlyph, UnitIds};

fn t(v: i64) -> Tick {
    Tick::from_raw(v)
}

/// A layout of `text` with one 10 px glyph per grapheme (line breaks
/// draw none), glyph ids from the first character, one line per `\n`.
fn layout(text: &str) -> TextLayout {
    let words: Vec<usize> = text.unicode_word_indices().map(|(i, _)| i).collect();
    let mut glyphs = Vec::new();
    let (mut line, mut x) = (0u32, 0.0f32);
    for (gi, (byte, g)) in text.grapheme_indices(true).enumerate() {
        if g == "\n" {
            line += 1;
            x = 0.0;
            continue;
        }
        let word = words.partition_point(|&w| w <= byte).saturating_sub(1) as u32;
        glyphs.push(PlacedGlyph {
            font: GlyphFont { blob: 1, index: 0 },
            size: 20.0,
            glyph_id: g.chars().next().map_or(0, |c| c as u16),
            origin: (x, 20.0 * (line as f32 + 1.0)),
            advance: 10.0,
            cluster: byte as u32,
            unit: UnitIds {
                grapheme: gi as u32,
                word,
                line,
            },
        });
        x += 10.0;
    }
    let lines = (0..=line)
        .map(|l| LineInfo {
            top: 25.0 * l as f32,
            bottom: 25.0 * (l as f32 + 1.0),
            baseline: 20.0 * (l as f32 + 1.0),
            left: 0.0,
            width: 10.0
                * glyphs
                    .iter()
                    .filter(|g: &&PlacedGlyph| g.unit.line == l)
                    .count() as f32,
        })
        .collect();
    TextLayout {
        size: (100.0, 25.0 * (line as f32 + 1.0)),
        lines,
        glyphs,
        family: "Test".into(),
        fallback: Default::default(),
        rtl: false,
        em: 20.0,
    }
}

fn table(curves: Vec<ChannelCurve>) -> UnitChannels {
    UnitChannels::Table(ChannelTable(curves))
}

fn curve(channel: GlyphChannel, points: &[(f32, f32)]) -> ChannelCurve {
    ChannelCurve {
        channel,
        points: points.to_vec(),
        hold: false,
        idle: None,
        noise: None,
    }
}

fn rise() -> UnitChannels {
    table(vec![
        curve(GlyphChannel::OffsetY, &[(0.0, 1.0), (1.0, 0.0)]),
        curve(GlyphChannel::Opacity, &[(0.0, 0.0), (1.0, 1.0)]),
    ])
}

/// Shown at once from the start of its window.
fn typewriter() -> UnitChannels {
    table(vec![ChannelCurve {
        hold: true,
        idle: Some(0.0),
        ..curve(GlyphChannel::Reveal, &[(0.0, 1.0)])
    }])
}

fn input(span: i64, overlap: u16) -> UnitTiming {
    UnitTiming::In {
        span: t(span),
        overlap: PerMille::new(overlap),
    }
}

// --- SplitMix64, shuffle and orders (frozen, R6) ---

#[test]
fn splitmix64_golden_vectors() {
    let golden: [(u64, [u64; 3]); 3] = [
        (
            0,
            [
                0xE220_A839_7B1D_CDAF,
                0x6E78_9E6A_A1B9_65F4,
                0x06C4_5D18_8009_454F,
            ],
        ),
        (
            1,
            [
                0x910A_2DEC_8902_5CC1,
                0xBEEB_8DA1_658E_EC67,
                0xF893_A2EE_FB32_555E,
            ],
        ),
        (
            0xDEAD_BEEF,
            [
                0x4ADF_B90F_68C9_EB9B,
                0xDE58_6A31_41A1_0922,
                0x021F_BC2F_8E1C_FC1D,
            ],
        ),
    ];
    for (seed, want) in golden {
        let mut r = SplitMix64::new(seed);
        assert_eq!([r.next_u64(), r.next_u64(), r.next_u64()], want, "{seed}");
    }
}

#[test]
fn random_order_golden_vectors() {
    let golden: [(u32, [u32; 10]); 3] = [
        (0, [4, 9, 2, 5, 1, 7, 6, 0, 3, 8]),
        (1, [9, 0, 1, 4, 8, 2, 3, 7, 6, 5]),
        (0xDEAD_BEEF, [4, 8, 9, 5, 6, 1, 3, 0, 7, 2]),
    ];
    for (seed, want) in golden {
        assert_eq!(play_order(10, UnitOrder::Random { seed }), want, "{seed}");
    }
}

#[test]
fn fixed_order_golden_vectors() {
    let o = |n, order| play_order(n, order);
    assert_eq!(o(5, UnitOrder::Forward), [0, 1, 2, 3, 4]);
    assert_eq!(o(5, UnitOrder::Backward), [4, 3, 2, 1, 0]);
    assert_eq!(o(5, UnitOrder::CenterOut), [2, 1, 3, 0, 4]);
    // Ties take the left unit first.
    assert_eq!(o(6, UnitOrder::CenterOut), [2, 3, 1, 4, 0, 5]);
    assert_eq!(o(5, UnitOrder::EdgesIn), [0, 4, 1, 3, 2]);
    assert_eq!(o(6, UnitOrder::EdgesIn), [0, 5, 1, 4, 2, 3]);
    for order in [
        UnitOrder::Forward,
        UnitOrder::Backward,
        UnitOrder::CenterOut,
        UnitOrder::EdgesIn,
        UnitOrder::Random { seed: 9 },
    ] {
        assert!(o(0, order).is_empty());
        assert_eq!(o(1, order), [0]);
        // Always a permutation.
        let mut p = o(37, order);
        p.sort_unstable();
        assert_eq!(p, (0..37).collect::<Vec<_>>(), "{order:?}");
    }
}

#[test]
fn bounded_draws_stay_below_the_bound() {
    let mut r = SplitMix64::new(42);
    for bound in [1u64, 2, 3, 7, 1000, u64::MAX / 3 + 1] {
        for _ in 0..200 {
            assert!(r.below(bound) < bound);
        }
    }
    assert_eq!(r.below(0), 0);
}

#[test]
fn unit_noise_golden_values() {
    let got: Vec<f32> = (0..4).map(|u| unit_noise(7, u, 0)).collect();
    let want = [-0.246_315_12, 0.188_433_41, -0.288_377_29, 0.330_263_02];
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-7, "{got:?}");
    }
    assert!((unit_noise(0xDEAD_BEEF, 3, 5) + 0.633_384_2).abs() < 1e-7);
}

// --- Windows, delays, progress ---

#[test]
fn unit_windows_examples() {
    // Overlap 0: one after another.
    assert_eq!(
        unit_windows(t(300), 3, PerMille::ZERO),
        [(t(0), t(100)), (t(100), t(200)), (t(200), t(300))]
    );
    // Overlap 100 %: all together.
    assert_eq!(unit_windows(t(300), 3, PerMille::ONE), [(t(0), t(300)); 3]);
    // 50 %: k = 500, d = 2000.
    assert_eq!(
        unit_windows(t(1000), 3, PerMille::new(500)),
        [(t(0), t(500)), (t(250), t(750)), (t(500), t(1000))]
    );
    assert!(unit_windows(t(1000), 0, PerMille::ZERO).is_empty());
    // A single unit runs over the whole slot.
    assert_eq!(unit_windows(t(7), 1, PerMille::new(300)), [(t(0), t(7))]);
    // Overlaps above 100 % count as 100 %, negative spans as 0.
    assert_eq!(
        unit_windows(t(10), 2, PerMille::new(2000)),
        [(t(0), t(10)); 2]
    );
    assert_eq!(unit_windows(t(-5), 2, PerMille::ZERO), [(t(0), t(0)); 2]);
}

/// The property of the text architecture's test strategy: for any span,
/// unit count and overlap, windows are monotonic, lie in `[0, span]`, the
/// last ends exactly at `span`, and overlap 1000 gives n copies of
/// `[0, span)`. Random cases from a fixed seed, plus every small case.
#[test]
fn unit_windows_property() {
    let check = |span: i64, n: u32, overlap: u16| {
        let w = unit_windows(t(span), n, PerMille::new(overlap));
        assert_eq!(w.len(), n as usize);
        if n == 0 {
            return;
        }
        assert_eq!(w[0].0, t(0), "{span} {n} {overlap}");
        assert_eq!(w[n as usize - 1].1, t(span), "{span} {n} {overlap}");
        for (i, &(s, e)) in w.iter().enumerate() {
            assert!(
                t(0) <= s && s <= e && e <= t(span),
                "{span} {n} {overlap}: {i}"
            );
            if i > 0 {
                assert!(
                    w[i - 1].0 <= s && w[i - 1].1 <= e,
                    "{span} {n} {overlap}: {i}"
                );
            }
        }
        if overlap >= 1000 {
            assert!(w.iter().all(|&x| x == (t(0), t(span))));
        }
        if overlap == 0 {
            // Back to back, never overlapping.
            for pair in w.windows(2) {
                assert_eq!(pair[0].1, pair[1].0);
            }
        }
    };
    for span in 0..40 {
        for n in 0..12 {
            for overlap in [0, 1, 250, 500, 999, 1000] {
                check(span, n, overlap);
            }
        }
    }
    let mut r = SplitMix64::new(0x5EED);
    for _ in 0..3000 {
        let span = match r.below(3) {
            0 => r.below(1000) as i64,
            1 => r.below(1 << 40) as i64,
            _ => i64::MAX - r.below(1000) as i64,
        };
        let n = 1 + r.below(10_000) as u32;
        let overlap = r.below(1001) as u16;
        check(span, n, overlap);
    }
}

#[test]
fn loop_delays_spread_one_cycle_across_the_units() {
    let cycle = t(1200);
    let n = 4;
    let d: Vec<Tick> = (0..n)
        .map(|r| loop_delay(cycle, n, r, PerMille::ZERO))
        .collect();
    assert_eq!(d, [t(0), t(300), t(600), t(900)]);
    // Overlap 100 %: all in phase.
    assert!((0..n).all(|r| loop_delay(cycle, n, r, PerMille::ONE) == t(0)));
    // The phase wraps: a unit delayed by 300 is at phase 0 at t = 300 and
    // at t = 1500.
    assert_eq!(loop_phase(t(300), t(300), cycle), 0.0);
    assert_eq!(loop_phase(t(1500), t(300), cycle), 0.0);
    assert_eq!(loop_phase(t(0), t(300), cycle), 0.75);
    assert_eq!(loop_phase(t(5), t(0), t(0)), 0.0);
}

#[test]
fn progress_is_exact_at_the_edges() {
    let w = (t(100), t(300));
    assert_eq!(progress(t(0), w), 0.0);
    assert_eq!(progress(t(100), w), 0.0);
    assert_eq!(progress(t(200), w), 0.5);
    assert_eq!(progress(t(300), w), 1.0);
    assert_eq!(progress(t(301), w), 1.0);
    // An empty window jumps at its end.
    assert_eq!(progress(t(99), (t(100), t(100))), 0.0);
    assert_eq!(progress(t(100), (t(100), t(100))), 1.0);
}

// --- Units ---

#[test]
fn whitespace_takes_no_time() {
    let l = layout(" ab cd ");
    let map = UnitMap::new(&l, " ab cd ", AnimUnit::Grapheme);
    assert_eq!(map.count, 4);
    // Leading space goes with the first letter, the others with the letter
    // before them.
    assert_eq!(map.glyph_unit, [0, 0, 1, 1, 2, 3, 3]);
    assert_eq!(map.blank, [true, false, false, true, false, false, true]);
    // The unit box covers ink glyphs only.
    assert_eq!(map.boxes[1], Some(Rect::new(20.0, 0.0, 30.0, 25.0)));
}

#[test]
fn words_keep_their_punctuation_and_lines_skip_empty_ones() {
    let text = "Hi, you!\n\nok";
    let l = layout(text);
    let words = UnitMap::new(&l, text, AnimUnit::Word);
    assert_eq!(words.count, 3);
    assert_eq!(words.glyph_unit, [0, 0, 0, 0, 1, 1, 1, 1, 2, 2]);
    let lines = UnitMap::new(&l, text, AnimUnit::Line);
    assert_eq!(lines.count, 2, "the empty line takes no time");
    assert_eq!(lines.glyph_unit[8..], [1, 1]);
    // Punctuation only: one unit.
    let bang = layout("!?");
    assert_eq!(UnitMap::new(&bang, "!?", AnimUnit::Word).count, 1);
}

#[test]
fn letters_in_cursive_scripts_degrade_to_words() {
    let text = "سلام دنیا";
    let l = layout(text);
    let map = UnitMap::new(&l, text, AnimUnit::Grapheme);
    assert_eq!(map.unit, AnimUnit::Word);
    assert_eq!(map.count, 2);
    let latin = UnitMap::new(&layout("ab"), "ab", AnimUnit::Grapheme);
    assert_eq!(latin.unit, AnimUnit::Grapheme);
}

// --- Channels ---

#[test]
fn channel_curves_interpolate_hold_and_vary_per_unit() {
    let s = |progress| UnitSample {
        unit: 2,
        rank: 0,
        count: 3,
        progress,
        window: (t(0), t(1)),
        idle: false,
        geometry: UnitGeometry::default(),
    };
    let c = curve(GlyphChannel::OffsetX, &[(0.2, 1.0), (0.6, 3.0)]);
    assert_eq!(c.at(&s(0.0)), Some(1.0));
    assert_eq!(c.at(&s(0.4)), Some(2.0));
    assert_eq!(c.at(&s(0.9)), Some(3.0));
    let held = ChannelCurve {
        hold: true,
        ..c.clone()
    };
    assert_eq!(held.at(&s(0.5)), Some(1.0));
    assert_eq!(held.at(&s(0.6)), Some(3.0));
    let noisy = ChannelCurve {
        noise: Some(Noise { seed: 7, salt: 0 }),
        ..c
    };
    assert_eq!(noisy.at(&s(0.0)), Some(unit_noise(7, 2, 0)));
    assert_eq!(curve(GlyphChannel::Blur, &[]).at(&s(0.5)), None);

    // Offsets add, scales and opacities multiply.
    let t2 = ChannelTable(vec![
        curve(GlyphChannel::OffsetY, &[(0.0, 1.0)]),
        curve(GlyphChannel::OffsetY, &[(0.0, 0.5)]),
        curve(GlyphChannel::Scale, &[(0.0, 2.0)]),
        curve(GlyphChannel::Scale, &[(0.0, 0.25)]),
        curve(GlyphChannel::Opacity, &[(0.0, 0.5)]),
    ]);
    let a = t2.eval(&s(0.0));
    assert_eq!((a.offset.1, a.scale, a.opacity), (1.5, 0.5, 0.5));
}

#[test]
fn poses_are_sanitized() {
    let a = GlyphAnim {
        offset: (f32::NAN, 1e9),
        scale: f32::INFINITY,
        opacity: 2.0,
        reveal: -1.0,
        blur: -3.0,
        ..GlyphAnim::REST
    }
    .sanitized();
    assert_eq!(a.offset, (0.0, 64.0));
    assert_eq!((a.scale, a.opacity, a.reveal, a.blur), (1.0, 1.0, 0.0, 0.0));
}

// --- Evaluation ---

fn at(motion: &TextMotion, text: &str, time: i64) -> MotionState {
    TextMotion {
        at: t(time),
        ..motion.clone()
    }
    .evaluate(&layout(text), text)
}

#[test]
fn in_runs_from_the_start_pose_to_rest_and_out_backwards() {
    let m = TextMotion::new(
        AnimUnit::Grapheme,
        UnitOrder::Forward,
        input(400, 0),
        t(0),
        rise(),
    );
    let s = at(&m, "abcd", 150);
    // Units 0..4 have windows of 100: a done, b halfway, c and d waiting.
    let y: Vec<f32> = s.glyphs.iter().map(|g| g.anim.offset.1).collect();
    assert_eq!(y, [0.0, 0.5, 1.0, 1.0]);
    assert!(at(&m, "abcd", 400).is_rest());

    let out = TextMotion {
        timing: UnitTiming::Out {
            span: t(400),
            overlap: PerMille::ZERO,
        },
        ..m.clone()
    };
    assert!(at(&out, "abcd", 0).is_rest(), "Out starts exactly at rest");
    let s = at(&out, "abcd", 150);
    // The first unit has left (start pose), the second is on its way.
    let y: Vec<f32> = s.glyphs.iter().map(|g| g.anim.offset.1).collect();
    assert_eq!(y, [1.0, 0.5, 0.0, 0.0]);

    let back = TextMotion {
        order: UnitOrder::Backward,
        ..m
    };
    let s = at(&back, "abcd", 150);
    let y: Vec<f32> = s.glyphs.iter().map(|g| g.anim.offset.1).collect();
    assert_eq!(y, [1.0, 1.0, 0.5, 0.0]);
}

#[test]
fn whitespace_moves_with_the_letter_before_it() {
    let m = TextMotion::new(
        AnimUnit::Grapheme,
        UnitOrder::Forward,
        input(300, 0),
        t(150),
        rise(),
    );
    let s = at(&m, "a bc", 150);
    // Three timed units of 100 ticks; the space goes with "a".
    assert_eq!(s.glyphs[1].unit, 0);
    assert_eq!(s.glyphs[1].anim, s.glyphs[0].anim);
    assert_eq!(s.glyphs[2].anim.offset.1, 0.5);
}

#[test]
fn a_loop_wave_travels_and_wraps() {
    let wave = table(vec![curve(
        GlyphChannel::OffsetY,
        &[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)],
    )]);
    let m = TextMotion::new(
        AnimUnit::Grapheme,
        UnitOrder::Forward,
        UnitTiming::Loop {
            cycle: t(400),
            overlap: PerMille::ZERO,
        },
        t(0),
        wave,
    );
    let y = |time| -> Vec<f32> {
        at(&m, "abcd", time)
            .glyphs
            .iter()
            .map(|g| g.anim.offset.1)
            .collect()
    };
    assert_eq!(y(0), [0.0, 0.5, 1.0, 0.5]);
    assert_eq!(y(100), [0.5, 0.0, 0.5, 1.0]);
    // Seamless: one cycle later, the same.
    assert_eq!(y(400), y(0));
}

#[test]
fn the_reveal_keyframe_and_a_unit_reveal_multiply() {
    let m = TextMotion {
        reveal: Some(PerMille::new(500)),
        ..TextMotion::new(
            AnimUnit::Word,
            UnitOrder::Forward,
            input(100, 0),
            t(100),
            rise(),
        )
    };
    // Four letters, half shown: "ab" of "ab cd"; the space after "b" too.
    let s = at(&m, "ab cd", 100);
    let shown: Vec<bool> = s.glyphs.iter().map(|g| g.shown == Shown::All).collect();
    assert_eq!(shown, [true, true, true, false, false]);

    // A unit half revealed (Word "abcd" box 0..40): only x < 20 shows,
    // multiplied with the keyframe's 3 of 4 letters.
    let half = table(vec![curve(GlyphChannel::Reveal, &[(0.0, 0.5)])]);
    let m = TextMotion {
        reveal: Some(PerMille::new(750)),
        channels: half,
        ..m
    };
    let s = at(&m, "abcd", 0);
    assert_eq!(s.glyphs[0].shown, Shown::Range(f32::NEG_INFINITY, 20.0));
    assert_eq!(s.glyphs[3].shown, Shown::None);
    assert_eq!(
        Shown::Range(0.0, 20.0).and(Shown::Range(10.0, 30.0)),
        Shown::Range(10.0, 20.0)
    );
}

#[test]
fn a_ligature_is_revealed_proportionally() {
    // One glyph for "fi" (two letters), then "x".
    let mut l = layout("fix");
    l.glyphs.remove(1);
    l.glyphs[0].advance = 20.0;
    l.glyphs[1].origin.0 = 20.0;
    let m = TextMotion {
        reveal: Some(PerMille::new(333)),
        ..TextMotion::new(
            AnimUnit::Grapheme,
            UnitOrder::Forward,
            input(10, 0),
            t(10),
            rise(),
        )
    };
    let s = m.evaluate(&l, "fix");
    // ⌊3 × 0.333⌋ = 0 letters.
    assert_eq!(s.glyphs[0].shown, Shown::None);
    let m = TextMotion {
        reveal: Some(PerMille::new(334)),
        ..m
    };
    let s = m.evaluate(&l, "fix");
    assert_eq!(s.glyphs[0].shown, Shown::Range(f32::NEG_INFINITY, 10.0));
    assert_eq!(s.glyphs[1].shown, Shown::None);
}

#[test]
fn rtl_reveals_from_the_right() {
    let mut l = layout("abcd");
    l.rtl = true;
    let half = table(vec![curve(GlyphChannel::Reveal, &[(0.0, 0.5)])]);
    let m = TextMotion::new(AnimUnit::Line, UnitOrder::Forward, input(10, 0), t(0), half);
    let s = m.evaluate(&l, "abcd");
    assert_eq!(s.glyphs[0].shown, Shown::Range(20.0, f32::INFINITY));
}

#[test]
fn decode_stand_ins_are_seeded_and_land_on_the_letter() {
    let m = TextMotion {
        decode: Some(Decode {
            seed: 11,
            step: t(20),
        }),
        ..TextMotion::new(
            AnimUnit::Grapheme,
            UnitOrder::Forward,
            input(400, 600),
            t(0),
            table(Vec::new()),
        )
    };
    let picks = |time| -> Vec<Option<u32>> {
        at(&m, "ABCDE", time)
            .glyphs
            .iter()
            .map(|g| g.stand_in)
            .collect()
    };
    // Frozen (R6): changing the choice changes saved projects' look.
    assert_eq!(
        picks(0),
        [Some(2), Some(0), Some(0), Some(4), Some(2)],
        "t = 0"
    );
    assert_eq!(
        picks(25),
        [Some(3), Some(0), Some(0), Some(2), Some(3)],
        "the next step"
    );
    // Within one step nothing changes; landed letters show themselves.
    assert_eq!(picks(39), picks(25));
    let late = picks(399);
    assert_eq!(late[..4], [None, None, None, None]);
    assert!(late[4].is_some());
    assert!(picks(400).iter().all(Option::is_none));
    // Never its own letter.
    for time in (0..400).step_by(20) {
        for (i, s) in picks(time).iter().enumerate() {
            assert_ne!(*s, Some(i as u32));
        }
    }
    // One distinct letter: nothing to swap with.
    assert!(
        TextMotion {
            at: t(0),
            ..m.clone()
        }
        .evaluate(&layout("aaa"), "aaa")
        .glyphs
        .iter()
        .all(|g| g.stand_in.is_none())
    );
    // Not on words.
    let words = TextMotion {
        unit: AnimUnit::Word,
        ..m
    };
    assert!(
        at(&words, "AB CD", 0)
            .glyphs
            .iter()
            .all(|g| g.stand_in.is_none())
    );
}

#[test]
fn the_caret_rides_after_the_last_letter_and_blinks_after_typing() {
    let m = TextMotion {
        caret: Some(Caret { blink: t(100) }),
        ..TextMotion::new(
            AnimUnit::Grapheme,
            UnitOrder::Forward,
            input(400, 0),
            t(0),
            typewriter(),
        )
    };
    // Typewriter: each letter appears at its window's start (0, 100, 200,
    // 300 for "abcd").
    let caret = |time| at(&m, "abcd", time).caret;
    let width = CARET_WIDTH_EM * 20.0;
    assert_eq!(
        caret(0),
        Some(CaretBar {
            line: 0,
            x: 10.0,
            width
        })
    );
    assert_eq!(caret(150).map(|c| c.x), Some(20.0));
    // Typing ends at 300 (the last letter shows); then 50 on, 50 off.
    assert_eq!(caret(320).map(|c| c.x), Some(40.0));
    assert_eq!(caret(360), None);
    // 399 is in the second half-period after typing ended: off.
    assert_eq!(caret(399), None);
    assert!(caret(349).is_some());
    assert_eq!(caret(400), None, "the slot is over");

    // Right to left: on the left of the last revealed letter.
    let mut l = layout("abcd");
    l.rtl = true;
    let s = TextMotion { at: t(150), ..m }.evaluate(&l, "abcd");
    assert_eq!(s.caret.map(|c| c.x), Some(10.0 - width));
}

#[test]
fn digests_identify_equal_states() {
    let m = TextMotion::new(
        AnimUnit::Grapheme,
        UnitOrder::Forward,
        input(400, 0),
        t(0),
        rise(),
    );
    let a = at(&m, "abcd", 150);
    let b = at(&m, "abcd", 150);
    assert_eq!(a.digest(), b.digest());
    assert_ne!(a.digest(), at(&m, "abcd", 160).digest());
    // Before any window starts every frame is the same.
    let before = TextMotion {
        timing: input(400, 0),
        ..m.clone()
    };
    let mut c = at(&before, "abcd", 0);
    c.glyphs[0].anim.offset.0 = -0.0;
    let d = at(&before, "abcd", 0);
    assert_eq!(c.digest(), d.digest(), "−0 and 0 alike");
    assert!(!d.is_rest());
}

#[test]
fn callbacks_drive_poses_and_compare_by_identity() {
    let f = UnitFn::new(|s: &UnitSample| GlyphAnim {
        rotation: 90.0 * (1.0 - s.progress),
        ..GlyphAnim::REST
    });
    assert_eq!(f, f.clone());
    assert_ne!(f, UnitFn::new(|_: &UnitSample| GlyphAnim::REST));
    let m = TextMotion::new(
        AnimUnit::Grapheme,
        UnitOrder::Forward,
        input(200, 0),
        t(50),
        UnitChannels::Callback(f),
    );
    let s = at(&m, "ab", 50);
    assert_eq!(s.glyphs[0].anim.rotation, 45.0);
    assert_eq!(s.glyphs[1].anim.rotation, 90.0);
}

// --- Line geometry (Spread, Word rise) ---

#[test]
fn units_know_the_way_to_their_line_centre_and_its_height() {
    // "ab cd" (50 px wide) over "ef" (20 px); 20 px em, 25 px lines.
    let text = "ab cd\nef";
    let l = layout(text);
    let map = UnitMap::new(&l, text, AnimUnit::Grapheme);
    assert_eq!(
        map.lines,
        [Some(0), Some(0), Some(0), Some(0), Some(1), Some(1)]
    );
    let g = UnitGeometry::of_units(&map, &l);
    let x: Vec<f32> = g.iter().map(|g| g.to_line_center.0).collect();
    // 'a' centred at 5 on a line centred at 25: 1 em to the right.
    assert_eq!(x, [1.0, 0.5, -0.5, -1.0, 0.25, -0.25]);
    assert!(
        g.iter()
            .all(|g| g.to_line_center.1 == 0.0 && g.line_height == 1.25)
    );
    // Words: "ab" is centred at 10, "cd" at 40.
    let words = UnitMap::new(&l, text, AnimUnit::Word);
    let w = UnitGeometry::of_units(&words, &l);
    let x: Vec<f32> = w.iter().map(|g| g.to_line_center.0).collect();
    assert_eq!(x, [0.75, -0.75, 0.0]);
    // A line is its own line's centre.
    let lines = UnitMap::new(&l, text, AnimUnit::Line);
    let g = UnitGeometry::of_units(&lines, &l);
    assert!(g.iter().all(|g| g.to_line_center == (0.0, 0.0)));
    // Without its em, the layout measures in its first glyph's size.
    let bare = TextLayout {
        em: 0.0,
        ..l.clone()
    };
    assert_eq!(UnitGeometry::of_units(&map, &bare)[0].line_height, 1.25);
}

#[test]
fn the_sampled_geometry_reaches_the_callback() {
    let f = UnitFn::new(|s: &UnitSample| GlyphAnim {
        offset: (s.geometry.to_line_center.0, s.geometry.line_height),
        ..GlyphAnim::REST
    });
    let m = TextMotion::new(
        AnimUnit::Grapheme,
        UnitOrder::Forward,
        input(200, 0),
        t(0),
        UnitChannels::Callback(f),
    );
    let s = at(&m, "ab", 0);
    assert_eq!(s.glyphs[0].anim.offset, (0.25, 1.25));
    assert_eq!(s.glyphs[1].anim.offset, (-0.25, 1.25));
}

#[test]
fn the_line_clip_survives_sanitizing_and_tells_digests_apart() {
    let clipped = GlyphAnim {
        offset: (0.0, f32::NAN),
        line_clip: true,
        ..GlyphAnim::REST
    }
    .sanitized();
    assert!(clipped.line_clip);
    assert_eq!(clipped.offset, (0.0, 0.0));
    let pose = |line_clip| {
        UnitFn::new(move |_: &UnitSample| GlyphAnim {
            offset: (0.0, 0.5),
            line_clip,
            ..GlyphAnim::REST
        })
    };
    let state = |line_clip| {
        let m = TextMotion::new(
            AnimUnit::Grapheme,
            UnitOrder::Forward,
            input(200, 0),
            t(0),
            UnitChannels::Callback(pose(line_clip)),
        );
        at(&m, "ab", 0)
    };
    assert_ne!(state(true).digest(), state(false).digest());
}
