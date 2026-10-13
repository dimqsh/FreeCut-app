//! Clip animations seen from the app (FX-018, FX-019, TX-022, TX-023,
//! ui-concept §7.2, §13.11): which presets each slot's grid offers, the
//! spec a tile chooses, the In / Loop / Out timing strip's geometry and
//! join drags, the Pace switch, and the shaded In and Out regions on
//! timeline clips. Pure maths: the Motion lens draws it and sends
//! `SetClipAnimation` requests.

use std::sync::Arc;

use freecut_timeline::model::{
    AnimUnit, AnimationSpec, Clip, PerMille, PresetId, Slot, SlotPolicy, slot_policy,
};
use freecut_timeline::motion::{
    Availability, PresetCatalog, PresetKind, PresetManifest, PresetTable, effective_unit,
};
use freecut_timeline::time::{FrameRate, Rounding, Tick};

/// One tile of a slot's preset grid.
#[derive(Debug, Clone)]
pub struct PresetChoice {
    /// The id stored when the tile is chosen.
    pub id: PresetId,
    pub name: String,
    /// The motion it plays: its own, or for an Out without a manifest of
    /// its own, its In played backwards (FX-030).
    pub manifest: Arc<PresetManifest>,
}

/// A preset's name from its id: `fc.in.letter_rise` → "Letter rise".
pub fn preset_name(id: &PresetId) -> String {
    let last = id.as_str().rsplit('.').next().unwrap_or(id.as_str());
    let words = last.replace(['_', '-'], " ");
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Whether `id` names a clip preset (FX-030: `fc.clip.…`, `resources/
/// animations/`) rather than a text preset (TX-056): the two grids never
/// share a preset.
pub fn is_clip_preset(id: &PresetId) -> bool {
    id.as_str().split('.').nth(1) == Some("clip")
}

/// The tiles of `slot`'s grid for a text clip (`text`, TX-056's presets)
/// or another visual clip (FX-030's clip presets), sorted by name: In
/// offers entrances; Out offers exits and the mirror of every entrance
/// without one (FX-018); Loop offers loops and drifts, plus combined
/// presets on non-text clips (FX-019). Clips other than text get
/// whole-layer presets only. A preset over FX-030's limits is never
/// offered; the check is the catalogue's cached one.
pub fn slot_presets(table: &PresetTable, slot: Slot, text: bool) -> Vec<PresetChoice> {
    let fits = |m: &PresetManifest| {
        is_clip_preset(&m.id) != text
            && (text || !m.is_per_unit())
            && table.availability(&m.id) == Availability::Ready
    };
    let mut out: Vec<PresetChoice> = Vec::new();
    for m in table.manifests().filter(|m| fits(m)) {
        if offered(slot, m.kind, text) {
            out.push(PresetChoice {
                id: m.id.clone(),
                name: preset_name(&m.id),
                manifest: Arc::clone(m),
            });
        }
    }
    if slot == Slot::Exit {
        for m in table
            .manifests()
            .filter(|m| m.kind == PresetKind::Entrance && fits(m))
        {
            let Some(id) = exit_twin(&m.id) else { continue };
            if out.iter().any(|c| c.id == id) {
                continue;
            }
            out.push(PresetChoice {
                name: preset_name(&id),
                id,
                manifest: Arc::clone(m),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Whether `slot`'s grid offers presets of `kind` (FX-018, FX-019).
fn offered(slot: Slot, kind: PresetKind, text: bool) -> bool {
    match (slot, kind) {
        (Slot::Entrance, PresetKind::Entrance)
        | (Slot::Exit, PresetKind::Exit)
        | (Slot::Continuous, PresetKind::Loop | PresetKind::Drift) => true,
        (Slot::Continuous, PresetKind::Combined) => !text,
        _ => false,
    }
}

/// A tile the grid shows but can't offer: a preset file this build can't
/// play (a channel it lacks, a newer schema), FX-018's **Unavailable
/// animation**, with the loader's reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableChoice {
    pub id: PresetId,
    pub name: String,
    pub reason: Arc<str>,
}

/// One group of a slot's grid: FX-019's kinds on the Loop grid (loops,
/// drifts, combined), one group on In and Out.
#[derive(Debug, Clone)]
pub struct GridGroup {
    /// The kind the group holds on the Loop grid; `None` on In and Out.
    pub kind: Option<PresetKind>,
    pub choices: Vec<PresetChoice>,
    pub unavailable: Vec<UnavailableChoice>,
}

impl GridGroup {
    /// The group's heading (FX-019's names for the kinds).
    pub fn title(&self) -> Option<&'static str> {
        match self.kind? {
            PresetKind::Loop => Some("Loops"),
            PresetKind::Drift => Some("Drifts"),
            PresetKind::Combined => Some("In and out"),
            PresetKind::Entrance | PresetKind::Exit => None,
        }
    }
}

/// Every grid of the Motion lens, built once per catalogue: the
/// photosensitivity check behind them is cached by the catalogue, and the
/// filtering, mirroring and grouping happen here once instead of on every
/// repaint.
#[derive(Debug, Clone)]
pub struct PresetGrids {
    /// By `(slot, text)`: [`Self::index`].
    grids: [Vec<GridGroup>; 6],
}

impl PresetGrids {
    pub fn new(table: &PresetTable) -> PresetGrids {
        let grids = [Slot::Entrance, Slot::Continuous, Slot::Exit]
            .into_iter()
            .flat_map(|slot| [true, false].map(|text| slot_groups(table, slot, text)));
        let mut out: [Vec<GridGroup>; 6] = Default::default();
        for (cell, groups) in out.iter_mut().zip(grids) {
            *cell = groups;
        }
        PresetGrids { grids: out }
    }

    fn index(slot: Slot, text: bool) -> usize {
        let s = match slot {
            Slot::Entrance => 0,
            Slot::Continuous => 1,
            Slot::Exit => 2,
        };
        s * 2 + usize::from(!text)
    }

    /// The groups of `slot`'s grid for a text clip (`text`) or another
    /// visual clip.
    pub fn groups(&self, slot: Slot, text: bool) -> &[GridGroup] {
        self.grids
            .get(Self::index(slot, text))
            .map_or(&[], Vec::as_slice)
    }
}

/// `slot`'s grid in groups: In and Out in one group sorted by name; Loop
/// grouped by kind in FX-019's order (loops, drifts, combined), each
/// sorted by name. Presets the catalogue lists as unavailable follow the
/// offered ones in the group their id names; a group with nothing in it is
/// left out.
pub fn slot_groups(table: &PresetTable, slot: Slot, text: bool) -> Vec<GridGroup> {
    let choices = slot_presets(table, slot, text);
    let mut unavailable: Vec<(PresetKind, UnavailableChoice)> = Vec::new();
    for (id, why) in table.unavailable() {
        if is_clip_preset(id) == text {
            continue;
        }
        let Some(kind) = kind_of_id(id) else { continue };
        let mut place = |id: PresetId, kind| {
            if !choices.iter().any(|c| c.id == id) && !unavailable.iter().any(|(_, u)| u.id == id) {
                unavailable.push((
                    kind,
                    UnavailableChoice {
                        name: preset_name(&id),
                        id,
                        reason: Arc::from(why),
                    },
                ));
            }
        };
        if offered(slot, kind, text) {
            place(id.clone(), kind);
        } else if slot == Slot::Exit
            && kind == PresetKind::Entrance
            && let Some(twin) = exit_twin(id)
            && table.get(&twin).is_none()
        {
            // Its mirrored Out can't play either.
            place(twin, PresetKind::Exit);
        }
    }
    unavailable.sort_by(|a, b| a.1.name.cmp(&b.1.name).then_with(|| a.1.id.cmp(&b.1.id)));
    let kinds: &[Option<PresetKind>] = match slot {
        Slot::Continuous => &[
            Some(PresetKind::Loop),
            Some(PresetKind::Drift),
            Some(PresetKind::Combined),
        ],
        Slot::Entrance | Slot::Exit => &[None],
    };
    kinds
        .iter()
        .map(|&kind| {
            let fits = |k: PresetKind| kind.is_none_or(|g| g == k);
            GridGroup {
                kind,
                choices: choices
                    .iter()
                    .filter(|c| fits(c.manifest.kind))
                    .cloned()
                    .collect(),
                unavailable: unavailable
                    .iter()
                    .filter(|(k, _)| fits(*k))
                    .map(|(_, u)| u.clone())
                    .collect(),
            }
        })
        .filter(|g| !g.choices.is_empty() || !g.unavailable.is_empty())
        .collect()
}

/// The kind a preset id names (`fc.in.x`, `fc.clip.drift.pan`, …), for
/// presets the loader couldn't read: the first of its parts that names a
/// kind.
pub fn kind_of_id(id: &PresetId) -> Option<PresetKind> {
    id.as_str().split('.').find_map(|part| match part {
        "in" => Some(PresetKind::Entrance),
        "out" => Some(PresetKind::Exit),
        "loop" => Some(PresetKind::Loop),
        "drift" => Some(PresetKind::Drift),
        "combined" => Some(PresetKind::Combined),
        _ => None,
    })
}

/// `fc.in.fade` → `fc.out.fade`: the exit that mirrors an entrance.
pub fn exit_twin(id: &PresetId) -> Option<PresetId> {
    let (head, rest) = id.as_str().split_once(".in.")?;
    Some(PresetId::new(format!("{head}.out.{rest}")))
}

/// The pieces a preset moves (TX-023): its units, or the whole block.
pub fn pieces_of(manifest: &PresetManifest) -> Vec<AnimUnit> {
    match &manifest.units {
        Some(units) if !units.is_empty() => units.clone(),
        _ => vec![AnimUnit::Block],
    }
}

/// The spec a tile chooses (TX-056's defaults). An exit mirroring an
/// entrance takes the entrance's settings under its own id; its direction
/// is left to the model, which turns it to the opposite side (FX-030).
pub fn chosen_spec(choice: &PresetChoice) -> AnimationSpec {
    let mut spec = choice.manifest.default_spec();
    spec.preset = choice.id.clone();
    spec
}

/// `spec` with Pieces set to `unit` when the preset moves that unit
/// (TX-022: a choice it doesn't support leaves the preset alone). Lines
/// and blocks have no Pace (TX-022).
pub fn with_unit(
    spec: &AnimationSpec,
    manifest: &PresetManifest,
    unit: AnimUnit,
) -> Option<AnimationSpec> {
    if !pieces_of(manifest).contains(&unit) || manifest.units.is_none() {
        return None;
    }
    let mut out = spec.clone();
    out.params.unit = Some(unit);
    if matches!(unit, AnimUnit::Line | AnimUnit::Block) {
        out.params.pace = None;
    }
    Some(out)
}

/// Whether the slot can take Pace: a per-unit In or Out on letters or
/// words (TX-022).
pub fn can_pace(slot: Slot, manifest: &PresetManifest, spec: &AnimationSpec) -> bool {
    slot != Slot::Continuous
        && matches!(
            effective_unit(manifest, spec, true),
            AnimUnit::Letter | AnimUnit::Word
        )
}

/// The Pace that makes `units` units at `overlap` last `len` (TX-022's
/// switch keeps the current length): the inverse of
/// `units × pace × (1 − overlap) + pace × overlap`, within 0.02–1 s.
pub fn pace_for_length(len: Tick, units: u32, overlap: PerMille) -> Tick {
    use freecut_timeline::model::AnimParams;
    let o = i128::from(overlap.get().min(1000));
    let n = i128::from(units.max(1));
    let weight = n * (1000 - o) + o;
    let pace = i128::from(len.raw().max(0)) * 1000 / weight.max(1);
    let pace = Tick::from_raw(i64::try_from(pace).unwrap_or(i64::MAX));
    pace.clamp(AnimParams::MIN_PACE, AnimParams::MAX_PACE)
}

/// `spec` switched to Pace (`on`) or back to a plain length, keeping the
/// current length (TX-022). Pace counts the effective unit, which is
/// stored with it.
pub fn with_pace(
    spec: &AnimationSpec,
    manifest: &PresetManifest,
    text: &str,
    on: bool,
) -> AnimationSpec {
    let mut out = spec.clone();
    if on {
        let unit = effective_unit(manifest, spec, true);
        let units = freecut_timeline::motion::count_units(text, unit);
        out.params.unit = Some(unit);
        out.params.pace = Some(pace_for_length(spec.duration, units, spec.params.overlap));
    } else {
        out.params.pace = None;
    }
    out
}

// ------------------------------------------------------ timing strip

/// The timing strip's geometry (ui-concept §13.11, TX-022): fractions of
/// the clip, In at the left, Out at the right, Loop between. An empty In
/// or Out shows as an empty segment of [`EMPTY_SHARE`] so it can be
/// clicked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StripLayout {
    pub dur: Tick,
    pub policy: SlotPolicy,
    /// Where the In segment ends and the Out segment starts, 0..=1.
    pub in_end: f64,
    pub out_start: f64,
    pub in_set: bool,
    pub out_set: bool,
    pub loop_set: bool,
    /// Effective lengths (FX-018's proportional scaling applied).
    pub in_len: Tick,
    pub out_len: Tick,
    /// In + Out were scaled down to fit the clip: hatched, with the hint
    /// "Shortened to fit the clip".
    pub shortened: bool,
}

/// The share of the strip an empty In or Out segment takes.
pub const EMPTY_SHARE: f64 = 0.15;

/// The strip of `clip`; `None` for clips without animation slots (audio)
/// or without length.
pub fn strip_layout(clip: &Clip) -> Option<StripLayout> {
    let policy = slot_policy(&clip.content)?;
    let dur = clip.duration();
    if !dur.is_positive() {
        return None;
    }
    let a = &clip.animations;
    let (in_len, out_len) = a.effective_in_out(dur);
    let d = dur.raw() as f64;
    let (in_set, out_set) = (a.entrance.is_some(), a.exit.is_some());
    let in_end = if in_set {
        in_len.raw() as f64 / d
    } else {
        EMPTY_SHARE
    };
    let out_start = if out_set {
        1.0 - out_len.raw() as f64 / d
    } else {
        1.0 - EMPTY_SHARE
    };
    let stored = |s: &Option<Arc<AnimationSpec>>| s.as_ref().map_or(Tick::ZERO, |s| s.duration);
    let shortened = in_len != stored(&a.entrance) || out_len != stored(&a.exit);
    Some(StripLayout {
        dur,
        policy,
        in_end,
        out_start: out_start.max(in_end),
        in_set,
        out_set,
        loop_set: a.continuous.is_some(),
        in_len,
        out_len,
        shortened,
    })
}

impl StripLayout {
    /// The slot under strip fraction `x` (clicking a segment selects it).
    pub fn slot_at(&self, x: f64) -> Slot {
        if x < self.in_end {
            Slot::Entrance
        } else if x >= self.out_start {
            Slot::Exit
        } else {
            Slot::Continuous
        }
    }

    /// The joins that can be dragged: the end of a set In and the start
    /// of a set Out, as (slot, fraction).
    pub fn joins(&self) -> Vec<(Slot, f64)> {
        let mut out = Vec::new();
        if self.in_set {
            out.push((Slot::Entrance, self.in_end));
        }
        if self.out_set {
            out.push((Slot::Exit, self.out_start));
        }
        out
    }
}

/// The shortest In or Out a join drag stores: FX-018's 0.1 s, rounded up
/// to a frame.
fn min_join_length(rate: FrameRate) -> Tick {
    rate.round_to_grid(AnimationSpec::MIN_DURATION, Rounding::Ceil)
        .unwrap_or(AnimationSpec::MIN_DURATION)
        .max(rate.frame_ticks())
}

/// The In (or Out) length a join dragged to strip fraction `x` asks for:
/// snapped to frames, at least FX-018's 0.1 s, and stopping at the other
/// slot's edge (`other`: the length the other slot stores), so In + Out
/// never exceed the clip through the strip (TX-022).
pub fn join_length(slot: Slot, x: f64, dur: Tick, other: Tick, rate: FrameRate) -> Tick {
    let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
    let share = match slot {
        Slot::Exit => 1.0 - x,
        _ => x,
    };
    let raw = Tick::from_raw((dur.raw() as f64 * share).round() as i64);
    let min = min_join_length(rate);
    let room = Tick::from_raw(dur.raw().saturating_sub(other.raw()));
    let max = rate.round_to_grid(room, Rounding::Floor).unwrap_or(room);
    let len = rate.quantize(raw).unwrap_or(raw);
    if max < min { min } else { len.clamp(min, max) }
}

/// The specs a drag of `slot`'s join to strip fraction `x` stores, in
/// order, as plain lengths (a dragged length replaces Pace, TX-022).
/// When In and Out are shortened to fit the clip, the other slot is also
/// stored at the length it shows, so the join moves from where it is drawn
/// and the other segment stays put; limiting the join by the other slot's
/// longer stored length instead would make one of them jump. The model
/// fits each slot into what the other one stores, so the dragged slot
/// first steps down to the shortest length to make room.
pub fn join_drag(
    clip: &Clip,
    layout: &StripLayout,
    slot: Slot,
    x: f64,
    rate: FrameRate,
) -> Vec<(Slot, AnimationSpec)> {
    let a = &clip.animations;
    let Some(dragged) = a.get(slot) else {
        return Vec::new();
    };
    let (other_slot, shown) = match slot {
        Slot::Exit => (Slot::Entrance, layout.in_len),
        _ => (Slot::Exit, layout.out_len),
    };
    let plain = |spec: &AnimationSpec, len: Tick| {
        let mut s = spec.clone();
        s.duration = len;
        s.params.pace = None;
        s
    };
    let min = min_join_length(rate);
    let floor = |t: Tick| rate.round_to_grid(t, Rounding::Floor).unwrap_or(t);
    let mut out = Vec::new();
    let mut other = a.get(other_slot).map_or(Tick::ZERO, |o| o.duration);
    if layout.shortened
        && let Some(o) = a.get(other_slot)
    {
        let room = Tick::from_raw(layout.dur.raw().saturating_sub(min.raw()));
        let pinned = floor(shown).min(floor(room)).max(min);
        if o.duration != pinned {
            out.push((slot, plain(dragged, min)));
            out.push((other_slot, plain(o, pinned)));
        }
        other = pinned;
    }
    let len = join_length(slot, x, layout.dur, other, rate);
    if dragged.duration != len || !out.is_empty() {
        out.push((slot, plain(dragged, len)));
    }
    out
}

/// The shaded In and Out regions of a timeline clip (TX-022, FX-018), in
/// sequence time: where the effective In and Out play.
pub fn shaded_regions(clip: &Clip, frame: Tick) -> Vec<(Slot, Tick, Tick)> {
    let Some(policy) = slot_policy(&clip.content) else {
        return Vec::new();
    };
    let w = clip.animations.windows(policy, clip.duration(), frame);
    let start = clip.start();
    [(Slot::Entrance, w.entrance), (Slot::Exit, w.exit)]
        .into_iter()
        .filter_map(|(slot, win)| {
            let win = win?;
            let a = start.checked_add(win.start).ok()?;
            let b = start.checked_add(win.end()).ok()?;
            Some((slot, a, b))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_timeline::edit::EditRequest;
    use freecut_timeline::history::Coalesce;
    use freecut_timeline::ids::ClipId;
    use freecut_timeline::model::ProjectSettings;
    use freecut_timeline::time::TICKS_PER_SECOND;

    use super::*;
    use crate::session::Project;

    fn ms(n: i64) -> Tick {
        Tick::from_raw(TICKS_PER_SECOND / 1000 * n)
    }

    fn rate() -> FrameRate {
        FrameRate::new(30, 1).unwrap()
    }

    fn table() -> PresetTable {
        PresetTable::samples()
    }

    fn text_clip() -> (Project, ClipId) {
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let clip = crate::ui::text::add_text_at_playhead(&mut project).unwrap();
        (project, clip)
    }

    fn set(project: &mut Project, clip: ClipId, slot: Slot, spec: Option<AnimationSpec>) {
        let req = EditRequest::SetClipAnimation {
            seq: project.doc().root(),
            clips: vec![clip],
            slot,
            spec,
        };
        project.apply(&req, Coalesce::Never).unwrap();
    }

    #[test]
    fn names_come_from_ids() {
        assert_eq!(
            preset_name(&PresetId::new("fc.in.letter_rise")),
            "Letter rise"
        );
        assert_eq!(preset_name(&PresetId::new("fc.loop.wave")), "Wave");
        assert_eq!(preset_name(&PresetId::new("plain")), "Plain");
        assert_eq!(
            exit_twin(&PresetId::new("fc.in.fade")),
            Some(PresetId::new("fc.out.fade"))
        );
        assert_eq!(exit_twin(&PresetId::new("fc.loop.wave")), None);
    }

    /// FX-018 / FX-019 / TX-022: each slot's grid, the derived Out tiles,
    /// and whole-layer presets only on clips other than text.
    #[test]
    fn each_slot_offers_its_own_presets() {
        let t = table();
        let ins = slot_presets(&t, Slot::Entrance, true);
        let names: Vec<&str> = ins.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Fade", "Pop", "Rise", "Slide", "Typewriter"]);
        let outs = slot_presets(&t, Slot::Exit, true);
        assert_eq!(outs.len(), ins.len(), "every In has a mirrored Out");
        assert!(outs.iter().all(|c| c.id.as_str().starts_with("fc.out.")));
        assert!(outs.iter().all(|c| c.manifest.kind == PresetKind::Entrance));
        let loops = slot_presets(&t, Slot::Continuous, true);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].id, PresetId::new("fc.loop.wave"));
        // A video gets the clip presets only, never text ones.
        assert!(slot_presets(&t, Slot::Entrance, false).is_empty());
        assert!(slot_presets(&t, Slot::Continuous, false).is_empty());
        let mut clip_fade = (*ins[0].manifest).clone();
        clip_fade.id = PresetId::new("fc.clip.in.fade");
        let mut manifests: Vec<PresetManifest> = t.manifests().map(|m| (**m).clone()).collect();
        manifests.push(clip_fade);
        let both = PresetTable::new(manifests);
        let video_in = slot_presets(&both, Slot::Entrance, false);
        assert_eq!(video_in.len(), 1);
        assert_eq!(video_in[0].id, PresetId::new("fc.clip.in.fade"));
        let video_out = slot_presets(&both, Slot::Exit, false);
        assert_eq!(video_out[0].id, PresetId::new("fc.clip.out.fade"));
        assert_eq!(slot_presets(&both, Slot::Entrance, true).len(), ins.len());
        for c in ins.iter().chain(&outs).chain(&loops) {
            assert!(
                freecut_timeline::motion::is_photosafe(&c.manifest),
                "{}",
                c.id
            );
        }
        // A mirrored Out keeps the entrance's settings under its own id.
        let fade_out = outs.iter().find(|c| c.name == "Fade").unwrap();
        let spec = chosen_spec(fade_out);
        assert_eq!(spec.preset, PresetId::new("fc.out.fade"));
        assert_eq!(spec.duration, fade_out.manifest.default_duration);
        assert_eq!(spec.direction, None);
    }

    /// FX-019: the Loop grid of a video groups loops, drifts and combined
    /// presets, in that order; In and Out are one group. Every bundled
    /// preset is offered once its catalogue's check is in.
    #[test]
    fn the_clip_loop_grid_is_grouped_by_kind() {
        let table = freecut_project::presets::bundled().table;
        let grids = PresetGrids::new(&table);
        let groups = grids.groups(Slot::Continuous, false);
        let kinds: Vec<Option<PresetKind>> = groups.iter().map(|g| g.kind).collect();
        assert_eq!(
            kinds,
            [
                Some(PresetKind::Loop),
                Some(PresetKind::Drift),
                Some(PresetKind::Combined)
            ]
        );
        let titles: Vec<_> = groups.iter().map(GridGroup::title).collect();
        assert_eq!(titles, [Some("Loops"), Some("Drifts"), Some("In and out")]);
        for g in groups {
            assert!(g.choices.iter().all(|c| Some(c.manifest.kind) == g.kind));
            let names: Vec<&str> = g.choices.iter().map(|c| c.name.as_str()).collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();
            assert_eq!(names, sorted, "each group by name");
            assert!(g.unavailable.is_empty());
        }
        let count = |slot, text| -> usize {
            grids
                .groups(slot, text)
                .iter()
                .map(|g| g.choices.len())
                .sum()
        };
        assert_eq!([groups[0].choices.len(), groups[1].choices.len()], [7, 3]);
        assert_eq!(
            count(Slot::Continuous, false),
            slot_presets(&table, Slot::Continuous, false).len()
        );
        // Text loops: one group; text has no combined presets (FX-019).
        assert_eq!(grids.groups(Slot::Continuous, true).len(), 1);
        assert_eq!(count(Slot::Continuous, true), 15);
        assert_eq!(grids.groups(Slot::Entrance, true).len(), 1);
        assert_eq!(grids.groups(Slot::Entrance, true)[0].kind, None);
        // The ten presets that once waited for a channel are offered.
        for (slot, text, name) in [
            (Slot::Entrance, true, "Scatter"),
            (Slot::Entrance, true, "Spread"),
            (Slot::Entrance, true, "Word rise"),
            (Slot::Exit, true, "Fall away"),
            (Slot::Continuous, true, "Shake"),
            (Slot::Continuous, true, "Flicker"),
            (Slot::Continuous, true, "Wiggle"),
            (Slot::Continuous, true, "Sparkle"),
            (Slot::Entrance, false, "Flip"),
            (Slot::Continuous, false, "Jitter"),
        ] {
            assert!(
                slot_presets(&table, slot, text)
                    .iter()
                    .any(|c| c.name == name),
                "{name} in {slot:?}"
            );
        }
    }

    /// FX-018: a preset the loader couldn't play is shown, with why, in the
    /// group its id names (and as its mirrored Out); one over FX-030's
    /// flash limit is never offered.
    #[test]
    fn unavailable_presets_are_listed_and_unsafe_ones_hidden() {
        use freecut_timeline::motion::{ChannelProp, ChannelUnit, Curve, CurvePoint};
        let mut manifests: Vec<PresetManifest> =
            table().manifests().map(|m| (**m).clone()).collect();
        let mut strobe = manifests
            .iter()
            .find(|m| m.id.as_str() == "fc.loop.wave")
            .unwrap()
            .clone();
        strobe.id = PresetId::new("fc.loop.strobe");
        strobe.units = None;
        strobe.channels = vec![freecut_timeline::motion::Channel {
            property: ChannelProp::Opacity,
            curve: Curve::sampled(
                (0..11)
                    .map(|i| CurvePoint {
                        t: f64::from(i) / 10.0,
                        v: if i % 2 == 0 { 0.0 } else { 1.0 },
                    })
                    .collect(),
                false,
            )
            .unwrap(),
            amplitude: -1.0,
            unit: ChannelUnit::Ratio,
            scales_with_intensity: false,
        }];
        manifests.push(strobe);
        let why: Arc<str> = Arc::from("unknown channel \"glitch\"");
        let t = PresetTable::new(manifests).with_unavailable([
            (PresetId::new("fc.in.glitch"), Arc::clone(&why)),
            (PresetId::new("fc.clip.drift.spiral"), Arc::clone(&why)),
            (PresetId::new("fc.loop.ripple"), Arc::clone(&why)),
        ]);
        let strobe = PresetId::new("fc.loop.strobe");
        assert!(matches!(t.availability(&strobe), Availability::Unsafe(_)));
        let grids = PresetGrids::new(&t);
        let loops = grids.groups(Slot::Continuous, true);
        assert!(
            loops
                .iter()
                .all(|g| g.choices.iter().all(|c| c.id != strobe))
        );
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].choices.len(), 1, "Wave");
        let names = |g: &GridGroup| -> Vec<String> {
            g.unavailable.iter().map(|u| u.name.clone()).collect()
        };
        assert_eq!(names(&loops[0]), ["Ripple"]);
        assert_eq!(loops[0].unavailable[0].reason, why);
        let ins = grids.groups(Slot::Entrance, true);
        assert_eq!(names(&ins[0]), ["Glitch"]);
        let outs = grids.groups(Slot::Exit, true);
        assert_eq!(outs[0].unavailable[0].id, PresetId::new("fc.out.glitch"));
        // A video's Loop grid: the drift waits in Drifts, the only group.
        let video = grids.groups(Slot::Continuous, false);
        assert_eq!(video.len(), 1);
        assert_eq!(video[0].kind, Some(PresetKind::Drift));
        assert_eq!(names(&video[0]), ["Spiral"]);
        assert!(grids.groups(Slot::Entrance, false).is_empty());
        assert_eq!(
            kind_of_id(&PresetId::new("org.example.loop.x")),
            Some(PresetKind::Loop)
        );
        assert_eq!(kind_of_id(&PresetId::new("plain")), None);
    }

    #[test]
    fn pieces_and_pace_follow_the_preset() {
        let t = table();
        let ins = slot_presets(&t, Slot::Entrance, true);
        let typewriter = ins.iter().find(|c| c.name == "Typewriter").unwrap();
        let fade = ins.iter().find(|c| c.name == "Fade").unwrap();
        assert_eq!(pieces_of(&fade.manifest), [AnimUnit::Block]);
        let spec = chosen_spec(typewriter);
        assert!(can_pace(Slot::Entrance, &typewriter.manifest, &spec));
        assert!(!can_pace(Slot::Continuous, &typewriter.manifest, &spec));
        assert!(with_unit(&spec, &fade.manifest, AnimUnit::Word).is_none());
        // A unit the preset doesn't move leaves it alone (TX-022).
        let offered = pieces_of(&typewriter.manifest);
        for unit in [
            AnimUnit::Block,
            AnimUnit::Line,
            AnimUnit::Word,
            AnimUnit::Letter,
        ] {
            assert_eq!(
                with_unit(&spec, &typewriter.manifest, unit).is_some(),
                offered.contains(&unit),
                "{unit:?}"
            );
        }

        // The Pace switch keeps the length (TX-022).
        let mut plain = spec.clone();
        plain.params.pace = None;
        plain.duration = ms(1000);
        let paced = with_pace(&plain, &typewriter.manifest, "Hello", true);
        let pace = paced.params.pace.unwrap();
        let again =
            freecut_timeline::motion::pace_duration(5, pace, paced.params.overlap, rate()).unwrap();
        assert!((again.raw() - ms(1000).raw()).abs() <= rate().frame_ticks().raw());
        assert_eq!(paced.params.unit, Some(AnimUnit::Letter));
        let off = with_pace(&paced, &typewriter.manifest, "Hello", false);
        assert_eq!(off.params.pace, None);
        assert_eq!(off.duration, paced.duration);
        // Limits: 0.02–1 s per unit.
        assert_eq!(pace_for_length(ms(100_000), 1, PerMille::ZERO), ms(1000));
        assert_eq!(pace_for_length(Tick::ZERO, 3, PerMille::new(500)), ms(20));
    }

    /// TX-022: on a strip shortened to fit, a join drag stores both slots
    /// so the dragged join lands where the pointer is and the other one
    /// stays where it was drawn, for either join.
    #[test]
    fn a_join_drag_resolves_a_shortened_strip() {
        let fade = slot_presets(&table(), Slot::Entrance, true)
            .into_iter()
            .find(|c| c.name == "Fade")
            .unwrap();
        // Each join away from the other segment.
        for (slot, x) in [(Slot::Entrance, 0.5), (Slot::Exit, 0.9)] {
            let (mut project, clip) = text_clip();
            let d = project.doc().clip(clip).unwrap().duration();
            let mut long = chosen_spec(&fade);
            long.duration = d;
            set(&mut project, clip, Slot::Entrance, Some(long));
            let mut out = chosen_spec(&fade);
            out.preset = PresetId::new("fc.out.fade");
            out.duration = ms(1000);
            set(&mut project, clip, Slot::Exit, Some(out));
            let c = project.doc().clip(clip).unwrap().clone();
            let before = strip_layout(&c).unwrap();
            assert!(before.shortened);
            for (s, spec) in join_drag(&c, &before, slot, x, rate()) {
                set(&mut project, clip, s, Some(spec));
            }
            let after = strip_layout(project.doc().clip(clip).unwrap()).unwrap();
            let frame = rate().frame_ticks().raw();
            assert!(!after.shortened, "{slot:?}");
            let (moved, kept, kept_before) = match slot {
                Slot::Exit => (after.out_start, after.in_len, before.in_len),
                _ => (after.in_end, after.out_len, before.out_len),
            };
            assert!((moved - x).abs() < 0.02, "{slot:?}: {after:?}");
            assert!((kept.raw() - kept_before.raw()).abs() <= frame, "{slot:?}");
        }
    }

    /// ui-concept §13.11 / TX-022: the strip's segments, and join drags
    /// snapped to frames, at least 0.1 s, stopping at the other slot.
    #[test]
    fn the_timing_strip_maps_segments_and_joins() {
        let (mut project, clip) = text_clip();
        let c = project.doc().clip(clip).unwrap().clone();
        let empty = strip_layout(&c).unwrap();
        assert_eq!(empty.policy, SlotPolicy::LoopBetween);
        assert_eq!(
            (empty.in_end, empty.out_start),
            (EMPTY_SHARE, 1.0 - EMPTY_SHARE)
        );
        assert!(empty.joins().is_empty());
        assert_eq!(empty.slot_at(0.05), Slot::Entrance);
        assert_eq!(empty.slot_at(0.5), Slot::Continuous);
        assert_eq!(empty.slot_at(0.95), Slot::Exit);

        let d = c.duration();
        let fade = slot_presets(&table(), Slot::Entrance, true)
            .into_iter()
            .find(|c| c.name == "Fade")
            .unwrap();
        set(&mut project, clip, Slot::Entrance, Some(chosen_spec(&fade)));
        let mut out = chosen_spec(&fade);
        out.preset = PresetId::new("fc.out.fade");
        out.duration = ms(1000);
        set(&mut project, clip, Slot::Exit, Some(out));
        let c = project.doc().clip(clip).unwrap().clone();
        let s = strip_layout(&c).unwrap();
        let share = |t: Tick| t.raw() as f64 / d.raw() as f64;
        assert!((s.in_end - share(ms(500))).abs() < 1e-12);
        assert!((s.out_start - (1.0 - share(ms(1000)))).abs() < 1e-12);
        assert_eq!(s.joins().len(), 2);
        assert!(!s.shortened);

        let r = rate();
        // Half way: snapped to a frame.
        let half = join_length(Slot::Entrance, 0.5, d, ms(1000), r);
        assert!(r.is_on_grid(half));
        assert!((half.raw() - d.raw() / 2).abs() <= r.frame_ticks().raw());
        // Never below 0.1 s, rounded up to a frame.
        let min = r.round_to_grid(ms(100), Rounding::Ceil).unwrap();
        assert_eq!(join_length(Slot::Entrance, 0.0, d, ms(1000), r), min);
        // Stops at the other slot's edge.
        let max = join_length(Slot::Entrance, 1.0, d, ms(1000), r);
        assert_eq!(max.raw() + ms(1000).raw(), d.raw());
        let out = join_length(Slot::Exit, 0.0, d, ms(500), r);
        assert_eq!(out.raw() + ms(500).raw(), d.raw());
        assert_eq!(join_length(Slot::Exit, 1.0, d, ms(500), r), min);

        // Shaded regions on the timeline: the effective In and Out.
        let regions = shaded_regions(&c, r.frame_ticks());
        assert_eq!(
            regions,
            [
                (
                    Slot::Entrance,
                    c.start(),
                    c.start().checked_add(ms(500)).unwrap()
                ),
                (Slot::Exit, c.end().checked_sub(ms(1000)).unwrap(), c.end()),
            ]
        );
    }
}
