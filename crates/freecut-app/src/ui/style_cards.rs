//! The Style lens's cards (TX-054, ui-concept §7.1): **Type** (TX-003,
//! TX-004, TX-008), **Fill** (TX-004, TX-014), **Spacing** (TX-009), and
//! the optional layers **Outline** (TX-010), **Shadow** (TX-011) and
//! **Glow** (TX-012). The card frames, switches and menus are the
//! inspector's; these draw the rows and commit what they asked for.

use eframe::egui::{self, Response};
use freecut_timeline::edit::{TextLayoutEdit, TextStyleEdit};
use freecut_timeline::model::{
    CaseTransform, DeciDegrees, Fill, FillCenter, FillExtent, FillKind, FontRef, Glow,
    GlowDirection, GradientStop, GradientStops, PerMille, PropPath, Rgba8, Shadow, SignedPerMille,
    Stroke, StrokePosition, TextClipData, TextLayout, TextStyle,
};

use super::inspector::lens::CardId;
use super::props::{
    CardEnv, Edit, FontChoices, Merge, PropsState, Steps, Texts, differ, each_style,
};
use super::rows::{self, At, Numeric, hex, ref_px, signed_per_mille_in};
use crate::theme::icons;
use crate::widgets::card::{self, CardMemory};
use crate::widgets::field::MIXED;
use crate::widgets::gradient::{BarStop, gradient_bar};
use crate::widgets::icon_button;
use crate::widgets::icon_button::IconButton;
use crate::widgets::row::PropRow;
use crate::widgets::segmented::Segment;
use crate::widgets::select::{self, Select, SelectOption};

fn style(edit: TextStyleEdit) -> Edit {
    Edit::Style(edit)
}

/// The Type card: font, size with steppers, emphasis (B, I, U, S) and
/// case; behind More, the face and Sentence case.
pub fn type_card(
    ui: &mut egui::Ui,
    env: &mut CardEnv<'_>,
    state: &mut PropsState,
    s: &TextStyle,
    fonts: &FontChoices,
    memory: &mut CardMemory,
) -> Option<String> {
    let defaults = TextStyle::default();
    let at = env.at(CardId::Type);
    let mut steps = Steps::default();
    let font_mixed = env.differ(|d| d.style.font.family.to_lowercase());
    let font_row = PropRow::new(at.id("font"), "Font")
        .changed(font_mixed || !s.font.family.eq_ignore_ascii_case(&defaults.font.family))
        .show(ui, |ui| {
            if fonts.scanning {
                // The spinner sits after the picker; keep its room.
                ui.set_max_width(ui.available_width() - 22.0);
            }
            let picked = font_picker(ui, at.id("font"), state, fonts, &s.font.family, font_mixed);
            if fonts.scanning {
                ui.spinner()
                    .on_hover_text("Looking for the fonts on this computer");
            }
            picked
        });
    if font_row.reset {
        steps.edit(
            style(TextStyleEdit::Font(defaults.font.clone())),
            Merge::Own,
        );
    }
    if let Some(family) = font_row.inner {
        steps.edit(
            style(TextStyleEdit::Font(FontRef::new(family))),
            Merge::WhileHeld,
        );
    }

    let stored = s.size;
    let n = Numeric::new(
        "size",
        "Size",
        stored.as_f64(),
        TextStyle::DEFAULT_SIZE.as_f64(),
        TextStyle::MIN_SIZE.as_f64()..=TextStyle::MAX_SIZE.as_f64(),
    )
    .soft(8.0..=400.0)
    .logarithmic()
    .suffix("px")
    .stepper(1.0)
    .headline();
    let size_at = at.mixed(env.differ(|d| d.style.size));
    rows::numeric(ui, size_at, n, &mut steps, |v| {
        let wanted = TextStyle::clamp_size(ref_px(v));
        (size_at.mixed || wanted != stored).then(|| style(TextStyleEdit::Size(wanted)))
    });

    let emphasis = PropRow::new(at.id("emphasis"), "Emphasis").show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        [
            emphasis_button(ui, icons::BOLD, s.bold, "Bold"),
            emphasis_button(ui, icons::ITALIC, s.italic, "Italic"),
            emphasis_button(ui, icons::UNDERLINE, s.decoration.underline, "Underline"),
            emphasis_button(ui, icons::STRIKE, s.decoration.strike, "Strikethrough"),
        ]
    });
    let [bold, italic, underline, strike] = emphasis.inner;
    let flip = |r: &Response, edit: TextStyleEdit| r.clicked().then(|| style(edit));
    let e = flip(&bold, TextStyleEdit::Bold(!s.bold));
    steps.button(bold, e);
    let e = flip(&italic, TextStyleEdit::Italic(!s.italic));
    steps.button(italic, e);
    let e = flip(
        &underline,
        TextStyleEdit::Underline(!s.decoration.underline),
    );
    steps.button(underline, e);
    let e = flip(&strike, TextStyleEdit::Strike(!s.decoration.strike));
    steps.button(strike, e);

    // TX-008: four one-click cases; Sentence case lights none of them.
    let items = [
        Segment {
            icon: None,
            label: "Aa",
            tooltip: Some("As typed"),
        },
        Segment {
            icon: None,
            label: "AA",
            tooltip: Some("UPPERCASE (display only; the text is kept as typed)"),
        },
        Segment {
            icon: None,
            label: "aa",
            tooltip: Some("lowercase (display only)"),
        },
        Segment {
            icon: None,
            label: "Ab",
            tooltip: Some("Title Case (display only)"),
        },
    ];
    let label = if s.case == CaseTransform::Sentence {
        "Sentence case"
    } else {
        "Case"
    };
    rows::choice(
        ui,
        at.mixed(env.differ(|d| d.style.case)),
        &mut steps,
        "case",
        label,
        &items,
        &[
            CaseTransform::AsTyped,
            CaseTransform::Upper,
            CaseTransform::Lower,
            CaseTransform::Title,
        ],
        s.case,
        defaults.case,
        false,
        |c| style(TextStyleEdit::Case(c)),
    );

    let face_mixed = env.differ(|d| (d.style.bold, d.style.italic));
    card::more(ui, at.id("more"), memory, env.everything, 2, |ui| {
        face_row(ui, at.mixed(face_mixed), &mut steps, s);
        let sentence = PropRow::new(at.id("sentence"), "Sentence")
            .hint("Capitalize the first letter of each sentence (display only)")
            .show(ui, |ui| {
                ui.add(
                    egui::Button::new("Sentence case").selected(s.case == CaseTransform::Sentence),
                )
            });
        let response = sentence.inner;
        let wanted = if s.case == CaseTransform::Sentence {
            CaseTransform::AsTyped
        } else {
            CaseTransform::Sentence
        };
        let e = response
            .clicked()
            .then(|| style(TextStyleEdit::Case(wanted)));
        steps.button(response, e);
    });
    env.commit(ui, steps)
}

/// The face dropdown (TX-003). FreeCut's font reference names a family;
/// its faces are the bold and italic switches, so this offers the four
/// combinations and a pick is one entry.
fn face_row(ui: &mut egui::Ui, at: At<'_>, steps: &mut Steps, s: &TextStyle) {
    const FACES: [(&str, bool, bool); 4] = [
        ("Regular", false, false),
        ("Italic", false, true),
        ("Bold", true, false),
        ("Bold italic", true, true),
    ];
    let current = FACES
        .iter()
        .find(|(_, b, i)| *b == s.bold && *i == s.italic)
        .filter(|_| !at.mixed)
        .map_or(if at.mixed { MIXED } else { "Regular" }, |(name, _, _)| {
            name
        });
    let options = FACES.map(|(name, _, _)| SelectOption::new(name));
    let index = FACES
        .iter()
        .position(|(_, b, i)| *b == s.bold && *i == s.italic)
        .filter(|_| !at.mixed);
    let row = PropRow::new(at.id("face"), "Face").show(ui, |ui| {
        Select::new(at.id("face").with("select"), &options, index)
            .text(current)
            .fill_width()
            .show(ui)
            .picked
            .and_then(|i| FACES.get(i))
            .map(|(_, bold, italic)| (*bold, *italic))
    });
    if let Some((bold, italic)) = row.inner {
        let mut edits = Vec::new();
        if at.mixed || bold != s.bold {
            edits.push(style(TextStyleEdit::Bold(bold)));
        }
        if at.mixed || italic != s.italic {
            edits.push(style(TextStyleEdit::Italic(italic)));
        }
        steps.group(edits);
    }
}

/// One emphasis switch: an icon button, checked while on (ui-concept
/// §13.18: a well chip and an `accent.text` glyph, never colour alone).
fn emphasis_button(ui: &mut egui::Ui, icon: &str, on: bool, tip: &str) -> Response {
    IconButton::new(icon, tip).checked(on).show(ui)
}

/// The family picker with a search box (TX-003), as a select (ui-concept
/// §13.20): the bundled default first, then every other family. A family
/// that isn't installed shows "Missing · ‹name›" on a `danger`-tinted
/// trigger. Returns a newly picked family; with `mixed` families any pick
/// is new.
fn font_picker(
    ui: &mut egui::Ui,
    id: egui::Id,
    state: &mut PropsState,
    fonts: &FontChoices,
    current: &str,
    mixed: bool,
) -> Option<String> {
    let mut picked = None;
    let missing = !mixed && !fonts.has(current) && !fonts.scanning;
    let shown = if mixed {
        MIXED.to_owned()
    } else if missing {
        format!("Missing · {current}")
    } else {
        current.to_owned()
    };
    let list_id = id.with("select");
    let open = egui::Popup::is_id_open(ui.ctx(), list_id.with("list"));
    let width = ui.available_width();
    let trigger = select::trigger(ui, list_id, &shown, width, open, missing);
    let trigger = if missing {
        trigger
            .on_hover_text("This font isn't on this computer; the text shows in the default font")
    } else {
        trigger
    };
    select::custom_list(ui, &trigger, 220.0, |ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.font_search)
                .hint_text("Search fonts")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let needle = state.font_search.to_lowercase();
        let mut close = false;
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for family in fonts.ordered() {
                    if !needle.is_empty() && !family.to_lowercase().contains(&needle) {
                        continue;
                    }
                    let selected = !mixed && family.eq_ignore_ascii_case(current);
                    let option = SelectOption::new(family);
                    let row = select::row(ui, list_id.with(family), &option, selected, false);
                    if row.clicked() {
                        close = true;
                        if !selected {
                            picked = Some(family.to_owned());
                        }
                    }
                }
            });
        close
    });
    picked
}

/// The Fill card (TX-004, TX-014): solid colour, or a linear or radial
/// gradient with its stop bar, angle, centre and spread; behind More, the
/// gradient's extent.
pub fn fill_card(
    ui: &mut egui::Ui,
    env: &mut CardEnv<'_>,
    state: &mut PropsState,
    s: &TextStyle,
    memory: &mut CardMemory,
) -> Option<String> {
    let at = env.at(CardId::Fill);
    let texts = env.texts;
    let mut steps = Steps::default();
    let items = [
        Segment::new(None, "Solid"),
        Segment::new(None, "Linear"),
        Segment::new(None, "Radial"),
    ];
    let linear = |d: &TextClipData| match &d.style.fill {
        Fill::LinearGradient(g) => Some(g.clone()),
        _ => None,
    };
    let radial = |d: &TextClipData| match &d.style.fill {
        Fill::RadialGradient(g) => Some(g.clone()),
        _ => None,
    };
    rows::choice(
        ui,
        at.mixed(env.differ(|d| d.style.fill.kind())),
        &mut steps,
        "kind",
        "Mode",
        &items,
        &[
            FillKind::Solid,
            FillKind::LinearGradient,
            FillKind::RadialGradient,
        ],
        s.fill.kind(),
        FillKind::Solid,
        false,
        |k| style(TextStyleEdit::FillKind(k)),
    );
    match &s.fill {
        Fill::Solid(c) => rows::color(
            ui,
            at.mixed(env.differ(|d| match &d.style.fill {
                Fill::Solid(c) => Some(*c),
                _ => None,
            })),
            &mut steps,
            "color",
            "Color",
            *c,
            Rgba8::WHITE,
            |c| style(TextStyleEdit::Fill(Fill::Solid(c))),
        ),
        Fill::LinearGradient(g) => {
            stop_rows(ui, at, &mut steps, state, &g.stops, texts);
            rows::angle(
                ui,
                at.mixed(env.differ(|d| linear(d).map(|g| g.angle))),
                &mut steps,
                "angle",
                "Angle",
                g.angle,
                DeciDegrees::DOWN,
                |a| style(TextStyleEdit::FillAngle(a)),
            );
        }
        Fill::RadialGradient(g) => {
            stop_rows(ui, at, &mut steps, state, &g.stops, texts);
            let center = g.center;
            // One axis of each clip's own centre: the other axis stays.
            let centred = |f: &dyn Fn(FillCenter) -> FillCenter| {
                each_style(texts, |s| match &s.fill {
                    Fill::RadialGradient(g) => Some(TextStyleEdit::FillCenter(f(g.center))),
                    _ => None,
                })
                .unwrap_or_else(|| style(TextStyleEdit::FillCenter(f(center))))
            };
            rows::percent(
                ui,
                at.mixed(env.differ(|d| radial(d).map(|g| g.center.x))),
                &mut steps,
                "center_x",
                "Centre X",
                center.x,
                FillCenter::MIDDLE.x,
                (0, 1000),
                |x| centred(&|c| FillCenter { x, ..c }),
            );
            rows::percent(
                ui,
                at.mixed(env.differ(|d| radial(d).map(|g| g.center.y))),
                &mut steps,
                "center_y",
                "Centre Y",
                center.y,
                FillCenter::MIDDLE.y,
                (0, 1000),
                |y| centred(&|c| FillCenter { y, ..c }),
            );
            rows::percent(
                ui,
                at.mixed(env.differ(|d| radial(d).map(|g| g.spread))),
                &mut steps,
                "spread",
                "Spread",
                g.spread,
                PerMille::ONE,
                (100, 2000),
                |v| style(TextStyleEdit::FillSpread(v)),
            );
        }
    }
    let extent = match &s.fill {
        Fill::Solid(_) => None,
        Fill::LinearGradient(g) => Some(g.extent),
        Fill::RadialGradient(g) => Some(g.extent),
    };
    if let Some(extent) = extent {
        let extent_mixed = env.differ(|d| match &d.style.fill {
            Fill::Solid(_) => None,
            Fill::LinearGradient(g) => Some(g.extent),
            Fill::RadialGradient(g) => Some(g.extent),
        });
        card::more(ui, at.id("more"), memory, env.everything, 1, |ui| {
            let items = [
                Segment::new(None, "Whole text"),
                Segment::new(None, "Each line"),
            ];
            rows::choice(
                ui,
                at.mixed(extent_mixed),
                &mut steps,
                "extent",
                "Spans",
                &items,
                &[FillExtent::Block, FillExtent::Line],
                extent,
                FillExtent::Block,
                false,
                |e| style(TextStyleEdit::FillExtent(e)),
            );
        });
    }
    env.commit(ui, steps)
}

/// The gradient's stop bar and the selected stop's colour, with add,
/// remove and reverse. The bar shows the first clip's stops; each change
/// is made to every selected clip's own stops (moving stop 2 moves each
/// clip's stop 2).
fn stop_rows(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    state: &mut PropsState,
    stops: &GradientStops,
    texts: &Texts,
) {
    let per_clip = |f: &dyn Fn(&GradientStops) -> Option<GradientStops>| {
        each_style(texts, |s| {
            s.fill.stops().and_then(f).map(TextStyleEdit::FillStops)
        })
    };
    let list = stops.as_slice();
    let selected = state.fill_stop.min(list.len().saturating_sub(1));
    let bar_stops: Vec<BarStop> = list
        .iter()
        .map(|s| BarStop {
            at: s.at.as_f64(),
            color: [s.color.r, s.color.g, s.color.b, s.color.a],
        })
        .collect();
    let stopped = PropRow::new(at.id("stops"), "Gradient")
        .hint("Drag a marker to move its stop; click one to pick it")
        .stacked()
        .show(ui, |ui| {
            let room = ui.available_width() - 3.0 * 24.0 - 8.0;
            let bar = ui
                .allocate_ui_with_layout(
                    egui::vec2(room.max(60.0), ui.available_height()),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| gradient_bar(ui, at.id("stops"), &bar_stops, selected),
                )
                .inner;
            ui.add_space(8.0 - ui.spacing().item_spacing.x);
            ui.spacing_mut().item_spacing.x = 0.0;
            let add = ui
                .add_enabled_ui(list.len() < GradientStops::MAX, |ui| {
                    icon_button(ui, icons::PLUS, 18.0, "Add a stop")
                })
                .inner;
            let remove = ui
                .add_enabled_ui(list.len() > GradientStops::MIN, |ui| {
                    icon_button(ui, icons::MINUS, 18.0, "Remove the picked stop")
                })
                .inner;
            let reverse = icon_button(ui, icons::REVERSE, 18.0, "Reverse the gradient");
            (bar, add, remove, reverse)
        });
    let (bar, add, remove, reverse) = stopped.inner;
    if let Some(i) = bar.selected {
        state.fill_stop = i;
    }
    if bar.cancelled {
        steps.cancel();
    } else {
        let edit = bar.moved.and_then(|(i, f)| {
            let at = rows::per_mille_in(f * 1000.0, 0, 1000);
            per_clip(&|st| moved_stop(st, i, at))
        });
        steps.button(bar.response, edit);
    }

    let Some(stop) = list.get(selected) else {
        steps.finish(add);
        steps.finish(remove);
        steps.finish(reverse);
        return;
    };
    let label = format!("Stop {}", selected + 1);
    // Fills hold at most 8 stops: the index fits a u8.
    let index = u8::try_from(selected).unwrap_or(u8::MAX);
    let nav = at.nav("Stop colour", Some(PropPath::FillStopColor(index)));
    let mut stop_row = PropRow::new(at.id("stop"), &label).parameter(rows::COLOUR_W);
    if let Some((_, n)) = nav {
        stop_row = stop_row.keys(n);
    }
    let row = stop_row.show(ui, |ui| {
        rows::colour_field(ui, at.id("stop"), stop.color, false, &label)
            .filter(|c| *c != stop.color)
    });
    if let Some((prop, _)) = nav {
        at.keys_clicked(prop, row.keys, steps);
    }
    let color = row.inner;
    if let Some(color) = color {
        steps.edit(
            style(TextStyleEdit::FillStopColor { index, color }),
            Merge::WhileHeld,
        );
    }
    let added = add.clicked().then(|| added_stop(stops)).flatten();
    if let Some((_, i)) = &added {
        state.fill_stop = *i;
    }
    let edit = added.and_then(|_| per_clip(&|st| added_stop(st).map(|(st, _)| st)));
    steps.button(add, edit);
    let removed = remove
        .clicked()
        .then(|| removed_stop(stops, selected))
        .flatten();
    if removed.is_some() {
        state.fill_stop = selected.saturating_sub(1);
    }
    let edit = removed.and_then(|_| per_clip(&|st| removed_stop(st, selected)));
    steps.button(remove, edit);
    let reversed = reverse.clicked().then(|| {
        state.fill_stop = list.len().saturating_sub(1) - selected;
        per_clip(&|st| Some(st.reversed()))
    });
    steps.button(reverse, reversed.flatten());
}

/// `stops` with stop `i` at `at`, kept between its neighbours so the order
/// (and every index) stays; `None` when nothing changes.
pub(crate) fn moved_stop(stops: &GradientStops, i: usize, at: PerMille) -> Option<GradientStops> {
    let list = stops.as_slice();
    let lo = i
        .checked_sub(1)
        .and_then(|j| list.get(j))
        .map_or(PerMille::ZERO, |s| s.at);
    let hi = list.get(i + 1).map_or(PerMille::ONE, |s| s.at);
    let at = at.clamp(lo, hi);
    let mut out = list.to_vec();
    let stop = out.get_mut(i)?;
    if stop.at == at {
        return None;
    }
    stop.at = at;
    GradientStops::new(out).ok()
}

/// `stops` with a new stop in the middle of the widest gap, coloured
/// halfway between its neighbours; with its index. `None` at 8 stops.
pub(crate) fn added_stop(stops: &GradientStops) -> Option<(GradientStops, usize)> {
    let list = stops.as_slice();
    if list.len() >= GradientStops::MAX {
        return None;
    }
    let (i, pair) = list
        .windows(2)
        .enumerate()
        .max_by_key(|(_, w)| w[1].at.get() - w[0].at.get())?;
    let (a, b) = (pair[0], pair[1]);
    let mid = |x: u8, y: u8| {
        // The mean of two u8 fits a u8.
        ((u16::from(x) + u16::from(y)) / 2) as u8
    };
    let stop = GradientStop {
        at: PerMille::new((a.at.get() + b.at.get()) / 2),
        color: Rgba8::new(
            mid(a.color.r, b.color.r),
            mid(a.color.g, b.color.g),
            mid(a.color.b, b.color.b),
            mid(a.color.a, b.color.a),
        ),
    };
    let mut out = list.to_vec();
    out.insert(i + 1, stop);
    GradientStops::new(out).ok().map(|s| (s, i + 1))
}

/// `stops` without stop `i`; `None` at 2 stops (TX-014 blocks deleting
/// down to one).
pub(crate) fn removed_stop(stops: &GradientStops, i: usize) -> Option<GradientStops> {
    let list = stops.as_slice();
    if list.len() <= GradientStops::MIN || i >= list.len() {
        return None;
    }
    let mut out = list.to_vec();
    out.remove(i);
    GradientStops::new(out).ok()
}

/// The Spacing card (TX-009, TX-054): letters and lines, each with −/+
/// steppers (±1 %, ±10 % with Shift).
pub fn spacing_card(
    ui: &mut egui::Ui,
    env: &mut CardEnv<'_>,
    s: &TextStyle,
    layout: &TextLayout,
) -> Option<String> {
    let at = env.at(CardId::Spacing);
    let mut steps = Steps::default();
    let n = Numeric::new(
        "letters",
        "Letters",
        s.letter_spacing.as_f64() * 100.0,
        0.0,
        f64::from(TextStyle::MIN_LETTER_SPACING.get()) / 10.0
            ..=f64::from(TextStyle::MAX_LETTER_SPACING.get()) / 10.0,
    )
    .soft(-20.0..=50.0)
    .suffix(" %")
    .stepper(1.0);
    let stored = s.letter_spacing;
    rows::signed(
        ui,
        at.mixed(env.differ(|d| d.style.letter_spacing)),
        &mut steps,
        n,
        stored,
        (
            TextStyle::MIN_LETTER_SPACING.get(),
            TextStyle::MAX_LETTER_SPACING.get(),
        ),
        |v| style(TextStyleEdit::LetterSpacing(v)),
    );
    let stored = layout.line_spacing;
    let n = Numeric::new(
        "lines",
        "Lines",
        stored.as_f64() * 100.0,
        100.0,
        TextLayout::MIN_LINE_SPACING.as_f64() * 100.0
            ..=TextLayout::MAX_LINE_SPACING.as_f64() * 100.0,
    )
    .suffix(" %")
    .stepper(1.0);
    let lines_at = at.mixed(env.differ(|d| d.layout.line_spacing));
    rows::numeric(ui, lines_at, n, &mut steps, |v| {
        let wanted = TextLayout::clamp_line_spacing(rows::per_mille_in(v * 10.0, 0, 3000));
        (lines_at.mixed || wanted != stored)
            .then_some(Edit::Layout(TextLayoutEdit::LineSpacing(wanted)))
    });
    env.commit(ui, steps)
}

/// The Outline card (TX-010): colour and width; behind More, opacity and
/// position.
pub fn outline_card(
    ui: &mut egui::Ui,
    env: &mut CardEnv<'_>,
    o: &Stroke,
    memory: &mut CardMemory,
) -> Option<String> {
    let d = Stroke::default();
    let at = env.at(CardId::Outline);
    let texts = env.texts;
    let mut steps = Steps::default();
    rows::color(
        ui,
        at.mixed(differ(texts, |d| d.style.stroke.value.color)),
        &mut steps,
        "color",
        "Color",
        o.color,
        d.color,
        |c| style(TextStyleEdit::OutlineColor(c)),
    );
    rows::percent(
        ui,
        at.mixed(differ(texts, |d| d.style.stroke.value.width)),
        &mut steps,
        "width",
        "Width",
        o.width,
        d.width,
        (0, Stroke::MAX_WIDTH.get()),
        |v| style(TextStyleEdit::OutlineWidth(v)),
    );
    card::more(ui, at.id("more"), memory, env.everything, 2, |ui| {
        rows::percent(
            ui,
            at.mixed(differ(texts, |d| d.style.stroke.value.opacity)),
            &mut steps,
            "opacity",
            "Opacity",
            o.opacity,
            d.opacity,
            (0, 1000),
            |v| style(TextStyleEdit::OutlineOpacity(v)),
        );
        let items = [
            Segment::new(None, "Outside"),
            Segment::new(None, "Center"),
            Segment::new(None, "Inside"),
        ];
        rows::choice(
            ui,
            at.mixed(differ(texts, |d| d.style.stroke.value.position)),
            &mut steps,
            "position",
            "Position",
            &items,
            &[
                StrokePosition::Outside,
                StrokePosition::Center,
                StrokePosition::Inside,
            ],
            o.position,
            d.position,
            false,
            |p| style(TextStyleEdit::OutlinePosition(p)),
        );
    });
    env.commit(ui, steps)
}

/// The Shadow card (TX-011): colour and blur; behind More, opacity,
/// distance, angle and whether the box casts it too.
pub fn shadow_card(
    ui: &mut egui::Ui,
    env: &mut CardEnv<'_>,
    sh: &Shadow,
    memory: &mut CardMemory,
) -> Option<String> {
    let d = Shadow::default();
    let at = env.at(CardId::Shadow);
    let texts = env.texts;
    let mut steps = Steps::default();
    rows::color(
        ui,
        at.mixed(differ(texts, |d| d.style.shadow.value.color)),
        &mut steps,
        "color",
        "Color",
        sh.color,
        d.color,
        |c| style(TextStyleEdit::ShadowColor(c)),
    );
    rows::percent(
        ui,
        at.mixed(differ(texts, |d| d.style.shadow.value.blur)),
        &mut steps,
        "blur",
        "Blur",
        sh.blur,
        d.blur,
        (0, Shadow::MAX_BLUR.get()),
        |v| style(TextStyleEdit::ShadowBlur(v)),
    );
    card::more(ui, at.id("more"), memory, env.everything, 4, |ui| {
        rows::percent(
            ui,
            at.mixed(differ(texts, |d| d.style.shadow.value.opacity)),
            &mut steps,
            "opacity",
            "Opacity",
            sh.opacity,
            d.opacity,
            (0, 1000),
            |v| style(TextStyleEdit::ShadowOpacity(v)),
        );
        rows::percent(
            ui,
            at.mixed(differ(texts, |d| d.style.shadow.value.distance)),
            &mut steps,
            "distance",
            "Distance",
            sh.distance,
            d.distance,
            (0, Shadow::MAX_DISTANCE.get()),
            |v| style(TextStyleEdit::ShadowDistance(v)),
        );
        rows::angle(
            ui,
            at.mixed(differ(texts, |d| d.style.shadow.value.angle)),
            &mut steps,
            "angle",
            "Angle",
            sh.angle,
            d.angle,
            |a| style(TextStyleEdit::ShadowAngle(a)),
        );
        rows::switch(
            ui,
            at.mixed(differ(texts, |d| d.style.shadow.value.include_box)),
            &mut steps,
            "include_box",
            "Box casts",
            sh.include_box,
            d.include_box,
            |on| style(TextStyleEdit::ShadowIncludeBox(on)),
        );
    });
    env.commit(ui, steps)
}

/// The Glow card (TX-012): colour and radius; behind More, intensity,
/// direction, offset and its angle.
pub fn glow_card(
    ui: &mut egui::Ui,
    env: &mut CardEnv<'_>,
    g: &Glow,
    memory: &mut CardMemory,
) -> Option<String> {
    let d = Glow::default();
    let at = env.at(CardId::Glow);
    let texts = env.texts;
    let mut steps = Steps::default();
    rows::color(
        ui,
        at.mixed(differ(texts, |d| d.style.glow.value.color)),
        &mut steps,
        "color",
        "Color",
        g.color,
        d.color,
        |c| style(TextStyleEdit::GlowColor(c)),
    );
    rows::percent(
        ui,
        at.mixed(differ(texts, |d| d.style.glow.value.radius)),
        &mut steps,
        "radius",
        "Radius",
        g.radius,
        d.radius,
        (0, Glow::MAX_RADIUS.get()),
        |v| style(TextStyleEdit::GlowRadius(v)),
    );
    card::more(ui, at.id("more"), memory, env.everything, 4, |ui| {
        rows::percent(
            ui,
            at.mixed(differ(texts, |d| d.style.glow.value.intensity)),
            &mut steps,
            "intensity",
            "Intensity",
            g.intensity,
            d.intensity,
            (0, 1000),
            |v| style(TextStyleEdit::GlowIntensity(v)),
        );
        let items = [
            Segment::new(None, "Outward"),
            Segment::new(None, "Inward"),
            Segment::new(None, "Both"),
        ];
        rows::choice(
            ui,
            at.mixed(differ(texts, |d| d.style.glow.value.direction)),
            &mut steps,
            "direction",
            "Direction",
            &items,
            &[
                GlowDirection::Outward,
                GlowDirection::Inward,
                GlowDirection::Both,
            ],
            g.direction,
            d.direction,
            false,
            |v| style(TextStyleEdit::GlowDirection(v)),
        );
        rows::percent(
            ui,
            at.mixed(differ(texts, |d| d.style.glow.value.offset)),
            &mut steps,
            "offset",
            "Offset",
            g.offset,
            d.offset,
            (0, Glow::MAX_OFFSET.get()),
            |v| style(TextStyleEdit::GlowOffset(v)),
        );
        rows::angle(
            ui,
            at.mixed(differ(texts, |d| d.style.glow.value.angle)),
            &mut steps,
            "angle",
            "Angle",
            g.angle,
            d.angle,
            |a| style(TextStyleEdit::GlowAngle(a)),
        );
    });
    env.commit(ui, steps)
}

/// "Noto Sans · 64 px · Bold · UPPER".
pub fn type_summary(s: &TextStyle) -> String {
    let mut out = format!("{} · {:.0} px", s.font.family, s.size.as_f64());
    match (s.bold, s.italic) {
        (true, true) => out.push_str(" · Bold italic"),
        (true, false) => out.push_str(" · Bold"),
        (false, true) => out.push_str(" · Italic"),
        (false, false) => {}
    }
    match s.case {
        CaseTransform::AsTyped => {}
        CaseTransform::Upper => out.push_str(" · UPPER"),
        CaseTransform::Lower => out.push_str(" · lower"),
        CaseTransform::Title => out.push_str(" · Title"),
        CaseTransform::Sentence => out.push_str(" · Sentence"),
    }
    out
}

/// "#FFFFFF", "Linear · 3 stops · 90°", "Radial · 2 stops".
pub fn fill_summary(fill: &Fill) -> String {
    match fill {
        Fill::Solid(c) => hex(*c),
        Fill::LinearGradient(g) => format!(
            "Linear · {} stops · {:.0}°",
            g.stops.as_slice().len(),
            g.angle.degrees()
        ),
        Fill::RadialGradient(g) => format!("Radial · {} stops", g.stops.as_slice().len()),
    }
}

/// "Letters 0 % · Lines 100 %".
pub fn spacing_summary(s: &TextStyle, layout: &TextLayout) -> String {
    format!(
        "Letters {:.0} % · Lines {:.0} %",
        s.letter_spacing.as_f64() * 100.0,
        layout.line_spacing.as_f64() * 100.0
    )
}

/// "6 % · #000000".
pub fn outline_summary(o: &Stroke) -> String {
    format!("{:.0} % · {}", o.width.as_f64() * 100.0, hex(o.color))
}

/// "#000000 · blur 10 % · 45°".
pub fn shadow_summary(s: &Shadow) -> String {
    format!(
        "{} · blur {:.0} % · {:.0}°",
        hex(s.color),
        s.blur.as_f64() * 100.0,
        s.angle.degrees()
    )
}

/// "#FFFFFF · 25 % · outward".
pub fn glow_summary(g: &Glow) -> String {
    let dir = match g.direction {
        GlowDirection::Outward => "outward",
        GlowDirection::Inward => "inward",
        GlowDirection::Both => "both ways",
    };
    format!(
        "{} · {:.0} % · {dir}",
        hex(g.color),
        g.radius.as_f64() * 100.0
    )
}

/// A signed per-mille shown as −100…+100 (curve, bend).
pub(crate) fn tenths(v: SignedPerMille) -> f64 {
    f64::from(v.get()) / 10.0
}

/// The −100…+100 value `v` as stored.
pub(crate) fn from_tenths(v: f64) -> SignedPerMille {
    signed_per_mille_in(v * 10.0, -1000, 1000)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use eframe::egui::{Event, Key, Modifiers, Pos2};
    use freecut_timeline::edit::EditRequest;
    use freecut_timeline::history::Coalesce;
    use freecut_timeline::ids::ClipId;
    use freecut_timeline::model::{ProjectSettings, RefPx};
    use freecut_timeline::time::Tick;

    use super::*;
    use crate::session::Project;
    use crate::ui::drag::DragGesture;
    use crate::ui::drag::tests::{Frames, button, key};
    use crate::ui::inspector::lens::SelectionKind;
    use crate::widgets::field::NumberField;

    /// Two text clips, both selected, the Style cards drawn for them.
    struct Rig {
        frames: Frames,
        project: Project,
        drag: DragGesture,
        state: PropsState,
        memory: CardMemory,
        a: ClipId,
        b: ClipId,
    }

    type Card = fn(&mut egui::Ui, &mut CardEnv<'_>, &mut PropsState, &TextStyle, &mut CardMemory);

    impl Rig {
        fn new() -> Rig {
            let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
            let a = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
            project.playhead = Tick::from_secs(10).unwrap();
            let b = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
            project.selection = vec![a, b];
            Rig {
                frames: Frames::new(),
                project,
                drag: DragGesture::default(),
                state: PropsState::default(),
                memory: CardMemory::new(true),
                a,
                b,
            }
        }

        fn styled(&mut self, clip: ClipId, edit: TextStyleEdit) {
            let req = EditRequest::SetTextStyle {
                seq: self.project.doc().root(),
                clips: vec![clip],
                edit,
            };
            self.project.apply(&req, Coalesce::Never).unwrap();
        }

        fn style(&self, clip: ClipId) -> TextStyle {
            let doc = self.project.doc();
            doc.clip(clip)
                .unwrap()
                .content
                .text()
                .unwrap()
                .style
                .clone()
        }

        fn frame(&mut self, card: Card, events: Vec<Event>) {
            let doc = self.project.doc().clone();
            let texts: Vec<(ClipId, std::sync::Arc<TextClipData>)> = [self.a, self.b]
                .iter()
                .map(|c| (*c, doc.clip(*c).unwrap().content.text().unwrap().clone()))
                .collect();
            let first = texts[0].1.style.clone();
            let clips = [self.a, self.b];
            let (project, drag, state, memory) = (
                &mut self.project,
                &mut self.drag,
                &mut self.state,
                &mut self.memory,
            );
            self.frames.run(events, |ui| {
                let mut env = CardEnv {
                    project: &mut *project,
                    drag: &mut *drag,
                    clips: &clips,
                    texts: &texts,
                    kind: SelectionKind::Several,
                    everything: true,
                    keys: None,
                };
                card(ui, &mut env, state, &first, memory);
            });
            let down = self.frames.ctx.input(|i| i.pointer.any_down());
            self.drag.settle(Some(&mut self.project), down);
        }

        fn click(&mut self, card: Card, id: egui::Id) {
            let at: Pos2 = self.frames.ctx.read_response(id).unwrap().rect.center();
            self.frame(card, vec![Event::PointerMoved(at), button(at, true)]);
            self.frame(card, vec![button(at, false)]);
            self.frame(card, Vec::new());
        }

        /// Clicks the field of row `row` and types `text`, then Enter.
        fn type_into(&mut self, card: Card, row: egui::Id, text: &str) {
            let key = row.with("field");
            let field = NumberField::field_id(&self.frames.ctx, key).unwrap();
            self.click(card, field);
            self.frame(card, vec![Event::Text(text.to_owned())]);
            self.frame(card, vec![key_event(Key::Enter)]);
            self.frame(card, Vec::new());
        }
    }

    fn key_event(k: Key) -> Event {
        key(k, Modifiers::NONE)
    }

    fn type_card_of(
        ui: &mut egui::Ui,
        env: &mut CardEnv<'_>,
        state: &mut PropsState,
        s: &TextStyle,
        memory: &mut CardMemory,
    ) {
        let fonts = FontChoices {
            families: vec!["Noto Sans".to_owned()].into(),
            scanning: false,
        };
        assert_eq!(type_card(ui, env, state, s, &fonts, memory), None);
    }

    fn fill_card_of(
        ui: &mut egui::Ui,
        env: &mut CardEnv<'_>,
        state: &mut PropsState,
        s: &TextStyle,
        memory: &mut CardMemory,
    ) {
        assert_eq!(fill_card(ui, env, state, s, memory), None);
    }

    /// TX-008 with several clips: different cases light no segment, and a
    /// click on the first clip's case sets it on all of them; the case row
    /// shows its changed dot.
    #[test]
    fn a_mixed_case_lights_nothing_and_a_click_sets_it_on_all() {
        let mut rig = Rig::new();
        rig.styled(rig.a, TextStyleEdit::Case(CaseTransform::Upper));
        let at = At::new(SelectionKind::Several, CardId::Type);
        rig.frame(type_card_of, Vec::new());
        assert!(crate::widgets::row::shows_changed(
            &rig.frames.ctx,
            at.id("case")
        ));
        let entries = rig.project.editor.history().len();
        // "AA", the first clip's own case.
        rig.click(type_card_of, at.id("case").with(1));
        assert_eq!(rig.style(rig.a).case, CaseTransform::Upper);
        assert_eq!(rig.style(rig.b).case, CaseTransform::Upper);
        assert_eq!(rig.project.editor.history().len(), entries + 1);
    }

    /// TX-054 with several clips: typing the first clip's own size into a
    /// mixed Size field gives every clip that size; the changed dot shows
    /// though the first clip has the default.
    #[test]
    fn typing_the_first_clips_size_sets_it_on_all() {
        let mut rig = Rig::new();
        rig.styled(rig.b, TextStyleEdit::Size(RefPx::from_px(80)));
        let at = At::new(SelectionKind::Several, CardId::Type);
        rig.frame(type_card_of, Vec::new());
        assert!(crate::widgets::row::shows_changed(
            &rig.frames.ctx,
            at.id("size")
        ));
        let first = rig.style(rig.a).size;
        rig.type_into(type_card_of, at.id("size"), &format!("{}", first.as_f64()));
        assert_eq!(rig.style(rig.b).size, first);
    }

    /// ui-concept §6.2 / UI-009: a double-click on a changed row's label
    /// puts the default back on every selected clip in one undo entry; a
    /// row at its default doesn't react.
    #[test]
    fn a_double_click_on_the_label_resets_in_one_entry() {
        let mut rig = Rig::new();
        rig.styled(rig.a, TextStyleEdit::Size(RefPx::from_px(80)));
        rig.styled(rig.b, TextStyleEdit::Size(RefPx::from_px(90)));
        let at = At::new(SelectionKind::Several, CardId::Type);
        rig.frame(type_card_of, Vec::new());
        let entries = rig.project.editor.history().len();
        let label = at.id("size").with("label");
        let double = |rig: &mut Rig| {
            let at: Pos2 = rig.frames.ctx.read_response(label).unwrap().rect.center();
            rig.frame(
                type_card_of,
                vec![Event::PointerMoved(at), button(at, true)],
            );
            rig.frame(type_card_of, vec![button(at, false)]);
            rig.frame(type_card_of, vec![button(at, true)]);
            rig.frame(type_card_of, vec![button(at, false)]);
            rig.frame(type_card_of, Vec::new());
        };
        double(&mut rig);
        assert_eq!(rig.style(rig.a).size, TextStyle::DEFAULT_SIZE);
        assert_eq!(rig.style(rig.b).size, TextStyle::DEFAULT_SIZE);
        assert_eq!(rig.project.editor.history().len(), entries + 1, "one entry");
        rig.project.undo().unwrap();
        assert_eq!(rig.style(rig.a).size, RefPx::from_px(80), "one undo");
        assert_eq!(rig.style(rig.b).size, RefPx::from_px(90));
        rig.project.redo().unwrap();
        // At the default: nothing to reset, no entry.
        let entries = rig.project.editor.history().len();
        double(&mut rig);
        assert_eq!(rig.project.editor.history().len(), entries);
    }

    /// ui-concept §13.5: a hex typed into the Fill colour's field and
    /// committed with Enter is one undo entry on every selected clip.
    #[test]
    fn a_typed_hex_colour_is_one_entry() {
        let mut rig = Rig::new();
        let at = At::new(SelectionKind::Several, CardId::Fill);
        rig.frame(fill_card_of, Vec::new());
        let entries = rig.project.editor.history().len();
        let hex = at.id("color").with("colour").with("hex");
        rig.click(fill_card_of, hex);
        rig.frame(fill_card_of, vec![Event::Text("f80".to_owned())]);
        rig.frame(fill_card_of, vec![key_event(Key::Enter)]);
        rig.frame(fill_card_of, Vec::new());
        let orange = Rgba8::new(0xFF, 0x88, 0x00, 255);
        assert_eq!(rig.style(rig.a).fill, Fill::Solid(orange));
        assert_eq!(rig.style(rig.b).fill, Fill::Solid(orange));
        assert_eq!(rig.project.editor.history().len(), entries + 1, "one entry");
        rig.project.undo().unwrap();
        assert_eq!(rig.style(rig.a).fill, Fill::Solid(Rgba8::WHITE), "one undo");
    }

    /// ui-concept §13.20: picking a face in its select is one entry.
    #[test]
    fn a_select_pick_is_one_entry() {
        let mut rig = Rig::new();
        let at = At::new(SelectionKind::Several, CardId::Type);
        rig.frame(type_card_of, Vec::new());
        let entries = rig.project.editor.history().len();
        let select = at.id("face").with("select");
        assert!(rig.style(rig.a).bold, "the default text is bold");
        rig.click(type_card_of, select);
        // "Bold italic", the fourth face: italic added to the bold text.
        rig.click(type_card_of, select.with("list").with(3usize));
        for clip in [rig.a, rig.b] {
            assert!(rig.style(clip).bold && rig.style(clip).italic);
        }
        assert_eq!(rig.project.editor.history().len(), entries + 1, "one entry");
        rig.project.undo().unwrap();
        assert!(!rig.style(rig.a).italic, "one undo");
        assert!(!rig.style(rig.b).italic);
    }

    /// A radial centre's X goes to every clip, and each keeps its own Y,
    /// in one entry.
    #[test]
    fn a_centre_axis_keeps_each_clips_other_axis() {
        let mut rig = Rig::new();
        for (clip, x, y) in [(rig.a, 200, 300), (rig.b, 600, 700)] {
            rig.styled(clip, TextStyleEdit::FillKind(FillKind::RadialGradient));
            rig.styled(
                clip,
                TextStyleEdit::FillCenter(FillCenter {
                    x: PerMille::new(x),
                    y: PerMille::new(y),
                }),
            );
        }
        let at = At::new(SelectionKind::Several, CardId::Fill);
        rig.frame(fill_card_of, Vec::new());
        let entries = rig.project.editor.history().len();
        rig.type_into(fill_card_of, at.id("center_x"), "50");
        let center = |rig: &Rig, clip| match rig.style(clip).fill {
            Fill::RadialGradient(g) => (g.center.x.get(), g.center.y.get()),
            _ => panic!("radial"),
        };
        assert_eq!(center(&rig, rig.a), (500, 300));
        assert_eq!(center(&rig, rig.b), (500, 700));
        assert_eq!(rig.project.editor.history().len(), entries + 1);
        rig.project.undo().unwrap();
        assert_eq!(center(&rig, rig.b), (600, 700), "one undo");
    }

    /// Gradient stop edits are made on each clip's own stops.
    #[test]
    fn stop_edits_keep_each_clips_own_stops() {
        let a: Vec<(ClipId, std::sync::Arc<TextClipData>)> = [
            [Rgba8::BLACK, Rgba8::WHITE],
            [Rgba8::new(255, 0, 0, 255), Rgba8::new(0, 0, 255, 255)],
        ]
        .iter()
        .enumerate()
        .map(|(i, [from, to])| {
            let mut d = TextClipData::default();
            d.style.fill = Fill::Solid(Rgba8::WHITE).with_kind(FillKind::LinearGradient);
            d.style.fill = Fill::LinearGradient(match d.style.fill {
                Fill::LinearGradient(mut g) => {
                    g.stops = GradientStops::two(*from, *to);
                    g
                }
                _ => unreachable!(),
            });
            let id = ClipId::from_raw(std::num::NonZeroU64::new(i as u64 + 1).unwrap());
            (id, std::sync::Arc::new(d))
        })
        .collect();
        let Some(Edit::Each(edits)) = each_style(&a, |s| {
            s.fill
                .stops()
                .map(|st| TextStyleEdit::FillStops(st.reversed()))
        }) else {
            panic!("one edit per clip");
        };
        let firsts: Vec<Rgba8> = edits
            .iter()
            .map(|(_, e)| match e {
                Edit::Style(TextStyleEdit::FillStops(st)) => st.as_slice()[0].color,
                _ => panic!("{e:?}"),
            })
            .collect();
        assert_eq!(firsts, [Rgba8::WHITE, Rgba8::new(0, 0, 255, 255)]);
    }

    fn stop(at: u16, c: u8) -> GradientStop {
        GradientStop {
            at: PerMille::new(at),
            color: Rgba8::new(c, c, c, 255),
        }
    }

    #[test]
    fn stops_move_between_neighbours_and_add_and_remove_within_limits() {
        let stops = GradientStops::new(vec![stop(0, 0), stop(400, 100), stop(1000, 200)]).unwrap();
        // A stop never passes a neighbour, so indices stay.
        let moved = moved_stop(&stops, 1, PerMille::new(900)).unwrap();
        assert_eq!(moved.as_slice()[1].at.get(), 900);
        let moved = moved_stop(&stops, 1, PerMille::ONE).unwrap();
        assert_eq!(moved.as_slice()[1], stop(1000, 100), "meets its neighbour");
        assert_eq!(
            moved_stop(&stops, 0, PerMille::new(500))
                .unwrap()
                .as_slice()[0]
                .at
                .get(),
            400
        );
        assert_eq!(moved_stop(&stops, 1, PerMille::new(400)), None);
        assert_eq!(moved_stop(&stops, 9, PerMille::ZERO), None);

        // Added in the widest gap, between 400 and 1000, coloured halfway.
        let (added, i) = added_stop(&stops).unwrap();
        assert_eq!(i, 2);
        assert_eq!(added.as_slice()[2], stop(700, 150));
        let mut full = stops.clone();
        for _ in 0..5 {
            full = added_stop(&full).unwrap().0;
        }
        assert_eq!(full.as_slice().len(), GradientStops::MAX);
        assert_eq!(added_stop(&full), None);

        let removed = removed_stop(&stops, 1).unwrap();
        assert_eq!(removed.as_slice(), &[stop(0, 0), stop(1000, 200)]);
        assert_eq!(removed_stop(&removed, 0), None, "two stops stay");
    }

    #[test]
    fn summaries_say_the_value_in_a_few_words() {
        let mut s = TextStyle::default();
        assert_eq!(type_summary(&s), "Noto Sans · 64 px · Bold");
        s.case = CaseTransform::Upper;
        assert_eq!(type_summary(&s), "Noto Sans · 64 px · Bold · UPPER");
        assert_eq!(fill_summary(&s.fill), "#FFFFFF");
        let linear = s.fill.with_kind(FillKind::LinearGradient);
        assert_eq!(fill_summary(&linear), "Linear · 2 stops · 90°");
        assert_eq!(outline_summary(&Stroke::default()), "6 % · #000000");
        assert_eq!(glow_summary(&Glow::default()), "#FFFFFF · 25 % · outward");
        assert_eq!(
            spacing_summary(&s, &TextLayout::default()),
            "Letters 0 % · Lines 100 %"
        );
        assert_eq!(from_tenths(tenths(SignedPerMille::new(-350))).get(), -350);
    }
}
