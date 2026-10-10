//! Bubble and tag boxes around a text clip (TX-055). The geometry is drawn
//! by `freecut-render::shape::bubble` from these parameters.

use super::style::{Border, Layer};
use super::units::{SignedPerMille, clamp_pm};
use super::{RefPx, Rgba8};
use crate::model::PerMille;

/// Which end of an arrow tag points (TX-055).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    #[default]
    Right,
}

/// The bubble shapes (TX-055), all FreeCut's own and drawn from parameters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BubbleKind {
    /// A rounded rectangle with a tail.
    #[default]
    Rounded,
    /// An ellipse with a tail.
    Oval,
    /// A cloud of lobes with a trail of shrinking circles.
    Thought,
    /// A burst with spikes.
    Shout,
    /// A sharp-cornered box with a tail.
    Square,
    /// A pill-shaped label, no tail.
    Tag,
    /// A label with one pointed end.
    ArrowTag { point: Side },
    /// A rectangle with notched corners, no tail.
    Ticket,
}

impl BubbleKind {
    /// Whether this kind shows its tail by default (TX-055): Rounded, Oval,
    /// Square and Shout do; Thought draws its trail on the same switch;
    /// Tag, Arrow tag and Ticket don't.
    pub const fn tail_by_default(self) -> bool {
        matches!(
            self,
            BubbleKind::Rounded
                | BubbleKind::Oval
                | BubbleKind::Square
                | BubbleKind::Shout
                | BubbleKind::Thought
        )
    }
}

/// The tail tip, in ref px from the text box centre in the clip's local
/// frame (TX-055), +x right, +y down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TailTip {
    pub x: RefPx,
    pub y: RefPx,
}

impl TailTip {
    /// ±40,000 ref px: 40 em at the largest size. The 40 em drag stop of
    /// TX-055 depends on the size and is the editor's; this bound keeps
    /// stored values sane.
    pub const LIMIT: RefPx = RefPx::from_px(40_000);

    fn clamped(self) -> TailTip {
        let l = Self::LIMIT.hundredths();
        TailTip {
            x: RefPx::from_hundredths(self.x.hundredths().clamp(-l, l)),
            y: RefPx::from_hundredths(self.y.hundredths().clamp(-l, l)),
        }
    }
}

/// A bubble's tail (TX-055).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tail {
    pub tip: TailTip,
    /// Width at the base: 100–600 per-mille of the body's shorter side.
    pub base: PerMille,
    /// −1000..=1000 (the UI's −100 to +100); 0 is a straight wedge.
    pub bend: SignedPerMille,
}

impl Tail {
    pub const MIN_BASE: PerMille = PerMille::new(100);
    pub const MAX_BASE: PerMille = PerMille::new(600);

    fn clamped(self) -> Tail {
        Tail {
            tip: self.tip.clamped(),
            base: clamp_pm(self.base, Self::MIN_BASE.get(), Self::MAX_BASE.get()),
            bend: self.bend.clamp(-1000, 1000),
        }
    }
}

impl Default for Tail {
    /// The model's default tip is TX-055's rule ("a quarter of the box width
    /// left of centre, 1.2 em below the bottom edge") applied to the factory
    /// text, "Your text" at 64 ref px in a default bubble: (−90, 135) ref px.
    /// The app may send the rule applied to the real box with the switch-on
    /// gesture. Base 24 %, bend 20 (TX-055).
    fn default() -> Tail {
        Tail {
            tip: TailTip {
                x: RefPx::from_px(-90),
                y: RefPx::from_px(135),
            },
            base: PerMille::new(240),
            bend: SignedPerMille::new(200),
        }
    }
}

/// The shape of a bubble (TX-055): kind, tail and the kind-specific extras.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BubbleSpec {
    pub kind: BubbleKind,
    /// The tail, or Thought's trail of circles.
    pub tail: Layer<Tail>,
    /// Shout: 8–32 spikes.
    pub spikes: u8,
    /// Shout: 50–400 per-mille of the body's shorter side.
    pub spike_depth: PerMille,
    /// Shout: irregularity, 0–1000.
    pub jitter: PerMille,
    /// Shout: the seed of the irregularity; the same seed always gives the
    /// same burst.
    pub seed: u32,
    /// Thought: lobe size, 100–400 per-mille of the body's shorter side.
    pub lobe: PerMille,
    /// Thought: 2–4 trail circles.
    pub trail: u8,
}

impl BubbleSpec {
    pub const MIN_SPIKES: u8 = 8;
    pub const MAX_SPIKES: u8 = 32;
    pub const MIN_TRAIL: u8 = 2;
    pub const MAX_TRAIL: u8 = 4;

    fn clamped(self) -> BubbleSpec {
        BubbleSpec {
            kind: self.kind,
            tail: self.tail.map(Tail::clamped),
            spikes: self.spikes.clamp(Self::MIN_SPIKES, Self::MAX_SPIKES),
            spike_depth: clamp_pm(self.spike_depth, 50, 400),
            jitter: clamp_pm(self.jitter, 0, 1000),
            seed: self.seed,
            lobe: clamp_pm(self.lobe, 100, 400),
            trail: self.trail.clamp(Self::MIN_TRAIL, Self::MAX_TRAIL),
        }
    }
}

impl Default for BubbleSpec {
    /// Rounded with its tail; Shout 14 spikes, depth 18 %, irregularity
    /// 30 %, seed 0; Thought lobes 22 %, 3 circles (TX-055).
    fn default() -> BubbleSpec {
        BubbleSpec {
            kind: BubbleKind::Rounded,
            tail: Layer::on(Tail::default()),
            spikes: 14,
            spike_depth: PerMille::new(180),
            jitter: PerMille::new(300),
            seed: 0,
            lobe: PerMille::new(220),
            trail: 3,
        }
    }
}

/// A bubble or tag box around the text (TX-055).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextBubble {
    pub spec: BubbleSpec,
    pub fill: Rgba8,
    pub opacity: PerMille,
    /// 0–1000 per-mille of the font size, as TX-013's box.
    pub pad_x: PerMille,
    pub pad_y: PerMille,
    pub roundness: PerMille,
    pub border: Layer<Border>,
}

impl TextBubble {
    pub(crate) fn clamped(self) -> TextBubble {
        TextBubble {
            spec: self.spec.clamped(),
            opacity: clamp_pm(self.opacity, 0, 1000),
            pad_x: clamp_pm(self.pad_x, 0, 1000),
            pad_y: clamp_pm(self.pad_y, 0, 1000),
            roundness: clamp_pm(self.roundness, 0, 1000),
            border: self.border.map(Border::clamped),
            fill: self.fill,
        }
    }
}

impl Default for TextBubble {
    /// White at 100 %, padding 45 % × 30 %, roundness 40 %, a dark border
    /// at 4 % (TX-055).
    fn default() -> TextBubble {
        TextBubble {
            spec: BubbleSpec::default(),
            fill: Rgba8::WHITE,
            opacity: PerMille::ONE,
            pad_x: PerMille::new(450),
            pad_y: PerMille::new(300),
            roundness: PerMille::new(400),
            border: Layer::on(Border {
                color: Rgba8::DARK,
                width: PerMille::new(40),
            }),
        }
    }
}
