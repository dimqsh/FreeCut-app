//! Text layout and rasterization with the bundled fonts only (never system
//! fonts in tests).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::sync::Arc;

use freecut_render::text::{
    BUNDLED_FONTS, DEFAULT_FAMILY, Fill, FontRegistry, TextAlign, TextRaster, TextRenderer,
    TextSpec,
};
use pretty_assertions::assert_eq;

fn renderer() -> TextRenderer {
    TextRenderer::new(FontRegistry::bundled())
}

fn spec(text: &str) -> TextSpec {
    TextSpec {
        size_px: 50.0,
        ..TextSpec::new(text)
    }
}

/// Alpha of the image pixel at (x, y).
fn alpha(raster: &TextRaster, x: u32, y: u32) -> u8 {
    let img = &raster.image;
    img.rgba[((y * img.width + x) * 4 + 3) as usize]
}

/// Columns (in box coordinates, layout px at scale 1) holding any ink.
fn inked_columns(raster: &TextRaster) -> (i32, i32) {
    let img = &raster.image;
    let cols: Vec<u32> = (0..img.width)
        .filter(|&x| (0..img.height).any(|y| alpha(raster, x, y) > 0))
        .collect();
    let first = *cols.first().unwrap() as i32 - raster.origin.0;
    let last = *cols.last().unwrap() as i32 - raster.origin.0;
    (first, last)
}

#[test]
fn the_bundled_fonts_are_four_noto_sans_faces() {
    assert_eq!(BUNDLED_FONTS.len(), 4);
    assert!(BUNDLED_FONTS.iter().all(|f| f.data.len() > 100_000));
    assert_eq!(
        &*FontRegistry::bundled().family_names(),
        &["Noto Sans".to_owned()]
    );
}

#[test]
fn layout_grows_with_the_text() {
    let mut r = renderer();
    let short = r.layout(&spec("Hi"));
    let long = r.layout(&spec("Hello, world"));
    assert!(long.size.0 > short.size.0 * 2.0, "{short:?} {long:?}");
    assert_eq!(short.size.1, long.size.1);
    assert_eq!(short.lines.len(), 1);
    // About one line of 50 px Noto Sans (line height ≈ 1.36 em).
    assert!((60.0..75.0).contains(&short.size.1), "{:?}", short.size);
    let bigger = r.layout(&TextSpec {
        size_px: 100.0,
        ..spec("Hi")
    });
    assert!((bigger.size.0 / short.size.0 - 2.0).abs() < 0.01);
    assert!((bigger.size.1 / short.size.1 - 2.0).abs() < 0.01);
}

#[test]
fn a_fixed_box_wraps_lines_at_its_width() {
    let mut r = renderer();
    let text = "the quick brown fox jumps over the lazy dog";
    let auto = r.layout(&spec(text));
    assert_eq!(auto.lines.len(), 1);
    let boxed = r.layout(&TextSpec {
        box_width_px: Some(300.0),
        ..spec(text)
    });
    assert!(boxed.lines.len() >= 3, "{boxed:?}");
    assert_eq!(boxed.size.0, 300.0);
    for line in &boxed.lines {
        assert!(line.left + line.width <= 300.0 + 0.01, "{line:?}");
    }
    assert!((boxed.size.1 - auto.size.1 * boxed.lines.len() as f32).abs() < 0.5);
    // A word longer than the box breaks mid-word as a last resort.
    let long_word = r.layout(&TextSpec {
        box_width_px: Some(120.0),
        ..spec("Supercalifragilistic")
    });
    assert!(long_word.lines.len() >= 2);
    // The box is never narrower than one em.
    let narrow = r.layout(&TextSpec {
        box_width_px: Some(5.0),
        ..spec("ab")
    });
    assert_eq!(narrow.size.0, 50.0);
}

#[test]
fn explicit_line_breaks_start_new_lines() {
    let mut r = renderer();
    let one = r.layout(&spec("one"));
    let two = r.layout(&spec("one\ntwo"));
    assert_eq!(two.lines.len(), 2);
    assert!((two.size.1 - 2.0 * one.size.1).abs() < 0.5);
    // Empty lines are kept.
    assert_eq!(r.layout(&spec("a\n\nb")).lines.len(), 3);
    // CRLF is normalized upstream; a stray CR is neither a break nor a
    // missing glyph.
    let cr = r.layout(&spec("a\r\nb"));
    assert_eq!(cr.lines.len(), 2);
    assert!(cr.fallback.missing_chars.is_empty(), "{:?}", cr.fallback);
}

#[test]
fn line_spacing_scales_the_line_height() {
    let mut r = renderer();
    let normal = r.layout(&spec("a\nb"));
    let double = r.layout(&TextSpec {
        line_spacing: 2.0,
        ..spec("a\nb")
    });
    assert!((double.size.1 / normal.size.1 - 2.0).abs() < 0.01);
}

#[test]
fn alignment_offsets_lines_inside_the_box() {
    let mut r = renderer();
    let width = r.layout(&spec("Hi")).size.0;
    let at = |r: &mut TextRenderer, align| {
        r.layout(&TextSpec {
            align,
            box_width_px: Some(1000.0),
            ..spec("Hi")
        })
        .lines[0]
            .left
    };
    assert!(at(&mut r, TextAlign::Left).abs() < 0.01);
    assert!((at(&mut r, TextAlign::Center) - (1000.0 - width) / 2.0).abs() < 0.5);
    assert!((at(&mut r, TextAlign::Right) - (1000.0 - width)).abs() < 0.5);
    // Auto width: lines align against the widest one.
    let multi = r.layout(&TextSpec {
        align: TextAlign::Right,
        ..spec("a\nwide line")
    });
    let (short, long) = (multi.lines[0], multi.lines[1]);
    assert!(long.left.abs() < 0.01);
    assert!((short.left + short.width - (long.left + long.width)).abs() < 0.5);

    // The rasterized ink moves with the alignment.
    let left = r.rasterize(
        &TextSpec {
            box_width_px: Some(600.0),
            ..spec("Hi")
        },
        1.0,
    );
    let right = r.rasterize(
        &TextSpec {
            align: TextAlign::Right,
            box_width_px: Some(600.0),
            ..spec("Hi")
        },
        1.0,
    );
    let (l0, _) = inked_columns(&left);
    let (r0, _) = inked_columns(&right);
    assert!(
        ((r0 - l0) as f32 - (600.0 - width)).abs() <= 1.0,
        "{l0} {r0}"
    );
}

#[test]
fn bold_and_italic_use_their_faces() {
    let mut r = renderer();
    let regular = r.layout(&spec("Bold text"));
    let bold = r.layout(&TextSpec {
        bold: true,
        ..spec("Bold text")
    });
    assert!(bold.size.0 > regular.size.0, "{regular:?} {bold:?}");
    let raster = |r: &mut TextRenderer, bold, italic| {
        r.rasterize(
            &TextSpec {
                bold,
                italic,
                ..spec("Bold text")
            },
            1.0,
        )
    };
    let plain = raster(&mut r, false, false);
    let heavy = raster(&mut r, true, false);
    let slanted = raster(&mut r, false, true);
    let ink = |t: &TextRaster| t.image.rgba.chunks(4).map(|p| u64::from(p[3])).sum::<u64>();
    assert!(ink(&heavy) as f64 > ink(&plain) as f64 * 1.3);
    assert_ne!(plain.image.rgba, slanted.image.rgba);
    assert!(!plain.layout.fallback.family_missing);
    assert!(heavy.layout.fallback.fallback_families.is_empty());
}

#[test]
fn italic_ink_leans_right() {
    let mut r = renderer();
    let raster = r.rasterize(
        &TextSpec {
            italic: true,
            size_px: 120.0,
            ..TextSpec::new("I")
        },
        1.0,
    );
    let img = &raster.image;
    let centroid = |rows: std::ops::Range<u32>| {
        let (mut sum, mut n) = (0u64, 0u64);
        for y in rows {
            for x in 0..img.width {
                let a = u64::from(alpha(&raster, x, y));
                sum += a * u64::from(x);
                n += a;
            }
        }
        sum as f64 / n.max(1) as f64
    };
    let ink_rows: Vec<u32> = (0..img.height)
        .filter(|&y| (0..img.width).any(|x| alpha(&raster, x, y) > 0))
        .collect();
    let (top, bottom) = (ink_rows[0], *ink_rows.last().unwrap());
    let quarter = (bottom - top) / 4;
    let upper = centroid(top..top + quarter);
    let lower = centroid(bottom - quarter..bottom);
    assert!(upper > lower + 3.0, "upper {upper} lower {lower}");
}

#[test]
fn an_unknown_family_falls_back_to_noto_sans_and_says_so() {
    let mut r = renderer();
    let missing = TextSpec {
        family: "Definitely Not Installed".into(),
        ..spec("Fallback")
    };
    let raster = r.rasterize(&missing, 1.0);
    assert!(raster.layout.fallback.family_missing);
    assert_eq!(raster.layout.family, DEFAULT_FAMILY);
    let noto = r.rasterize(&spec("Fallback"), 1.0);
    assert!(!noto.layout.fallback.family_missing);
    assert_eq!(raster.image.rgba, noto.image.rgba);
    // Family names match case-insensitively.
    let lower = r.layout(&TextSpec {
        family: "noto sans".into(),
        ..spec("x")
    });
    assert!(!lower.fallback.family_missing);
}

#[test]
fn characters_no_font_has_are_reported() {
    let mut r = renderer();
    // The bundled fonts cover Latin, Greek and Cyrillic, not Han.
    let layout = r.layout(&spec("Ωж漢字 ok 漢"));
    assert_eq!(layout.fallback.missing_chars, vec!['漢', '字']);
    let covered = r.layout(&spec("Ωж ok"));
    assert!(covered.fallback.missing_chars.is_empty());
}

#[test]
fn empty_text_has_no_pixels() {
    let mut r = renderer();
    for text in ["", " ", "\n", "   \n  "] {
        let raster = r.rasterize(&spec(text), 2.0);
        assert!(raster.image.is_empty(), "{text:?}");
        assert_eq!(raster.layout.size.0, 0.0, "{text:?}");
    }
    let transparent = r.rasterize(
        &TextSpec {
            fill: Fill::Solid([255, 255, 255, 0]),
            ..spec("ink")
        },
        1.0,
    );
    assert!(transparent.image.is_empty());
}

#[test]
fn rasters_scale_and_are_cached() {
    let mut r = renderer();
    let s = spec("Scale me");
    let one = r.rasterize(&s, 1.0);
    let again = r.rasterize(&s, 1.0);
    assert!(Arc::ptr_eq(&one, &again));
    let two = r.rasterize(&s, 2.0);
    assert!(!Arc::ptr_eq(&one, &two));
    assert_eq!(two.scale, 2.0);
    let ratio = f64::from(two.image.width) / f64::from(one.image.width);
    assert!((1.9..2.1).contains(&ratio), "{ratio}");
    // Layout doesn't depend on the raster scale.
    assert_eq!(one.layout, two.layout);
}

#[test]
fn rasters_are_deterministic_and_premultiplied() {
    let s = TextSpec {
        fill: Fill::Solid([200, 40, 10, 255]),
        ..spec("Determinism äöü")
    };
    let a = renderer().rasterize(&s, 1.37);
    let b = renderer().rasterize(&s, 1.37);
    assert_eq!(a.image.rgba, b.image.rgba);
    assert_eq!(a.origin, b.origin);
    // Premultiplied (C46): every pixel is the fill colour times its
    // coverage.
    let mul = |c: u8, a: u8| (f64::from(c) * f64::from(a) / 255.0).round() as i32;
    assert!(a.image.rgba.chunks(4).all(|p| {
        [200u8, 40, 10]
            .iter()
            .zip(&p[..3])
            .all(|(&c, &v)| (i32::from(v) - mul(c, p[3])).abs() <= 1)
    }));
    assert!(a.image.rgba.chunks(4).any(|p| p[3] == 255));
    assert!(a.image.rgba.chunks(4).any(|p| p[3] > 0 && p[3] < 255));
}

#[test]
fn golden_stem_of_a_capital_i() {
    // Noto Sans Regular "I" at 100 px: one vertical stem ≈ 9 px wide (8 solid
    // columns plus antialiased edges), standing on the baseline, cap height
    // ≈ 71 px.
    let mut r = renderer();
    let raster = r.rasterize(
        &TextSpec {
            size_px: 100.0,
            ..TextSpec::new("I")
        },
        1.0,
    );
    let layout = &raster.layout;
    let baseline = (layout.lines[0].baseline.round() as i32 + raster.origin.1) as u32;
    let img = &raster.image;
    let mid = baseline - 35;
    let solid: Vec<u32> = (0..img.width)
        .filter(|&x| alpha(&raster, x, mid) == 255)
        .collect();
    assert!((7..=10).contains(&solid.len()), "{solid:?}");
    assert!(solid.windows(2).all(|w| w[1] == w[0] + 1), "{solid:?}");
    let coverage: f64 = (0..img.width)
        .map(|x| f64::from(alpha(&raster, x, mid)) / 255.0)
        .sum();
    assert!((8.5..=9.5).contains(&coverage), "stem width {coverage}");
    // Ink stops at the baseline and starts near the cap height.
    let x = solid[solid.len() / 2];
    assert_eq!(alpha(&raster, x, baseline - 1), 255);
    assert_eq!(alpha(&raster, x, baseline + 1), 0);
    let ink_top = (0..img.height).find(|&y| alpha(&raster, x, y) > 0).unwrap();
    let cap = baseline - ink_top;
    assert!((70..=73).contains(&cap), "cap height {cap}");
}

#[test]
fn trailing_spaces_do_not_count_for_width_or_alignment() {
    let mut r = renderer();
    let plain = r.layout(&spec("Hi"));
    let spaced = r.layout(&spec("Hi   "));
    assert_eq!(plain.size, spaced.size);
    let centered = |r: &mut TextRenderer, text: &str| {
        r.layout(&TextSpec {
            align: TextAlign::Center,
            box_width_px: Some(800.0),
            ..spec(text)
        })
        .lines[0]
    };
    assert_eq!(centered(&mut r, "Hi"), centered(&mut r, "Hi   "));
    // Leading spaces are content.
    let indented = r.layout(&spec("  Hi"));
    assert!(indented.size.0 > plain.size.0);
}

#[test]
fn system_fonts_are_discovered_off_the_calling_thread() {
    use std::time::{Duration, Instant};

    use freecut_render::text::ScanState;
    let start = Instant::now();
    let fonts = FontRegistry::with_system_fonts();
    // The bundled fonts are usable at once, whatever the scan is doing.
    assert!(fonts.has_family(DEFAULT_FAMILY));
    let created = start.elapsed();
    let state = fonts.wait_for_scan(Duration::from_secs(120));
    assert_eq!(state, ScanState::Complete);
    assert!(
        created < Duration::from_millis(500),
        "creating the registry blocked for {created:?}"
    );
    let names = fonts.family_names();
    assert!(names.iter().any(|n| n == DEFAULT_FAMILY));
    assert!(names.iter().all(|n| !n.starts_with('.')));
    let lower: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    assert!(
        lower.windows(2).all(|w| w[0] < w[1]),
        "not sorted and unique"
    );
    // A renderer made before the scan picks up the new font set.
    assert!(fonts.generation() > 1);
}

/// Overlapping glyphs (a combining mark over its base, joined underscores)
/// must not double the opacity of translucent text: the fill alpha is
/// applied once to the combined coverage.
#[test]
fn translucent_text_has_one_alpha_where_glyphs_overlap() {
    let mut r = renderer();
    for text in ["o\u{336}", "___"] {
        let raster = r.rasterize(
            &TextSpec {
                fill: Fill::Solid([255, 255, 255, 128]),
                ..spec(text)
            },
            1.0,
        );
        let max = raster.image.rgba.chunks(4).map(|p| p[3]).max().unwrap();
        assert!(max <= 129, "{text:?}: max alpha {max}");
    }
}

/// Abutting glyph edges add up to full coverage: an opaque run of
/// underscores at a fractional scale has no seams at the joins.
#[test]
fn abutting_glyphs_leave_no_seams() {
    let mut r = renderer();
    let raster = r.rasterize(&spec("______"), 1.13);
    let img = &raster.image;
    // The row through the middle of the stroke: the one with the most
    // fully covered pixels.
    let row = (0..img.height)
        .max_by_key(|&y| {
            (0..img.width)
                .filter(|&x| alpha(&raster, x, y) == 255)
                .count()
        })
        .unwrap();
    let covered: Vec<u32> = (0..img.width)
        .filter(|&x| alpha(&raster, x, row) == 255)
        .collect();
    let (first, last) = (covered[0], *covered.last().unwrap());
    let dips: Vec<(u32, u8)> = (first..=last)
        .map(|x| (x, alpha(&raster, x, row)))
        .filter(|&(_, a)| a < 250)
        .collect();
    assert!(dips.is_empty(), "seams at {dips:?}");
}
