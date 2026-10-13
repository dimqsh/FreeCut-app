//! Markers (TL-037): timeline markers in sequence time and clip markers in
//! the clip's content time (source time for media, clip-local content time
//! for generated items), plus the shared colour palette (TL-049, TL-045).

use std::sync::Arc;

use super::clip::Clip;
use super::sequence::Sequence;
use crate::ids::{ClipId, MarkerId};
use crate::time::{MarkRange, Tick, TimeRange};

/// FreeCut's 8-slot palette, shared by markers, track colours and clip
/// labels. The model stores the slot; each theme maps it to a shade.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PaletteColor {
    #[default]
    Blue,
    Teal,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
    Purple,
}

impl PaletteColor {
    /// Every slot, in palette order.
    pub const ALL: [PaletteColor; 8] = [
        PaletteColor::Blue,
        PaletteColor::Teal,
        PaletteColor::Green,
        PaletteColor::Yellow,
        PaletteColor::Orange,
        PaletteColor::Red,
        PaletteColor::Pink,
        PaletteColor::Purple,
    ];

    /// The slot after this one, wrapping (*Add marker with next colour*).
    pub fn next(self) -> PaletteColor {
        let i = PaletteColor::ALL
            .iter()
            .position(|c| *c == self)
            .unwrap_or(0);
        PaletteColor::ALL[(i + 1) % PaletteColor::ALL.len()]
    }

    /// A stable string id, for project files.
    pub const fn id(self) -> &'static str {
        match self {
            PaletteColor::Blue => "blue",
            PaletteColor::Teal => "teal",
            PaletteColor::Green => "green",
            PaletteColor::Yellow => "yellow",
            PaletteColor::Orange => "orange",
            PaletteColor::Red => "red",
            PaletteColor::Pink => "pink",
            PaletteColor::Purple => "purple",
        }
    }

    /// The slot with this id; `None` for an unknown one (a newer file).
    pub fn from_id(id: &str) -> Option<PaletteColor> {
        PaletteColor::ALL.into_iter().find(|c| c.id() == id)
    }
}

/// Where a marker came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MarkerOrigin {
    /// Placed by the user.
    #[default]
    User,
    /// A beat (AU-025): detected (`auto`) or tapped/placed by hand. Clip
    /// markers only.
    Beat { auto: bool },
    /// A detected scene change.
    Scene,
}

/// Longest marker name, in characters.
pub const MAX_MARKER_NAME_CHARS: usize = 120;
/// Longest marker note, in characters.
pub const MAX_MARKER_NOTE_CHARS: usize = 2_000;

/// One marker. `at` is in sequence time for timeline markers and in the
/// clip's content time for clip markers; a zero duration is a point.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Marker {
    pub id: MarkerId,
    pub at: MarkRange,
    pub name: String,
    pub note: String,
    pub color: PaletteColor,
    pub origin: MarkerOrigin,
}

impl Marker {
    /// A user point marker with default name, note and colour.
    pub fn new(id: MarkerId, at: Tick) -> Marker {
        Marker {
            id,
            at: MarkRange::point(at),
            name: String::new(),
            note: String::new(),
            color: PaletteColor::default(),
            origin: MarkerOrigin::User,
        }
    }

    /// Whether the name and note fit their limits and the origin fits the
    /// owner (`Beat` is for clip markers only).
    pub fn is_valid(&self, on_clip: bool) -> bool {
        self.name.chars().count() <= MAX_MARKER_NAME_CHARS
            && self.note.chars().count() <= MAX_MARKER_NOTE_CHARS
            && (on_clip || !matches!(self.origin, MarkerOrigin::Beat { .. }))
    }
}

/// A marker list sorted by `(at.start, id)`. Shared: clips and sequences are
/// cloned far more often than their markers change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Markers(Option<Arc<[Marker]>>);

impl Markers {
    /// Sorts `markers` by `(at.start, id)`.
    pub fn new(mut markers: Vec<Marker>) -> Markers {
        markers.sort_by_key(|m| (m.at.start(), m.id));
        Markers((!markers.is_empty()).then(|| Arc::from(markers)))
    }

    pub fn as_slice(&self) -> &[Marker] {
        self.0.as_deref().unwrap_or(&[])
    }

    pub fn iter(&self) -> core::slice::Iter<'_, Marker> {
        self.as_slice().iter()
    }

    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub fn get(&self, id: MarkerId) -> Option<&Marker> {
        self.iter().find(|m| m.id == id)
    }

    /// Sorted by `(at.start, id)` with no id twice.
    pub fn is_sorted(&self) -> bool {
        self.as_slice()
            .windows(2)
            .all(|w| (w[0].at.start(), w[0].id) < (w[1].at.start(), w[1].id))
    }

    /// The list with `marker` added, or replacing the marker with its id.
    pub(crate) fn with(&self, marker: Marker) -> Markers {
        let mut v: Vec<Marker> = self.iter().filter(|m| m.id != marker.id).cloned().collect();
        v.push(marker);
        Markers::new(v)
    }

    /// The list without the markers `drop` picks.
    pub(crate) fn without(&self, drop: impl Fn(&Marker) -> bool) -> Markers {
        Markers::new(self.iter().filter(|m| !drop(m)).cloned().collect())
    }

    /// The name of a new marker: "Marker N", one more than the highest N
    /// among this owner's "Marker N" names (TL-037).
    pub fn next_name(&self) -> String {
        let highest = self
            .iter()
            .filter_map(|m| m.name.strip_prefix("Marker "))
            .filter_map(|n| n.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        format!("Marker {}", highest.saturating_add(1))
    }

    /// Approximate heap size, for the history's memory budget.
    pub(crate) fn heap_bytes(&self) -> usize {
        self.iter()
            .map(|m| core::mem::size_of::<Marker>() + m.name.len() + m.note.len())
            .sum()
    }
}

impl<'a> IntoIterator for &'a Markers {
    type Item = &'a Marker;
    type IntoIter = core::slice::Iter<'a, Marker>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Addresses one marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MarkerRef {
    /// A timeline marker of the sequence.
    Timeline(MarkerId),
    /// A marker on a clip.
    Clip(ClipId, MarkerId),
}

impl MarkerRef {
    pub const fn id(self) -> MarkerId {
        match self {
            MarkerRef::Timeline(id) | MarkerRef::Clip(_, id) => id,
        }
    }
}

/// Which markers a query counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MarkerScope {
    /// Timeline markers only.
    TimelineOnly,
    /// Timeline markers and the visible clip markers of clips on tracks
    /// that aren't hidden (TL-037's navigation default).
    #[default]
    VisibleTracks,
    /// Timeline markers and the visible clip markers of every clip.
    AllTracks,
}

impl Clip {
    /// The span of content time the clip shows: its source range for media,
    /// else `[origin, origin + duration)` of its content time (TR-027).
    pub fn content_range(&self) -> Option<TimeRange> {
        match self.content.source() {
            Some(source) => Some(source),
            None => TimeRange::new(self.keys.origin(), self.duration()).ok(),
        }
    }

    /// Where clip marker `m` starts in sequence time, if the clip shows it
    /// (markers trimmed out of view are kept but hidden, TL-037).
    pub fn marker_start(&self, m: &Marker) -> Option<Tick> {
        let shown = self.content_range()?.contains(m.at.start());
        shown.then(|| self.sequence_time_of(m.at.start()))?
    }
}

/// Every marker start and range end of `seq` in sequence time, unsorted:
/// timeline markers, then the visible markers of clips in `scope`, skipping
/// clips `skip` picks. Range ends of clip markers are clipped to the clip.
pub(crate) fn marker_points(
    seq: &Sequence,
    scope: MarkerScope,
    skip: impl Fn(ClipId) -> bool,
) -> Vec<(Tick, bool)> {
    let mut out: Vec<(Tick, bool)> = Vec::new();
    for m in &seq.markers {
        out.push((m.at.start(), true));
        if !m.at.is_point() {
            out.push((m.at.end(), false));
        }
    }
    if scope == MarkerScope::TimelineOnly {
        return out;
    }
    for (_, track) in seq.tracks() {
        if scope == MarkerScope::VisibleTracks && track.header.flags.hidden {
            continue;
        }
        for clip in track.clips() {
            if clip.markers.is_empty() || skip(clip.id) {
                continue;
            }
            for m in &clip.markers {
                let Some(start) = clip.marker_start(m) else {
                    continue;
                };
                out.push((start, true));
                if !m.at.is_point()
                    && let Some(end) = clip.sequence_time_of(m.at.end())
                {
                    out.push((end.min(clip.end()), false));
                }
            }
        }
    }
    out
}

/// The nearest marker start strictly after `t` among the markers of
/// `scope`; `None` past the last one (no wrap-around, TL-037).
pub fn next_marker(seq: &Sequence, t: Tick, scope: MarkerScope) -> Option<Tick> {
    marker_points(seq, scope, |_| false)
        .into_iter()
        .filter(|&(p, is_start)| is_start && p > t)
        .map(|(p, _)| p)
        .min()
}

/// The nearest marker start strictly before `t`; `None` before the first.
pub fn prev_marker(seq: &Sequence, t: Tick, scope: MarkerScope) -> Option<Tick> {
    marker_points(seq, scope, |_| false)
        .into_iter()
        .filter(|&(p, is_start)| is_start && p < t)
        .map(|(p, _)| p)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::NonZeroU64;

    fn mid(n: u64) -> MarkerId {
        MarkerId::from_raw(NonZeroU64::new(n).unwrap())
    }

    #[test]
    fn next_colour_walks_the_palette_and_wraps() {
        assert_eq!(PaletteColor::default(), PaletteColor::Blue);
        assert_eq!(PaletteColor::Blue.next(), PaletteColor::Teal);
        assert_eq!(PaletteColor::Purple.next(), PaletteColor::Blue);
        for c in PaletteColor::ALL {
            assert_eq!(PaletteColor::from_id(c.id()), Some(c));
        }
        assert_eq!(PaletteColor::from_id("chartreuse"), None);
    }

    #[test]
    fn names_count_up_from_the_highest_used_number() {
        let mut list = Markers::default();
        assert_eq!(list.next_name(), "Marker 1");
        for (n, name) in [(1, "Marker 1"), (2, "Marker 7"), (3, "Intro")] {
            let mut m = Marker::new(mid(n), Tick::from_raw(n as i64));
            m.name = name.into();
            list = list.with(m);
        }
        assert_eq!(list.next_name(), "Marker 8");
        let list = list.without(|m| m.name == "Marker 1");
        assert_eq!(list.next_name(), "Marker 8", "gaps are not reused");
    }

    #[test]
    fn lists_stay_sorted_by_start_then_id() {
        let list = Markers::new(vec![
            Marker::new(mid(3), Tick::from_raw(5)),
            Marker::new(mid(1), Tick::from_raw(9)),
            Marker::new(mid(2), Tick::from_raw(5)),
        ]);
        let ids: Vec<u64> = list.iter().map(|m| m.id.get()).collect();
        assert_eq!(ids, vec![2, 3, 1]);
        assert!(list.is_sorted());
    }
}
