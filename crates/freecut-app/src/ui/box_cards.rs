//! The Box & bubble lens's text layers (TX-054, ui-concept §7):
//! **Background** (TX-013), **Bubble** (TX-055) and **Curve** (TX-016).
//! Background and Bubble are never on together; the inspector's card
//! switches send `SetLayer`, which switches the other off in the same
//! entry, and the summaries say why a layer is off or paused.

use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use freecut_timeline::edit::TextStyleEdit;
use freecut_timeline::model::{
    BoxHeight, BoxShape, BubbleKind, BubbleSpec, Layer, PerMille, Side, SignedPerMille, Tail,
    TextBox, TextBubble, TextStyle,
};

use super::inspector::lens::CardId;
use super::props::{CardEnv, Edit, Steps};
use super::rows::{self, At, Numeric, hex};
use super::style_cards::{from_tenths, tenths};
use crate::theme::icons;
use crate::widgets::card::{self, CardMemory};
use crate::widgets::icon_button;
use crate::widgets::row::PropRow;
use crate::widgets::segmented::Segment;
use crate::widgets::select::{Select, SelectOption};

fn style(edit: TextStyleEdit) -> Edit {
    Edit::Style(edit)
}

/// The Background card (TX-013): colour, opacity, shape, padding and
/// roundness; behind More, height and the border.
pub fn background_card(
    ui: &mut egui::Ui,
    env: &mut CardEnv<'_>,
    s: &TextStyle,
    memory: &mut CardMemory,
) -> Option<String> {
    let b = &s.background.value;
    let d = TextBox::default();
    let at = env.at(CardId::Background);
    let mut steps = Steps::default();
    paused_note(ui, s);
    rows::color(
        ui,
        at,
        &mut steps,
        "color",
        "Color",
        b.color,
        d.color,
        |c| style(TextStyleEdit::BackgroundColor(c)),
    );
    rows::percent(
        ui,
        at,
        &mut steps,
        "opacity",
        "Opacity",
        b.opacity,
        d.opacity,
        (0, 1000),
        |v| style(TextStyleEdit::BackgroundOpacity(v)),
    );
    let items = [
        Segment::new(None, "Block"),
        Segment::new(None, "Lines"),
        Segment::new(None, "Letters"),
    ];
    rows::choice(
        ui,
        at,
        &mut steps,
        "shape",
        "Shape",
        &items,
        &[BoxShape::Block, BoxShape::PerLine, BoxShape::PerChar],
        b.shape,
        d.shape,
        false,
        |v| style(TextStyleEdit::BackgroundShape(v)),
    );
    padding_rows(
        ui,
        at,
        &mut steps,
        (b.pad_x, b.pad_y),
        (d.pad_x, d.pad_y),
        |x| style(TextStyleEdit::BackgroundPadX(x)),
        |y| style(TextStyleEdit::BackgroundPadY(y)),
    );
    rows::percent(
        ui,
        at,
        &mut steps,
        "roundness",
        "Roundness",
        b.roundness,
        d.roundness,
        (0, 1000),
        |v| style(TextStyleEdit::BackgroundRoundness(v)),
    );
    card::more(ui, at.id("more"), memory, env.everything, 4, |ui| {
        let items = [Segment::new(None, "Full"), Segment::new(None, "Lower half")];
        rows::choice(
            ui,
            at,
            &mut steps,
            "height",
            "Height",
            &items,
            &[BoxHeight::Full, BoxHeight::LowerHalf],
            b.height,
            d.height,
            false,
            |v| style(TextStyleEdit::BackgroundHeight(v)),
        );
        border_rows(
            ui,
            at,
            &mut steps,
            b.border,
            d.border,
            |on| style(TextStyleEdit::BackgroundBorder(on)),
            |c| style(TextStyleEdit::BackgroundBorderColor(c)),
            |w| style(TextStyleEdit::BackgroundBorderWidth(w)),
        );
    });
    env.commit(ui, steps)
}

/// The kinds in the Kind dropdown, with their names (TX-055).
const KINDS: [(BubbleKind, &str); 8] = [
    (BubbleKind::Rounded, "Rounded"),
    (BubbleKind::Oval, "Oval"),
    (BubbleKind::Thought, "Thought"),
    (BubbleKind::Shout, "Shout"),
    (BubbleKind::Square, "Square"),
    (BubbleKind::Tag, "Tag"),
    (BubbleKind::ArrowTag { point: Side::Right }, "Arrow tag"),
    (BubbleKind::Ticket, "Ticket"),
];

/// The name of `kind` (an arrow tag of either side is "Arrow tag").
pub fn kind_name(kind: BubbleKind) -> &'static str {
    KINDS
        .iter()
        .find(|(k, _)| std::mem::discriminant(k) == std::mem::discriminant(&kind))
        .map_or("Rounded", |(_, name)| name)
}

/// Whether `kind` draws a tail (or Thought's trail) when its tail is on:
/// the kinds with a tail handle on the Stage.
pub fn kind_has_tail(kind: BubbleKind) -> bool {
    matches!(
        kind,
        BubbleKind::Rounded
            | BubbleKind::Oval
            | BubbleKind::Square
            | BubbleKind::Shout
            | BubbleKind::Thought
    )
}

/// The Bubble card (TX-055): kind, tail and colour, the kind's own extras
/// (Arrow tag's point, Shout's spikes with a dice for a new seed, Thought's
/// lobes and trail); behind More, opacity, tail width and bend, padding,
/// roundness and the border.
pub fn bubble_card(
    ui: &mut egui::Ui,
    env: &mut CardEnv<'_>,
    s: &TextStyle,
    memory: &mut CardMemory,
) -> Option<String> {
    let b = &s.bubble.value;
    let spec = &b.spec;
    let d = TextBubble::default();
    let ds = BubbleSpec::default();
    let at = env.at(CardId::Bubble);
    let mut steps = Steps::default();
    paused_note(ui, s);
    let options = KINDS.map(|(_, name)| SelectOption::new(name));
    let current = KINDS
        .iter()
        .position(|(_, name)| kind_name(spec.kind) == *name);
    let kind_row = PropRow::new(at.id("kind"), "Kind")
        .changed(spec.kind != ds.kind)
        .show(ui, |ui| {
            Select::new(at.id("kind").with("select"), &options, current)
                .fill_width()
                .show(ui)
                .picked
                .filter(|i| Some(*i) != current)
                .and_then(|i| KINDS.get(i))
                .map(|(kind, _)| *kind)
        });
    if kind_row.reset {
        steps.edit(
            style(TextStyleEdit::BubbleKind(ds.kind)),
            super::props::Merge::Own,
        );
    }
    if let Some(kind) = kind_row.inner {
        steps.edit(
            style(TextStyleEdit::BubbleKind(kind)),
            super::props::Merge::Own,
        );
    }
    if let BubbleKind::ArrowTag { point } = spec.kind {
        let items = [Segment::new(None, "Left"), Segment::new(None, "Right")];
        rows::choice(
            ui,
            at,
            &mut steps,
            "point",
            "Points",
            &items,
            &[Side::Left, Side::Right],
            point,
            Side::Right,
            false,
            |p| style(TextStyleEdit::BubbleKind(BubbleKind::ArrowTag { point: p })),
        );
    }
    if kind_has_tail(spec.kind) {
        let label = if spec.kind == BubbleKind::Thought {
            "Trail"
        } else {
            "Tail"
        };
        rows::switch(
            ui,
            at,
            &mut steps,
            "tail",
            label,
            spec.tail.on,
            spec.kind.tail_by_default(),
            |on| style(TextStyleEdit::BubbleTail(on)),
        );
    }
    rows::color(ui, at, &mut steps, "color", "Color", b.fill, d.fill, |c| {
        style(TextStyleEdit::BubbleColor(c))
    });
    match spec.kind {
        BubbleKind::Shout => shout_rows(ui, at, &mut steps, spec, &ds),
        BubbleKind::Thought => {
            rows::percent(
                ui,
                at,
                &mut steps,
                "lobe",
                "Lobes",
                spec.lobe,
                ds.lobe,
                (100, 400),
                |v| style(TextStyleEdit::BubbleLobe(v)),
            );
            rows::count(
                ui,
                at,
                &mut steps,
                "trail_circles",
                "Circles",
                spec.trail,
                ds.trail,
                (BubbleSpec::MIN_TRAIL, BubbleSpec::MAX_TRAIL),
                |n| style(TextStyleEdit::BubbleTrail(n)),
            );
        }
        _ => {}
    }
    let tailed = kind_has_tail(spec.kind) && spec.kind != BubbleKind::Thought;
    let more = if tailed { 8 } else { 6 };
    card::more(ui, at.id("more"), memory, env.everything, more, |ui| {
        rows::percent(
            ui,
            at,
            &mut steps,
            "opacity",
            "Opacity",
            b.opacity,
            d.opacity,
            (0, 1000),
            |v| style(TextStyleEdit::BubbleOpacity(v)),
        );
        if tailed {
            let tail = &spec.tail.value;
            let dt = Tail::default();
            rows::percent(
                ui,
                at,
                &mut steps,
                "tail_base",
                "Tail width",
                tail.base,
                dt.base,
                (Tail::MIN_BASE.get(), Tail::MAX_BASE.get()),
                |v| style(TextStyleEdit::BubbleTailBase(v)),
            );
            let n = Numeric::new(
                "tail_bend",
                "Tail bend",
                tenths(tail.bend),
                tenths(dt.bend),
                -100.0..=100.0,
            )
            .decimals(0);
            let stored = tail.bend;
            rows::numeric(ui, at, n, &mut steps, |v| {
                let wanted = from_tenths(v);
                (wanted != stored).then(|| style(TextStyleEdit::BubbleTailBend(wanted)))
            });
        }
        padding_rows(
            ui,
            at,
            &mut steps,
            (b.pad_x, b.pad_y),
            (d.pad_x, d.pad_y),
            |x| style(TextStyleEdit::BubblePadX(x)),
            |y| style(TextStyleEdit::BubblePadY(y)),
        );
        rows::percent(
            ui,
            at,
            &mut steps,
            "roundness",
            "Roundness",
            b.roundness,
            d.roundness,
            (0, 1000),
            |v| style(TextStyleEdit::BubbleRoundness(v)),
        );
        border_rows(
            ui,
            at,
            &mut steps,
            b.border,
            d.border,
            |on| style(TextStyleEdit::BubbleBorder(on)),
            |c| style(TextStyleEdit::BubbleBorderColor(c)),
            |w| style(TextStyleEdit::BubbleBorderWidth(w)),
        );
    });
    env.commit(ui, steps)
}

/// Shout's spikes, depth and irregularity, and the dice for a new seed.
fn shout_rows(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    spec: &BubbleSpec,
    ds: &BubbleSpec,
) {
    rows::count(
        ui,
        at,
        steps,
        "spikes",
        "Spikes",
        spec.spikes,
        ds.spikes,
        (BubbleSpec::MIN_SPIKES, BubbleSpec::MAX_SPIKES),
        |n| style(TextStyleEdit::BubbleSpikes(n)),
    );
    rows::percent(
        ui,
        at,
        steps,
        "spike_depth",
        "Depth",
        spec.spike_depth,
        ds.spike_depth,
        (50, 400),
        |v| style(TextStyleEdit::BubbleSpikeDepth(v)),
    );
    rows::percent(
        ui,
        at,
        steps,
        "jitter",
        "Irregular",
        spec.jitter,
        ds.jitter,
        (0, 1000),
        |v| style(TextStyleEdit::BubbleJitter(v)),
    );
    let row = PropRow::new(at.id("seed"), "Burst")
        .hint("The same burst always comes back from the same seed")
        .parameter(120.0)
        .show(ui, |ui| {
            let dice = icon_button(ui, icons::DICE, 18.0, "Roll a new burst");
            ui.label(egui::RichText::new(format!("#{}", spec.seed)).monospace());
            dice
        });
    let dice = row.inner;
    let edit = dice
        .clicked()
        .then(|| style(TextStyleEdit::BubbleSeed(next_seed(spec.seed))));
    steps.button(dice, edit);
}

/// A new seed, different from `seed`: the clock mixed with the old seed
/// (SplitMix64's finaliser).
fn next_seed(seed: u32) -> u32 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    // Only the low 64 bits matter for mixing.
    let mut z = (now as u64) ^ u64::from(seed).rotate_left(32);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // The low 32 bits of the mix.
    let out = z as u32;
    if out == seed {
        out.wrapping_add(1)
    } else {
        out
    }
}

/// The Curve card (TX-016): the amount, −100 (bowl) to +100 (arch).
pub fn curve_card(ui: &mut egui::Ui, env: &mut CardEnv<'_>, s: &TextStyle) -> Option<String> {
    let at = env.at(CardId::Curve);
    let mut steps = Steps::default();
    let stored = s.curve.value;
    let n = Numeric::new("amount", "Amount", tenths(stored), 0.0, -100.0..=100.0).decimals(0);
    rows::numeric(ui, at, n, &mut steps, |v| {
        let wanted = from_tenths(v);
        (wanted != stored).then(|| style(TextStyleEdit::Curve(wanted)))
    });
    env.commit(ui, steps)
}

/// "Paused while the text is curved" at the top of a box card (TX-013,
/// TX-055): the settings stay editable.
fn paused_note(ui: &mut egui::Ui, s: &TextStyle) {
    if s.is_curved() {
        let p = crate::theme::look(ui.ctx()).palette;
        let mut job = crate::widgets::icon_then_text(
            ui,
            icons::INFO,
            "Paused while the text is curved (see Curve)",
        );
        for section in &mut job.sections {
            section.format.color = p.text_tertiary;
        }
        ui.label(job);
    }
}

/// Horizontal and vertical padding, 0–100 % of the font size.
fn padding_rows(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    (x, y): (PerMille, PerMille),
    (dx, dy): (PerMille, PerMille),
    make_x: impl Fn(PerMille) -> Edit,
    make_y: impl Fn(PerMille) -> Edit,
) {
    rows::percent(
        ui,
        at,
        steps,
        "pad_x",
        "Padding X",
        x,
        dx,
        (0, 1000),
        make_x,
    );
    rows::percent(
        ui,
        at,
        steps,
        "pad_y",
        "Padding Y",
        y,
        dy,
        (0, 1000),
        make_y,
    );
}

/// A border's switch, colour and width (0–20 % of the font size).
#[allow(clippy::too_many_arguments)]
fn border_rows(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    border: Layer<freecut_timeline::model::Border>,
    default: Layer<freecut_timeline::model::Border>,
    make_on: impl Fn(bool) -> Edit,
    make_color: impl Fn(freecut_timeline::model::Rgba8) -> Edit,
    make_width: impl Fn(PerMille) -> Edit,
) {
    rows::switch(
        ui, at, steps, "border", "Border", border.on, default.on, make_on,
    );
    if border.on {
        rows::color(
            ui,
            at,
            steps,
            "border_color",
            "Border color",
            border.value.color,
            default.value.color,
            make_color,
        );
        rows::percent(
            ui,
            at,
            steps,
            "border_width",
            "Border width",
            border.value.width,
            default.value.width,
            (0, freecut_timeline::model::Border::MAX_WIDTH.get()),
            make_width,
        );
    }
}

/// The Background card's chip: its look, or why it is off or paused.
pub fn background_summary(s: &TextStyle) -> String {
    let b = &s.background.value;
    if !s.background.on {
        return if s.bubble.on {
            "off · the bubble is on".to_owned()
        } else {
            "off".to_owned()
        };
    }
    if s.is_curved() {
        return "paused while curved".to_owned();
    }
    let shape = match b.shape {
        BoxShape::Block => "Block",
        BoxShape::PerLine => "Lines",
        BoxShape::PerChar => "Letters",
    };
    format!(
        "{shape} · {} · {:.0} %",
        hex(b.color),
        b.opacity.as_f64() * 100.0
    )
}

/// The Bubble card's chip.
pub fn bubble_summary(s: &TextStyle) -> String {
    let b = &s.bubble.value;
    if !s.bubble.on {
        return if s.background.on {
            "off · the background is on".to_owned()
        } else {
            "off".to_owned()
        };
    }
    if s.is_curved() {
        return "paused while curved".to_owned();
    }
    let tail = if kind_has_tail(b.spec.kind) && b.spec.tail.on {
        " · tail"
    } else {
        ""
    };
    format!("{}{tail} · {}", kind_name(b.spec.kind), hex(b.fill))
}

/// The Curve card's chip: "+30", "arch +30", "off".
pub fn curve_summary(s: &TextStyle) -> String {
    if !s.curve.on {
        return "off".to_owned();
    }
    let v = tenths(s.curve.value);
    if s.curve.value == SignedPerMille::ZERO {
        "straight".to_owned()
    } else if v > 0.0 {
        format!("arch +{v:.0}")
    } else {
        format!("bowl {v:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecut_timeline::model::TextLayerKind;

    #[test]
    fn chips_say_why_a_box_layer_is_off_or_paused() {
        let s = TextStyle::default();
        assert_eq!(background_summary(&s), "off");
        let boxed = s.with_layer(TextLayerKind::Background, true);
        assert_eq!(background_summary(&boxed), "Block · #000000 · 70 %");
        assert_eq!(bubble_summary(&boxed), "off · the background is on");
        let bubbled = boxed.with_layer(TextLayerKind::Bubble, true);
        assert_eq!(background_summary(&bubbled), "off · the bubble is on");
        assert_eq!(bubble_summary(&bubbled), "Rounded · tail · #1E1E24");
        let mut curved = bubbled.with_layer(TextLayerKind::Curve, true);
        curved.curve.value = SignedPerMille::new(300);
        assert_eq!(bubble_summary(&curved), "paused while curved");
        assert_eq!(curve_summary(&curved), "arch +30");
        assert_eq!(
            kind_name(BubbleKind::ArrowTag { point: Side::Left }),
            "Arrow tag"
        );
        assert!(!kind_has_tail(BubbleKind::Ticket));
    }

    #[test]
    fn the_dice_always_gives_another_seed() {
        for seed in [0, 1, u32::MAX] {
            assert_ne!(next_seed(seed), seed);
        }
    }
}
