//! The timeline's pointer tools (TL-024) and the pure helpers behind the
//! blade and the marker flags: which tool is active, where a blade click
//! cuts, and which flag a press hits. No painting and no window: the
//! timeline panel draws and routes input, these decide.

use freecut_timeline::ids::ClipId;
use freecut_timeline::model::Sequence;
use freecut_timeline::snap::SnapIndex;
use freecut_timeline::time::{FrameRate, Tick};

use crate::theme::icons;

/// What a press in the lanes does (TL-024). View state: every project
/// opens with Select.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TimelineTool {
    /// Select, move and trim clips.
    #[default]
    Select,
    /// Click a clip to cut it (`Shift`: every unlocked track).
    Blade,
    /// Drag to pan the view.
    Hand,
}

impl TimelineTool {
    pub const ALL: [TimelineTool; 3] = [
        TimelineTool::Select,
        TimelineTool::Blade,
        TimelineTool::Hand,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TimelineTool::Select => "Select",
            TimelineTool::Blade => "Blade",
            TimelineTool::Hand => "Hand",
        }
    }

    /// The tool's key (UI-031).
    pub fn key(self) -> &'static str {
        match self {
            TimelineTool::Select => "A",
            TimelineTool::Blade => "B",
            TimelineTool::Hand => "H",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            TimelineTool::Select => icons::CURSOR,
            TimelineTool::Blade => icons::BLADE,
            TimelineTool::Hand => icons::HAND,
        }
    }
}

/// The active tool and the last non-select one, which the tool button's
/// click switches back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolState {
    pub active: TimelineTool,
    pub last_other: TimelineTool,
}

impl Default for ToolState {
    fn default() -> Self {
        ToolState {
            active: TimelineTool::Select,
            last_other: TimelineTool::Blade,
        }
    }
}

impl ToolState {
    /// Picks `tool` (a key or the chevron's menu).
    pub fn set(&mut self, tool: TimelineTool) {
        self.active = tool;
        if tool != TimelineTool::Select {
            self.last_other = tool;
        }
    }

    /// The tool button's click: Select ↔ the last other tool.
    pub fn toggle(&mut self) {
        if self.active == TimelineTool::Select {
            self.active = self.last_other;
        } else {
            self.active = TimelineTool::Select;
        }
    }

    /// The tool button's tooltip: the active tool and its key.
    pub fn tooltip(self) -> String {
        let other = if self.active == TimelineTool::Select {
            self.last_other
        } else {
            TimelineTool::Select
        };
        format!(
            "{} tool ({}). Click for the {} tool",
            self.active.label(),
            self.active.key(),
            other.label().to_lowercase()
        )
    }
}

/// Where a blade click at `pointer` on `clip` cuts (TL-024): the pointer
/// time, snapped when `snap` is given, quantized to a frame, strictly
/// inside the clip. `None` on a clip edge, outside the clip, on a locked
/// track or for a clip that isn't in `seq`.
pub fn blade_time(
    seq: &Sequence,
    clip: ClipId,
    pointer: Tick,
    snap: Option<(&SnapIndex, u64)>,
    rate: FrameRate,
) -> Option<Tick> {
    let (_, track) = seq.find_clip(clip)?;
    if track.is_locked() {
        return None;
    }
    let c = track.clips().iter().find(|c| c.id == clip)?;
    let mut t = pointer;
    if let Some((index, threshold)) = snap
        && let Some(s) = index.snap(&[pointer], Tick::ZERO, threshold)
        && let Ok(snapped) = pointer.checked_add(s.delta)
    {
        t = snapped;
    }
    let t = rate.quantize(t).ok()?;
    (c.start() < t && t < c.end()).then_some(t)
}

/// The flag of a marker as drawn: its x and, for a range, its end's x.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlagSpot {
    pub x: f32,
    pub end_x: Option<f32>,
}

/// Width of a timeline marker's flag (TL-037: 9 pt).
pub const FLAG_W: f32 = 9.0;
/// How far from a range's end a press grabs the end (TL-037's "drag a
/// marker's right side").
pub const RANGE_END_GRAB: f32 = 4.0;

/// What a press near a flag grabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagGrab {
    /// The flag: moving the marker.
    Start(usize),
    /// A range's end: changing its duration.
    End(usize),
}

/// Which of `flags` a press at `x` hits: a flag spans `[x - 1, x + 9]`; a
/// range's end grabs within 4 pt. The topmost (last drawn) wins.
pub fn flag_hit(flags: &[FlagSpot], x: f32) -> Option<FlagGrab> {
    for (i, f) in flags.iter().enumerate().rev() {
        if let Some(end) = f.end_x
            && (x - end).abs() <= RANGE_END_GRAB
            && end - f.x > FLAG_W + RANGE_END_GRAB
        {
            return Some(FlagGrab::End(i));
        }
        if x >= f.x - 1.0 && x <= f.x + FLAG_W {
            return Some(FlagGrab::Start(i));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::collections::BTreeSet;

    use freecut_timeline::edit::EditRequest;
    use freecut_timeline::history::Coalesce;
    use freecut_timeline::model::{TrackFlags, TrackHeader};

    use super::*;
    use crate::timeline_commands::tests::{s, talking_head};

    #[test]
    fn the_tool_button_toggles_select_and_the_last_other_tool() {
        let mut t = ToolState::default();
        assert_eq!(t.active, TimelineTool::Select);
        t.toggle();
        assert_eq!(t.active, TimelineTool::Blade, "Blade before any other");
        t.toggle();
        assert_eq!(t.active, TimelineTool::Select);
        t.set(TimelineTool::Hand);
        t.set(TimelineTool::Select);
        t.toggle();
        assert_eq!(t.active, TimelineTool::Hand, "the last other tool");
        assert_eq!(t.tooltip(), "Hand tool (H). Click for the select tool");
        t.toggle();
        assert_eq!(t.tooltip(), "Select tool (A). Click for the hand tool");
    }

    #[test]
    fn the_blade_cuts_at_the_snapped_frame_and_never_on_edges_gaps_or_locks() {
        let (mut p, [a, b, title]) = talking_head();
        let doc = p.doc().clone();
        let seq = doc.sequence(doc.root()).unwrap();
        let rate = doc.settings().frame_rate;
        let frame = rate.frame_ticks();
        // A pointer between frames lands on a frame.
        let off = Tick::from_raw(s(2).raw() + frame.raw() / 3);
        assert_eq!(blade_time(seq, a, off, None, rate), Some(s(2)));
        // Snapping pulls it to the playhead within the threshold.
        let index = SnapIndex::build(seq, Some(s(3)), &BTreeSet::new());
        let near = Tick::from_raw(s(3).raw() - 2 * frame.raw());
        let threshold = (3 * frame.raw()) as u64;
        assert_eq!(
            blade_time(seq, a, near, Some((&index, threshold)), rate),
            Some(s(3))
        );
        // On an edge (snapped there, or exactly), or past the clip: nothing.
        assert_eq!(blade_time(seq, a, s(4), None, rate), None);
        let by_edge = Tick::from_raw(s(4).raw() - frame.raw());
        assert_eq!(
            blade_time(seq, b, by_edge, Some((&index, threshold)), rate),
            None
        );
        assert_eq!(blade_time(seq, title, s(5), None, rate), None);
        // A locked track: nothing.
        let (_, track) = seq.find_clip(title).unwrap();
        let header = TrackHeader {
            name: None,
            flags: TrackFlags {
                locked: true,
                ..track.header.flags
            },
        };
        let req = EditRequest::SetTrackHeader {
            seq: doc.root(),
            track: track.id,
            header,
        };
        p.apply(&req, Coalesce::Never).unwrap();
        let doc = p.doc().clone();
        let seq = doc.sequence(doc.root()).unwrap();
        assert_eq!(blade_time(seq, title, s(2), None, rate), None);
    }

    #[test]
    fn flags_and_range_ends_take_presses() {
        let flags = [
            FlagSpot {
                x: 100.0,
                end_x: None,
            },
            FlagSpot {
                x: 200.0,
                end_x: Some(260.0),
            },
        ];
        assert_eq!(flag_hit(&flags, 104.0), Some(FlagGrab::Start(0)));
        assert_eq!(flag_hit(&flags, 99.5), Some(FlagGrab::Start(0)));
        assert_eq!(flag_hit(&flags, 120.0), None);
        assert_eq!(flag_hit(&flags, 203.0), Some(FlagGrab::Start(1)));
        assert_eq!(flag_hit(&flags, 258.0), Some(FlagGrab::End(1)));
        assert_eq!(flag_hit(&flags, 230.0), None, "the bar itself is inert");
        // A short range: the flag wins over its end.
        let short = [FlagSpot {
            x: 10.0,
            end_x: Some(18.0),
        }];
        assert_eq!(flag_hit(&short, 16.0), Some(FlagGrab::Start(0)));
    }
}
