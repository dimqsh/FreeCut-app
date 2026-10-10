//! Clip animation slots (FX-018 Entrance and Exit, FX-019 Continuous;
//! TX-022's In, Out and Loop on text): what a clip stores, the slot policy
//! per clip kind, and the effective windows each slot plays in.
//!
//! Stored values are never rewritten by trims: when In + Out don't fit the
//! clip, the effective durations are scaled down proportionally where the
//! windows are computed (FX-018).

use std::sync::Arc;

use super::animate::Interp;
use super::clip::ClipContent;
use super::transform::PerMille;
use crate::time::{TICKS_PER_SECOND, Tick};

/// Which animation slot (FX-018, FX-019). Text labels them In, Out and
/// Loop (TX-022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Slot {
    /// In: plays over the first part of the clip.
    Entrance,
    /// Out: plays over the last part of the clip.
    Exit,
    /// Loop: plays between them (text) or over the whole clip.
    Continuous,
}

impl Slot {
    pub const ALL: [Slot; 3] = [Slot::Entrance, Slot::Exit, Slot::Continuous];
}

/// A stable preset id such as `fc.in.fade` (TX-022, FX-030). Shared, so
/// clip clones stay cheap.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PresetId(Arc<str>);

impl PresetId {
    pub fn new(id: impl AsRef<str>) -> PresetId {
        PresetId(Arc::from(id.as_ref()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The entrance an exit id mirrors when the exit has no manifest of its
    /// own (FX-030: exits are derived from entrances): `fc.out.fade` →
    /// `fc.in.fade`.
    pub fn entrance_twin(&self) -> Option<PresetId> {
        let (head, rest) = self.0.split_once(".out.")?;
        Some(PresetId::new(format!("{head}.in.{rest}")))
    }
}

impl core::fmt::Display for PresetId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The side a directional entrance comes from, or an exit goes to
/// (FX-018), or a spin's sense (FX-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
    Clockwise,
    CounterClockwise,
}

impl Direction {
    /// The other side (exits default to the opposite of their entrance's
    /// default, FX-030).
    pub const fn opposite(self) -> Direction {
        match self {
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Clockwise => Direction::CounterClockwise,
            Direction::CounterClockwise => Direction::Clockwise,
        }
    }
}

/// The pieces a preset moves (TX-023's **Pieces**).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnimUnit {
    /// The whole text (or the whole layer).
    Block,
    Line,
    Word,
    /// A grapheme cluster.
    Letter,
}

/// The order units start in (TX-023).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnimOrder {
    #[default]
    Forward,
    Backward,
    CenterOut,
    /// The first and last units first, ending in the middle.
    EdgesIn,
    /// Seeded by [`AnimParams::seed`], so deterministic.
    Random,
}

/// Where a slide starts (FX-030).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Travel {
    /// Just outside the canvas edge on the chosen side.
    #[default]
    FromEdge,
    /// A short distance away: 0.15·k of the canvas short edge (3·k em on
    /// text).
    Short,
}

/// The settings of an animation (FX-018, FX-030, TX-023).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimParams {
    /// TX-023's unit; `None` takes the preset's default unit.
    pub unit: Option<AnimUnit>,
    /// 0 (one after another) to 1000 (all together). Default 500.
    pub overlap: PerMille,
    pub order: AnimOrder,
    /// The seed of random orders and seeded motions, stored so preview,
    /// export and every OS agree.
    pub seed: u32,
    /// TX-022's **Pace**: time per unit (0.02–1.0 s). When set, the slot
    /// duration is computed from it ([`crate::motion::pace_duration`]) and
    /// recomputed whenever the text changes.
    pub pace: Option<Tick>,
    /// FX-030's Intensity in per-mille of its 0–100 scale: 500 is the
    /// default amplitude (`k` = 1), 1000 doubles it.
    pub intensity: PerMille,
    pub travel: Travel,
    /// TX-056's **Caret** option of Typewriter (and any preset whose
    /// letters appear with a step) in an In or Out: a bar after the last
    /// revealed letter that blinks once typing ends. Off by default;
    /// other presets ignore it.
    pub caret: bool,
}

impl AnimParams {
    /// FX-030's default intensity, 50.
    pub const DEFAULT_INTENSITY: PerMille = PerMille::new(500);
    /// The shortest and longest Pace (TX-022): 0.02 s and 1 s.
    pub const MIN_PACE: Tick = Tick::from_raw(TICKS_PER_SECOND / 50);
    pub const MAX_PACE: Tick = Tick::from_raw(TICKS_PER_SECOND);

    /// `k`: Intensity / 50, for the evaluation edge.
    pub fn k(&self) -> f64 {
        f64::from(self.intensity.get()) / 500.0
    }

    /// Every value inside its range: overlap and intensity at most 1000,
    /// Pace within 0.02–1 s.
    pub fn clamped(&self) -> AnimParams {
        AnimParams {
            overlap: PerMille::new(self.overlap.get().min(1000)),
            intensity: PerMille::new(self.intensity.get().min(1000)),
            pace: self.pace.map(|p| p.clamp(Self::MIN_PACE, Self::MAX_PACE)),
            ..*self
        }
    }
}

impl Default for AnimParams {
    fn default() -> AnimParams {
        AnimParams {
            unit: None,
            overlap: PerMille::new(500),
            order: AnimOrder::Forward,
            seed: 0,
            pace: None,
            intensity: Self::DEFAULT_INTENSITY,
            travel: Travel::FromEdge,
            caret: false,
        }
    }
}

/// One slot's animation (FX-018, FX-019).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AnimationSpec {
    pub preset: PresetId,
    /// The slot length (In and Out), or the cycle length (Loop). Stored as
    /// set; FX-018's proportional scaling applies at evaluation.
    pub duration: Tick,
    /// FX-018's easing override (any TR-031 preset but Hold); `None` keeps
    /// the preset's built-in curve.
    pub easing: Option<Interp>,
    /// `None` takes the preset's default direction.
    pub direction: Option<Direction>,
    pub params: AnimParams,
    /// FX-019's phase offset (Continuous only): loops are evaluated at
    /// clip-local time plus this, so a split continues the motion
    /// seamlessly on the right piece.
    pub phase: Tick,
    /// TX-022's seams as they were before a split (Continuous only; `None`
    /// on a slot never split): the pieces keep easing in and out exactly
    /// as the unsplit clip did ([`AnimationSpec::seams_in`]).
    pub seams: Option<LoopSeams>,
}

/// Where a Continuous slot eases in and out (TX-022's seam rule), in loop
/// time: clip-local time plus [`AnimationSpec::phase`], which a split
/// keeps continuous across the cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoopSeams {
    /// Where the slot eases in from: In's end, if an In plays before it.
    pub start: Option<Tick>,
    /// Where it eases out to: Out's start, if an Out follows it.
    pub end: Option<Tick>,
    /// The length of both ramps: `min(250 ms, window / 4)` of the window
    /// they were measured on.
    pub ramp: Tick,
}

impl LoopSeams {
    /// TX-022's longest ramp: 250 ms.
    pub const MAX_RAMP: Tick = Tick::from_raw(TICKS_PER_SECOND / 4);
}

impl AnimationSpec {
    /// FX-018's shortest In or Out: 0.1 s.
    pub const MIN_DURATION: Tick = Tick::from_raw(TICKS_PER_SECOND / 10);
    /// FX-019's longest cycle: 10 s.
    pub const MAX_CYCLE: Tick = Tick::from_raw(10 * TICKS_PER_SECOND);

    /// `preset` for `duration` with default settings.
    pub fn new(preset: PresetId, duration: Tick) -> AnimationSpec {
        AnimationSpec {
            preset,
            duration,
            easing: None,
            direction: None,
            params: AnimParams::default(),
            phase: Tick::ZERO,
            seams: None,
        }
    }

    /// The settings within their ranges (durations are limited per clip by
    /// the edit, not here).
    pub fn clamped(&self) -> AnimationSpec {
        AnimationSpec {
            params: self.params.clamped(),
            easing: self.easing.filter(|e| *e != Interp::Hold),
            ..self.clone()
        }
    }

    /// In range: a positive duration and clamped settings.
    pub fn is_valid(&self) -> bool {
        self.duration.is_positive() && self.clamped() == *self
    }

    /// The seams of this spec playing as a Continuous slot in `window` on
    /// a clip of `clip_len` (TX-022), in loop time: where the window starts
    /// after the clip start (an In plays before it) and where it ends
    /// before the clip end (an Out follows), with ramps of
    /// `min(250 ms, window / 4)`. Where the window reaches a clip edge
    /// instead, a seam a split cut away ([`AnimationSpec::seams`]) still
    /// counts when it lies at or beyond that edge, and a split piece keeps
    /// the unsplit ramp length; a stored seam inside the clip (its In or Out
    /// removed since) is ignored. `None` without any seam.
    pub fn seams_in(&self, window: SlotWindow, clip_len: Tick) -> Option<LoopSeams> {
        let phase = i128::from(self.phase.raw());
        let loop_time = |t: Tick| {
            let v = i128::from(t.raw()) + phase;
            Tick::from_raw(i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX }))
        };
        let live_start = (window.start.raw() > 0).then(|| loop_time(window.start));
        let live_end = (window.end() < clip_len).then(|| loop_time(window.end()));
        let (clip_start, clip_end) = (loop_time(Tick::ZERO), loop_time(clip_len));
        let stored = self.seams.as_ref();
        let start =
            live_start.or_else(|| stored.and_then(|s| s.start).filter(|s| *s <= clip_start));
        let end = live_end.or_else(|| stored.and_then(|s| s.end).filter(|e| *e >= clip_end));
        if start.is_none() && end.is_none() {
            return None;
        }
        let ramp = stored.map_or_else(
            || Tick::from_raw((window.len.raw() / 4).clamp(0, LoopSeams::MAX_RAMP.raw())),
            |s| s.ramp,
        );
        Some(LoopSeams { start, end, ramp })
    }
}

/// How the three slots combine on a clip kind (FX-019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotPolicy {
    /// Video, image and generated clips: Continuous excludes Entrance and
    /// Exit, and the reverse.
    Exclusive,
    /// Text: Loop coexists with In and Out and plays between them, in
    /// `[in, d − out)` (TX-022).
    LoopBetween,
}

/// The slot policy of a clip's content (FX-019); `None` for audio clips,
/// which have no animation slots.
pub fn slot_policy(content: &ClipContent) -> Option<SlotPolicy> {
    match content {
        ClipContent::Text(_) => Some(SlotPolicy::LoopBetween),
        c if c.is_visual() => Some(SlotPolicy::Exclusive),
        _ => None,
    }
}

/// Where a slot plays, in clip-local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotWindow {
    pub start: Tick,
    pub len: Tick,
}

impl SlotWindow {
    pub fn end(self) -> Tick {
        Tick::from_raw(self.start.raw().saturating_add(self.len.raw()))
    }

    pub fn contains(self, t: Tick) -> bool {
        t >= self.start && t < self.end()
    }
}

/// The effective windows of a clip's slots (FX-018, FX-019, TX-022).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AnimWindows {
    pub entrance: Option<SlotWindow>,
    pub exit: Option<SlotWindow>,
    pub continuous: Option<SlotWindow>,
}

/// The slot playing at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveSlot<'a> {
    pub slot: Slot,
    pub window: SlotWindow,
    pub spec: &'a AnimationSpec,
}

/// A clip's animations (FX-018): stored on every visual clip, empty on
/// audio clips. Each slot is shared, so the many clip clones of ripple
/// edits and undo snapshots stay small and cheap.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ClipAnimations {
    pub entrance: Option<Arc<AnimationSpec>>,
    pub exit: Option<Arc<AnimationSpec>>,
    pub continuous: Option<Arc<AnimationSpec>>,
}

impl ClipAnimations {
    pub fn is_empty(&self) -> bool {
        self.entrance.is_none() && self.exit.is_none() && self.continuous.is_none()
    }

    pub fn get(&self, slot: Slot) -> Option<&AnimationSpec> {
        match slot {
            Slot::Entrance => self.entrance.as_deref(),
            Slot::Exit => self.exit.as_deref(),
            Slot::Continuous => self.continuous.as_deref(),
        }
    }

    fn slot_mut(&mut self, slot: Slot) -> &mut Option<Arc<AnimationSpec>> {
        match slot {
            Slot::Entrance => &mut self.entrance,
            Slot::Exit => &mut self.exit,
            Slot::Continuous => &mut self.continuous,
        }
    }

    /// `slot` set to `spec`, with the policy's side effects (FX-019): under
    /// [`SlotPolicy::Exclusive`] choosing a Continuous animation clears
    /// Entrance and Exit, and choosing either of those clears Continuous.
    /// Clearing a slot never touches the others.
    pub fn with(
        &self,
        slot: Slot,
        spec: Option<AnimationSpec>,
        policy: SlotPolicy,
    ) -> ClipAnimations {
        let mut out = self.clone();
        if spec.is_some() && policy == SlotPolicy::Exclusive {
            match slot {
                Slot::Continuous => {
                    out.entrance = None;
                    out.exit = None;
                }
                Slot::Entrance | Slot::Exit => out.continuous = None,
            }
        }
        *out.slot_mut(slot) = spec.map(Arc::new);
        out
    }

    /// Whether the slots obey `policy` and every spec is in range.
    pub fn is_valid(&self, policy: SlotPolicy) -> bool {
        let exclusive_ok = policy == SlotPolicy::LoopBetween
            || self.continuous.is_none()
            || (self.entrance.is_none() && self.exit.is_none());
        let specs_ok = [&self.entrance, &self.exit, &self.continuous]
            .into_iter()
            .flatten()
            .all(|s| s.is_valid());
        exclusive_ok && specs_ok
    }

    /// The effective In and Out lengths on a clip of `clip_dur` (FX-018):
    /// as stored, or scaled down proportionally when together they exceed
    /// the clip (In rounded down, Out takes the rest), so they never
    /// overlap.
    pub fn effective_in_out(&self, clip_dur: Tick) -> (Tick, Tick) {
        let len =
            |s: &Option<Arc<AnimationSpec>>| s.as_ref().map_or(0, |s| s.duration.raw().max(0));
        let (a, b, d) = (len(&self.entrance), len(&self.exit), clip_dur.raw().max(0));
        let total = i128::from(a) + i128::from(b);
        if total <= i128::from(d) {
            return (Tick::from_raw(a), Tick::from_raw(b));
        }
        // a·d / (a + b) ≤ d fits an i64.
        let a2 = i64::try_from(i128::from(a) * i128::from(d) / total).unwrap_or(d);
        let b2 = if b > 0 { d - a2 } else { 0 };
        (Tick::from_raw(a2), Tick::from_raw(b2))
    }

    /// The windows each slot plays in on a clip of `clip_dur` under
    /// `policy` (FX-018, FX-019, TX-022). Clips shorter than two frames of
    /// `frame` play no animation. In covers `[0, in)`, Out `[d − out, d)`;
    /// a text Loop plays in `[in, d − out)` (none when that is empty), any
    /// other Continuous over the whole clip.
    pub fn windows(&self, policy: SlotPolicy, clip_dur: Tick, frame: Tick) -> AnimWindows {
        let d = clip_dur.raw();
        if d < frame.raw().saturating_mul(2) || d <= 0 {
            return AnimWindows::default();
        }
        let (a, b) = self.effective_in_out(clip_dur);
        let win = |start: i64, len: i64| {
            (len > 0).then_some(SlotWindow {
                start: Tick::from_raw(start),
                len: Tick::from_raw(len),
            })
        };
        let entrance = self.entrance.as_ref().and_then(|_| win(0, a.raw()));
        let exit = self.exit.as_ref().and_then(|_| win(d - b.raw(), b.raw()));
        let continuous = self.continuous.as_ref().and_then(|_| match policy {
            SlotPolicy::LoopBetween => win(a.raw(), d - a.raw() - b.raw()),
            // A hand-edited file with all three: Entrance and Exit win
            // their windows, Continuous plays only where they don't.
            SlotPolicy::Exclusive => win(0, d),
        });
        AnimWindows {
            entrance,
            exit,
            continuous,
        }
    }

    /// The slot playing at clip-local time `local`, if any. Only one slot
    /// plays at a time: In, then Out, then Loop.
    pub fn active_at(
        &self,
        policy: SlotPolicy,
        clip_dur: Tick,
        frame: Tick,
        local: Tick,
    ) -> Option<ActiveSlot<'_>> {
        let w = self.windows(policy, clip_dur, frame);
        let pick = |slot: Slot, window: Option<SlotWindow>| {
            let window = window.filter(|w| w.contains(local))?;
            Some(ActiveSlot {
                slot,
                window,
                spec: self.get(slot)?,
            })
        };
        pick(Slot::Entrance, w.entrance)
            .or_else(|| pick(Slot::Exit, w.exit))
            .or_else(|| pick(Slot::Continuous, w.continuous))
    }

    /// FX-018's and FX-019's split rule: In stays on the left piece and Out
    /// on the right; Continuous stays on both, the right piece's phase
    /// advanced by the left piece's length `left_len` so the motion
    /// continues seamlessly.
    ///
    /// Both pieces also keep the seams the slot had on the whole clip of
    /// `clip_dur` under `policy` (TX-022, [`AnimationSpec::seams_in`]), so
    /// a Loop that eased in after the In or out before the Out still does
    /// on each piece, and the split changes no frame.
    pub(crate) fn split(
        &self,
        left_len: Tick,
        clip_dur: Tick,
        policy: Option<SlotPolicy>,
    ) -> (ClipAnimations, ClipAnimations) {
        let seams = match (&self.continuous, policy) {
            (Some(c), Some(policy)) => self
                .windows(policy, clip_dur, Tick::ZERO)
                .continuous
                .and_then(|w| c.seams_in(w, clip_dur)),
            _ => None,
        };
        let mut left = ClipAnimations {
            exit: None,
            ..self.clone()
        };
        let mut right = ClipAnimations {
            entrance: None,
            ..self.clone()
        };
        if let Some(c) = &mut left.continuous
            && c.seams != seams
        {
            Arc::make_mut(c).seams = seams;
        }
        if let Some(c) = &mut right.continuous {
            let c = Arc::make_mut(c);
            c.phase = Tick::from_raw(c.phase.raw().saturating_add(left_len.raw()));
            c.seams = seams;
        }
        (left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(v: i64) -> Tick {
        Tick::from_raw(v)
    }

    fn spec(d: i64) -> AnimationSpec {
        AnimationSpec::new(PresetId::new("fc.in.fade"), t(d))
    }

    fn shared(d: i64) -> Option<Arc<AnimationSpec>> {
        Some(Arc::new(spec(d)))
    }

    #[test]
    fn exclusive_policy_clears_the_other_kind_and_text_keeps_all() {
        let a = ClipAnimations::default()
            .with(Slot::Entrance, Some(spec(10)), SlotPolicy::Exclusive)
            .with(Slot::Exit, Some(spec(10)), SlotPolicy::Exclusive);
        let looped = a.with(Slot::Continuous, Some(spec(5)), SlotPolicy::Exclusive);
        assert!(looped.entrance.is_none() && looped.exit.is_none());
        let back = looped.with(Slot::Entrance, Some(spec(5)), SlotPolicy::Exclusive);
        assert!(back.continuous.is_none() && back.entrance.is_some());
        let text = a.with(Slot::Continuous, Some(spec(5)), SlotPolicy::LoopBetween);
        assert!(text.entrance.is_some() && text.exit.is_some() && text.continuous.is_some());
        let cleared = text.with(Slot::Entrance, None, SlotPolicy::Exclusive);
        assert!(
            cleared.continuous.is_some(),
            "clearing touches nothing else"
        );
        assert!(!text.is_valid(SlotPolicy::Exclusive));
        assert!(text.is_valid(SlotPolicy::LoopBetween));
    }

    #[test]
    fn in_and_out_scale_down_proportionally() {
        let a = ClipAnimations {
            entrance: shared(30),
            exit: shared(10),
            continuous: shared(7),
        };
        assert_eq!(a.effective_in_out(t(100)), (t(30), t(10)));
        assert_eq!(a.effective_in_out(t(20)), (t(15), t(5)));
        assert_eq!(a.effective_in_out(t(7)), (t(5), t(2)));
        let w = a.windows(SlotPolicy::LoopBetween, t(100), t(1));
        assert_eq!(
            w.entrance,
            Some(SlotWindow {
                start: t(0),
                len: t(30)
            })
        );
        assert_eq!(
            w.exit,
            Some(SlotWindow {
                start: t(90),
                len: t(10)
            })
        );
        assert_eq!(
            w.continuous,
            Some(SlotWindow {
                start: t(30),
                len: t(60)
            })
        );
        // In + Out fill the clip: Loop has no window but stays stored.
        let w = a.windows(SlotPolicy::LoopBetween, t(20), t(1));
        assert_eq!(w.continuous, None);
        assert_eq!(
            a.active_at(SlotPolicy::LoopBetween, t(100), t(1), t(30))
                .unwrap()
                .slot,
            Slot::Continuous
        );
        assert_eq!(
            a.active_at(SlotPolicy::LoopBetween, t(100), t(1), t(29))
                .unwrap()
                .slot,
            Slot::Entrance
        );
        assert_eq!(
            a.active_at(SlotPolicy::LoopBetween, t(100), t(1), t(90))
                .unwrap()
                .slot,
            Slot::Exit
        );
        // Shorter than two frames: nothing plays.
        assert_eq!(
            a.windows(SlotPolicy::LoopBetween, t(19), t(10)),
            AnimWindows::default()
        );
        // Only In, longer than the clip: the whole clip.
        let only = ClipAnimations {
            entrance: shared(50),
            ..ClipAnimations::default()
        };
        assert_eq!(only.effective_in_out(t(20)), (t(20), t(0)));
    }

    #[test]
    fn split_keeps_in_left_out_right_and_continues_the_loop() {
        let mut a = ClipAnimations {
            entrance: shared(3),
            exit: shared(4),
            continuous: shared(5),
        };
        Arc::make_mut(a.continuous.as_mut().unwrap()).phase = t(2);
        let (l, r) = a.split(t(40), t(100), Some(SlotPolicy::LoopBetween));
        assert!(l.entrance.is_some() && l.exit.is_none());
        assert!(r.entrance.is_none() && r.exit.is_some());
        assert_eq!(l.continuous.as_ref().unwrap().phase, t(2));
        assert_eq!(r.continuous.as_ref().unwrap().phase, t(42));
        // Both pieces remember the Loop's seams in loop time: after In
        // (3 + 2) and before Out (96 + 2), ramps of a quarter of 93.
        let seams = Some(LoopSeams {
            start: Some(t(5)),
            end: Some(t(98)),
            ramp: t(23),
        });
        assert_eq!(l.continuous.as_ref().unwrap().seams, seams);
        assert_eq!(r.continuous.as_ref().unwrap().seams, seams);
    }

    #[test]
    fn seams_cut_away_count_only_beyond_the_clip() {
        let mut s = spec(10);
        s.phase = t(40);
        s.seams = Some(LoopSeams {
            start: Some(t(30)),
            end: Some(t(200)),
            ramp: t(8),
        });
        // The middle piece of two splits, at 40 and 100: the In ended at
        // 30 and the Out starts at 200, both beyond it.
        let w = SlotWindow {
            start: t(0),
            len: t(60),
        };
        assert_eq!(s.seams_in(w, t(60)), s.seams);
        // A live Out wins over the stored end.
        let w = SlotWindow {
            start: t(0),
            len: t(50),
        };
        assert_eq!(s.seams_in(w, t(60)).unwrap().end, Some(t(90)));
        // A stored seam inside the clip is stale: its In was removed (and
        // the Out was never cut away).
        s.phase = t(0);
        s.seams = Some(LoopSeams {
            start: Some(t(30)),
            end: None,
            ramp: t(8),
        });
        let w = SlotWindow {
            start: t(0),
            len: t(60),
        };
        assert_eq!(s.seams_in(w, t(60)), None);
        // Never split: the live seams with their own ramp.
        s.seams = None;
        let w = SlotWindow {
            start: t(10),
            len: t(40),
        };
        assert_eq!(
            s.seams_in(w, t(60)),
            Some(LoopSeams {
                start: Some(t(10)),
                end: Some(t(50)),
                ramp: t(10),
            })
        );
    }

    #[test]
    fn params_clamp_and_ids_mirror() {
        let p = AnimParams {
            overlap: PerMille::new(4000),
            intensity: PerMille::new(1001),
            pace: Some(t(1)),
            ..AnimParams::default()
        }
        .clamped();
        assert_eq!((p.overlap.get(), p.intensity.get()), (1000, 1000));
        assert_eq!(p.pace, Some(AnimParams::MIN_PACE));
        assert!((AnimParams::default().k() - 1.0).abs() < 1e-12);
        assert_eq!(
            PresetId::new("fc.out.letter-rise").entrance_twin(),
            Some(PresetId::new("fc.in.letter-rise"))
        );
        assert_eq!(PresetId::new("fc.in.fade").entrance_twin(), None);
        let mut s = spec(10);
        s.easing = Some(Interp::Hold);
        assert!(!s.is_valid());
        assert_eq!(s.clamped().easing, None);
        assert!(!spec(0).is_valid());
    }
}
