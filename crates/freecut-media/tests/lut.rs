//! LUT parsing (CL-016) against FreeCut's own hand-made fixtures in
//! `tests/fixtures/lut/`: the valid ones parse to the expected tables, and
//! every file in `bad/` breaks exactly one validation rule and is rejected
//! with that rule and its line. Runs without the `ffmpeg` feature.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::path::PathBuf;

use freecut_media::lut::{
    Lut, LutError, LutErrorKind, LutNotice, Malformed, Unsupported, parse_3dl, parse_cube,
};
use proptest::prelude::*;

fn fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", "lut", name]
        .iter()
        .collect();
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn parse(name: &str) -> Result<Lut, LutError> {
    let bytes = fixture(name);
    if name.ends_with(".3dl") {
        parse_3dl(&bytes)
    } else {
        parse_cube(&bytes)
    }
}

fn ok(name: &str) -> Lut {
    parse(name).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
    (0..3).all(|c| (a[c] - b[c]).abs() <= tol)
}

/// Grouped under `lut` so `cargo test -p freecut-media lut` selects them.
mod lut {
    /// Valid `.cube` files.
    mod cube {
        use super::super::*;
        use pretty_assertions::assert_eq;

        #[test]
        fn identity_2() {
            let lut = ok("identity_2.cube");
            assert_eq!(lut.title.as_deref(), Some("FreeCut identity 2"));
            assert!(lut.shaper.is_none());
            let cube = lut.cube.as_ref().unwrap();
            assert_eq!(cube.size(), 2);
            assert_eq!(cube.data()[1], [1.0, 0.0, 0.0], "red varies fastest");
            assert_eq!(cube.data()[2], [0.0, 1.0, 0.0]);
            assert_eq!(cube.data()[4], [0.0, 0.0, 1.0]);
            assert_eq!((lut.domain_min, lut.domain_max), ([0.0; 3], [1.0; 3]));
            assert_eq!(lut.skipped_keywords, 0);
            assert!(lut.notices.is_empty());
            assert!(lut.is_identity());
        }

        #[test]
        fn invert_17() {
            let lut = ok("invert_17.cube");
            let cube = lut.cube.as_ref().unwrap();
            assert_eq!(cube.size(), 17);
            assert_eq!(cube.at(0, 0, 0), [1.0, 1.0, 1.0]);
            assert_eq!(cube.at(16, 4, 0), [0.0, 0.75, 1.0]);
            for rgb in [[0.1, 0.5, 0.9], [0.33, 0.66, 0.99], [1.0, 0.0, 0.25]] {
                let want = rgb.map(|v: f32| 1.0 - v);
                assert!(close(lut.apply_tetrahedral(rgb), want, 1e-6), "{rgb:?}");
                assert!(close(lut.apply_trilinear(rgb), want, 1e-6), "{rgb:?}");
            }
            assert!(!lut.is_identity());
        }

        #[test]
        fn ramp_1d_1024() {
            let lut = ok("ramp_1d_1024.cube");
            assert!(lut.cube.is_none());
            let shaper = lut.shaper.as_ref().unwrap();
            assert_eq!(shaper.size(), 1024);
            // Each channel is looked up on its own column.
            let out = lut.apply_tetrahedral([0.5, 0.5, 0.5]);
            assert!(close(out, [0.5, 0.25, 0.5], 1e-5), "{out:?}");
            let out = lut.apply_trilinear([1.0, 0.2, 0.0]);
            assert!(close(out, [1.0, 0.04, 1.0], 1e-5), "{out:?}");
        }

        #[test]
        fn shaper_then_3d() {
            let lut = ok("shaper_3d.cube");
            assert_eq!(lut.shaper.as_ref().unwrap().size(), 3);
            assert_eq!(lut.cube.as_ref().unwrap().size(), 2);
            // 0.5 → shaper 0.25 → invert 0.75.
            let out = lut.apply_tetrahedral([0.5, 0.0, 1.0]);
            assert!(close(out, [0.75, 1.0, 0.0], 1e-6), "{out:?}");
        }

        #[test]
        fn domain_0_2() {
            let lut = ok("domain_0_2.cube");
            assert_eq!((lut.domain_min, lut.domain_max), ([0.0; 3], [2.0; 3]));
            assert!(close(
                lut.apply_tetrahedral([1.0, 2.0, 3.0]),
                [0.5, 1.0, 1.0],
                1e-6
            ));
            assert!(!lut.is_identity(), "a 0–2 domain halves values");
        }

        #[test]
        fn input_range_matches_domain() {
            assert_eq!(ok("input_range.cube"), ok("domain_0_2.cube"));
        }

        #[test]
        fn crlf_bom_whitespace() {
            let lut = ok("crlf_bom.cube");
            assert_eq!(lut.title.as_deref(), Some("FreeCut CRLF"));
            let mut want = ok("identity_2.cube");
            want.title = lut.title.clone();
            assert_eq!(lut, want);
        }

        #[test]
        fn comments_and_skipped_keywords() {
            let lut = ok("comments_title.cube");
            assert_eq!(lut.title.as_deref(), Some("Warm #2 (FreeCut)"));
            assert_eq!(lut.skipped_keywords, 2);
            assert!(lut.is_identity());
        }

        #[test]
        fn latin1_title() {
            let lut = ok("latin1_title.cube");
            assert_eq!(lut.title.as_deref(), Some("Café crème"));
        }

        #[test]
        fn outputs_outside_0_1_are_kept() {
            let lut = ok("out_of_range.cube");
            let cube = lut.cube.as_ref().unwrap();
            assert_eq!(cube.at(0, 0, 0), [-0.1, 0.0, 0.0]);
            assert_eq!(cube.at(1, 1, 1), [1.5, 1.25, -0.2]);
            assert_eq!(lut.apply_tetrahedral([1.0; 3]), [1.5, 1.25, -0.2]);
        }

        #[test]
        fn blank_title_is_none() {
            let lut = parse_cube(b"TITLE \"  \"\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n").unwrap();
            assert_eq!(lut.title, None);
        }

        #[test]
        fn large_3d_tables_are_resampled_to_65() {
            let n = 130usize;
            let mut text = String::with_capacity(n * n * n * 24);
            text.push_str("LUT_3D_SIZE 130\n");
            let last = (n - 1) as f32;
            for b in 0..n {
                for g in 0..n {
                    for r in 0..n {
                        // A linear grade, which trilinear resampling keeps exactly.
                        let (r, g, b) = (r as f32 / last, g as f32 / last, b as f32 / last);
                        text.push_str(&format!("{} {} {}\n", 0.5 * r + 0.25, g, 1.0 - b));
                    }
                }
            }
            let lut = parse_cube(text.as_bytes()).unwrap();
            assert_eq!(
                lut.notices,
                vec![LutNotice::Resampled { from: 130, to: 65 }]
            );
            let cube = lut.cube.as_ref().unwrap();
            assert_eq!(cube.size(), 65);
            for rgb in [[0.0, 0.0, 0.0], [1.0, 0.5, 0.25], [0.3, 0.7, 0.9]] {
                let want = [0.5 * rgb[0] + 0.25, rgb[1], 1.0 - rgb[2]];
                assert!(close(lut.apply_tetrahedral(rgb), want, 1e-5), "{rgb:?}");
            }
        }
    }

    /// Valid `.3dl` files.
    mod three_dl {
        use super::super::*;
        use pretty_assertions::assert_eq;

        #[test]
        fn rotate_17_10bit_is_reordered_red_fastest() {
            let lut = ok("rotate_17_10bit.3dl");
            assert_eq!(lut.title, None);
            assert!(lut.notices.is_empty(), "0, 64, …, 960, 1023 counts as even");
            let cube = lut.cube.as_ref().unwrap();
            assert_eq!(cube.size(), 17);
            // The fixture maps (r, g, b) to (g, b, r) at mesh values / 1023.
            let mesh = |i: usize| ((i * 64).min(1023)) as f32 / 1023.0;
            for (r, g, b) in [(1, 2, 3), (16, 0, 5), (0, 16, 9), (7, 7, 16)] {
                assert_eq!(cube.at(r, g, b), [mesh(g), mesh(b), mesh(r)], "{r} {g} {b}");
            }
        }

        #[test]
        fn identity_3_12bit() {
            let lut = ok("identity_3_12bit.3dl");
            let cube = lut.cube.as_ref().unwrap();
            assert_eq!(cube.size(), 3);
            assert_eq!(cube.at(2, 2, 2), [1.0; 3]);
            assert_eq!(cube.at(1, 0, 2), [2048.0 / 4095.0, 0.0, 1.0]);
        }

        #[test]
        fn bit_depth_comes_from_the_largest_output() {
            let rows = |v: &str| format!("0 1023\n{}", format!("0 0 0\n0 0 {v}\n").repeat(4));
            let lut = parse_3dl(rows("1023").as_bytes()).unwrap();
            assert_eq!(lut.cube.unwrap().at(1, 1, 1), [0.0, 0.0, 1.0]);
            let lut = parse_3dl(rows("4000").as_bytes()).unwrap();
            assert_eq!(lut.cube.unwrap().at(1, 1, 1), [0.0, 0.0, 4000.0 / 4095.0]);
            let lut = parse_3dl(rows("65535").as_bytes()).unwrap();
            assert_eq!(lut.cube.unwrap().at(1, 1, 1), [0.0, 0.0, 1.0]);
        }

        #[test]
        fn float_3dl() {
            let lut = ok("invert_2_float.3dl");
            let cube = lut.cube.as_ref().unwrap();
            assert_eq!(cube.at(0, 0, 0), [1.0, 1.0, 1.0]);
            assert_eq!(cube.at(1, 0, 0), [0.0, 1.0, 1.0]);
            assert_eq!(cube.at(0, 0, 1), [1.0, 1.0, 0.0]);
        }

        #[test]
        fn uneven_mesh_is_resampled_to_even() {
            let lut = ok("uneven_mesh.3dl");
            assert_eq!(lut.notices, vec![LutNotice::MeshResampled { size: 3 }]);
            // Identity at the mesh positions stays identity on the even lattice.
            let cube = lut.cube.as_ref().unwrap();
            assert!(
                close(cube.at(1, 1, 1), [0.5; 3], 1e-6),
                "{:?}",
                cube.at(1, 1, 1)
            );
            assert!(close(cube.at(2, 1, 0), [1.0, 0.5, 0.0], 1e-6));
            assert!(lut.is_identity());
        }
    }

    /// One rejected fixture per validation rule.
    mod rejected {
        use super::super::*;
        use pretty_assertions::assert_eq;

        fn bad(reason: Malformed) -> LutErrorKind {
            LutErrorKind::Malformed(reason)
        }

        fn unsupported(reason: Unsupported) -> LutErrorKind {
            LutErrorKind::Unsupported(reason)
        }

        fn token(s: &str) -> String {
            s.to_owned()
        }

        /// One fixture per CL-016 validation rule: the line and the reason.
        fn rejected() -> Vec<(&'static str, Option<u32>, LutErrorKind)> {
            use Malformed as M;
            use Unsupported as U;
            vec![
                // .cube
                (
                    "unexpected_text.cube",
                    Some(3),
                    bad(M::UnexpectedText {
                        text: token("hello world"),
                    }),
                ),
                (
                    "not_a_number.cube",
                    Some(4),
                    bad(M::NotANumber { token: token("x") }),
                ),
                (
                    "not_finite.cube",
                    Some(8),
                    bad(M::NotFinite {
                        token: token("nan"),
                    }),
                ),
                ("value_count.cube", Some(5), bad(M::ValueCount { found: 2 })),
                (
                    "keyword_after_data.cube",
                    Some(7),
                    bad(M::KeywordAfterData {
                        keyword: token("TITLE"),
                    }),
                ),
                (
                    "duplicate_keyword.cube",
                    Some(3),
                    bad(M::DuplicateKeyword {
                        keyword: "LUT_3D_SIZE",
                    }),
                ),
                (
                    "argument_count.cube",
                    Some(3),
                    bad(M::ArgumentCount {
                        keyword: "DOMAIN_MIN",
                        expected: 3,
                        found: 2,
                    }),
                ),
                ("title_not_quoted.cube", Some(2), bad(M::TitleNotQuoted)),
                (
                    "title_too_long.cube",
                    Some(3),
                    bad(M::TitleTooLong { chars: 256 }),
                ),
                (
                    "size_not_integer.cube",
                    Some(2),
                    bad(M::SizeNotInteger {
                        keyword: "LUT_3D_SIZE",
                        token: token("2.5"),
                    }),
                ),
                (
                    "size_too_small.cube",
                    Some(2),
                    bad(M::SizeOutOfRange {
                        keyword: "LUT_3D_SIZE",
                        size: 1,
                        min: 2,
                        max: 256,
                    }),
                ),
                (
                    "size_too_large.cube",
                    Some(3),
                    bad(M::SizeOutOfRange {
                        keyword: "LUT_3D_SIZE",
                        size: 257,
                        min: 2,
                        max: 256,
                    }),
                ),
                (
                    "size_1d_too_large.cube",
                    Some(2),
                    bad(M::SizeOutOfRange {
                        keyword: "LUT_1D_SIZE",
                        size: 65_537,
                        min: 2,
                        max: 65_536,
                    }),
                ),
                ("missing_size.cube", Some(4), bad(M::MissingSize)),
                (
                    "empty_domain.cube",
                    Some(4),
                    bad(M::EmptyDomain {
                        keyword: "DOMAIN_MAX",
                        channel: Some("green"),
                    }),
                ),
                (
                    "empty_input_range.cube",
                    Some(3),
                    bad(M::EmptyDomain {
                        keyword: "LUT_3D_INPUT_RANGE",
                        channel: None,
                    }),
                ),
                (
                    "range_without_table.cube",
                    Some(2),
                    bad(M::RangeWithoutTable {
                        keyword: "LUT_1D_INPUT_RANGE",
                        table: "1D",
                    }),
                ),
                (
                    "too_few_rows.cube",
                    Some(3),
                    bad(M::TooFewRows {
                        expected: 8,
                        found: 7,
                    }),
                ),
                (
                    "too_many_rows.cube",
                    Some(11),
                    bad(M::TooManyRows { expected: 8 }),
                ),
                ("decimal_comma.cube", Some(4), LutErrorKind::DecimalComma),
                (
                    "mixed_domain.cube",
                    Some(4),
                    unsupported(U::MixedDomainKeywords),
                ),
                (
                    "shaper_input_range.cube",
                    Some(5),
                    unsupported(U::ShaperInputRange),
                ),
                // .3dl
                ("missing_mesh.3dl", None, bad(M::MissingMesh)),
                (
                    "mesh_header.3dl",
                    Some(2),
                    unsupported(U::MeshHeader {
                        keyword: token("3DMESH"),
                    }),
                ),
                ("mesh_size.3dl", Some(3), bad(M::MeshSize { found: 1 })),
                ("mesh_start.3dl", Some(2), bad(M::MeshStart { first: 1 })),
                (
                    "mesh_not_increasing.3dl",
                    Some(2),
                    bad(M::MeshNotIncreasing { position: 3 }),
                ),
                (
                    "mesh_not_integer.3dl",
                    Some(2),
                    bad(M::NotAnInteger {
                        token: token("511.5"),
                    }),
                ),
                (
                    "mesh_depth.3dl",
                    Some(2),
                    unsupported(U::MeshDepth { last: 255 }),
                ),
                (
                    "mesh_mismatch.3dl",
                    Some(2),
                    unsupported(U::MeshMismatch {
                        points: 2,
                        expected: 8,
                        found: 27,
                    }),
                ),
                (
                    "data_not_integer.3dl",
                    Some(4),
                    bad(M::NotAnInteger { token: token("-5") }),
                ),
                (
                    "data_too_large.3dl",
                    Some(6),
                    bad(M::ValueTooLarge { value: 70_000 }),
                ),
                (
                    "data_value_count.3dl",
                    Some(3),
                    bad(M::ValueCount { found: 4 }),
                ),
                (
                    "data_not_a_number.3dl",
                    Some(5),
                    bad(M::NotANumber {
                        token: token("red"),
                    }),
                ),
                (
                    "data_decimal_comma.3dl",
                    Some(10),
                    LutErrorKind::DecimalComma,
                ),
                (
                    "data_not_finite.3dl",
                    Some(4),
                    bad(M::NotFinite {
                        token: token("1.0e39"),
                    }),
                ),
            ]
        }

        #[test]
        fn every_rule_is_rejected_with_its_line() {
            for (name, line, kind) in rejected() {
                let err = parse(&format!("bad/{name}"))
                    .err()
                    .unwrap_or_else(|| panic!("{name} parsed"));
                assert_eq!((name, err.line, err.kind), (name, line, kind));
            }
        }

        #[test]
        fn every_bad_fixture_is_listed() {
            let dir: PathBuf = [
                env!("CARGO_MANIFEST_DIR"),
                "tests",
                "fixtures",
                "lut",
                "bad",
            ]
            .iter()
            .collect();
            let mut on_disk: Vec<String> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            on_disk.sort();
            let mut listed: Vec<String> = rejected().iter().map(|r| r.0.to_owned()).collect();
            listed.sort();
            assert_eq!(on_disk, listed);
        }

        #[test]
        fn messages_name_the_line_and_reason() {
            let msg = |name: &str| parse(&format!("bad/{name}")).unwrap_err().to_string();
            assert_eq!(
                msg("value_count.cube"),
                "line 5: expected 3 numbers, found 2"
            );
            assert_eq!(
                msg("too_few_rows.cube"),
                "line 3: 8 data lines expected, 7 found"
            );
            assert_eq!(
                msg("decimal_comma.cube"),
                "line 4: numbers use a comma as the decimal mark; LUT files need a point (0.5, not 0,5)"
            );
            assert_eq!(
                msg("mesh_mismatch.3dl"),
                "line 2: unsupported LUT: the input mesh has 2 points, so 8 data lines are expected; 27 found"
            );
            assert_eq!(msg("missing_mesh.3dl"), "no input mesh line");
        }

        #[test]
        fn a_file_with_no_table_is_missing_its_size() {
            let err = parse_cube(b"# nothing here\nTITLE \"empty\"\n").unwrap_err();
            assert_eq!((err.line, err.kind), (None, bad(Malformed::MissingSize)));
            let err = parse_cube(b"").unwrap_err();
            assert_eq!(err.kind, bad(Malformed::MissingSize));
        }

        #[test]
        fn a_declared_table_with_no_data_is_short() {
            let err = parse_cube(b"LUT_1D_SIZE 4\n").unwrap_err();
            assert_eq!(
                (err.line, err.kind),
                (
                    Some(1),
                    bad(Malformed::TooFewRows {
                        expected: 4,
                        found: 0
                    })
                )
            );
        }
    }

    /// Robustness and round trips.
    mod properties {
        use super::super::*;

        /// Writes a `.cube` the way an exporting tool would.
        fn write_cube(title: &str, n: usize, data: &[[f32; 3]]) -> String {
            let mut s = format!("TITLE \"{title}\"\nLUT_3D_SIZE {n}\n");
            for v in data {
                s.push_str(&format!("{} {} {}\n", v[0], v[1], v[2]));
            }
            s
        }

        proptest! {
            #[test]
            fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
                let _ = parse_cube(&bytes);
                let _ = parse_3dl(&bytes);
            }

            #[test]
            fn arbitrary_lines_never_panic(
                lines in proptest::collection::vec(
                    prop_oneof![
                        Just("LUT_3D_SIZE 2".to_owned()),
                        Just("LUT_1D_SIZE 2".to_owned()),
                        Just("DOMAIN_MIN 0 0 0".to_owned()),
                        Just("DOMAIN_MAX 1 1 1".to_owned()),
                        Just("LUT_3D_INPUT_RANGE 0 1".to_owned()),
                        Just("TITLE \"x\"".to_owned()),
                        Just("0 1023".to_owned()),
                        Just("0 0 0".to_owned()),
                        Just("1 1 1".to_owned()),
                        Just("1023 0 512".to_owned()),
                        Just("# c".to_owned()),
                        Just(String::new()),
                        "[ -~]{0,12}",
                    ],
                    0..40,
                )
            ) {
                let text = lines.join("\n");
                let _ = parse_cube(text.as_bytes());
                let _ = parse_3dl(text.as_bytes());
            }

            #[test]
            fn written_cubes_read_back(
                n in 2usize..6,
                seed in proptest::collection::vec(-2.0f32..2.0, 3 * 5 * 5 * 5),
                title in "[A-Za-z0-9 ]{1,20}",
            ) {
                let data: Vec<[f32; 3]> = (0..n * n * n)
                    .map(|i| [seed[3 * i], seed[3 * i + 1], seed[3 * i + 2]])
                    .collect();
                let lut = parse_cube(write_cube(&title, n, &data).as_bytes()).unwrap();
                prop_assert_eq!(lut.title.as_deref(), Some(title.trim()).filter(|t| !t.is_empty()));
                let cube = lut.cube.unwrap();
                prop_assert_eq!(cube.size() as usize, n);
                // `{}` prints the shortest text that reads back to the same f32.
                prop_assert_eq!(cube.data(), &data[..]);
            }
        }
    }
}
