//! Snapping (TL-028).
//!
//! A [`SnapIndex`] is built once per gesture and queried on every pointer
//! move. The nearest target wins; ties go by kind priority, then to the
//! earlier reference edge, so results are deterministic.

use std::collections::BTreeSet;

use crate::ids::ClipId;
use crate::model::{MarkerRef, MarkerScope, Sequence, marker_points_except};
use crate::time::Tick;

/// What a snap target is. Declaration order is the tie-break priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SnapKind {
    Playhead,
    Marker,
    ClipEdge,
    /// The sequence start and end.
    Bound,
}

/// The result of a snap: the adjusted delta and where to draw guides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snap {
    pub delta: Tick,
    pub guides: Vec<Tick>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapIndex {
    /// Sorted by time, one entry per time (highest-priority kind kept).
    points: Vec<(Tick, SnapKind)>,
}

impl SnapIndex {
    /// Targets from `seq`: clip edges on every track (hidden and locked clips
    /// included) except `exclude`, every marker start and range end (timeline
    /// markers and the shown markers of clips not in `exclude`, TL-037), plus
    /// the playhead, 0 and the end.
    pub fn build(seq: &Sequence, playhead: Option<Tick>, exclude: &BTreeSet<ClipId>) -> SnapIndex {
        SnapIndex::build_except(seq, playhead, exclude, None)
    }

    /// [`SnapIndex::build`] without the start and end of marker `dragged`,
    /// for dragging it (TL-028 leaves the dragged items out).
    pub fn build_for_marker(
        seq: &Sequence,
        playhead: Option<Tick>,
        dragged: MarkerRef,
    ) -> SnapIndex {
        SnapIndex::build_except(seq, playhead, &BTreeSet::new(), Some(dragged))
    }

    fn build_except(
        seq: &Sequence,
        playhead: Option<Tick>,
        exclude: &BTreeSet<ClipId>,
        marker: Option<MarkerRef>,
    ) -> SnapIndex {
        let mut points = Vec::new();
        let mut end = Tick::ZERO;
        for (_, track) in seq.tracks() {
            for clip in track.clips().iter().filter(|c| !exclude.contains(&c.id)) {
                points.push((clip.start(), SnapKind::ClipEdge));
                points.push((clip.end(), SnapKind::ClipEdge));
                end = end.max(clip.end());
            }
        }
        points.extend(
            marker_points_except(
                seq,
                MarkerScope::AllTracks,
                |id| exclude.contains(&id),
                marker,
            )
            .into_iter()
            .map(|(t, _)| (t, SnapKind::Marker)),
        );
        points.push((Tick::ZERO, SnapKind::Bound));
        points.push((end, SnapKind::Bound));
        if let Some(p) = playhead {
            points.push((p, SnapKind::Playhead));
        }
        SnapIndex::from_points(points)
    }

    pub fn from_points(mut points: Vec<(Tick, SnapKind)>) -> SnapIndex {
        points.sort();
        points.dedup_by_key(|(t, _)| *t);
        SnapIndex { points }
    }

    fn contains(&self, t: Tick) -> bool {
        self.points.binary_search_by_key(&t, |(p, _)| *p).is_ok()
    }

    /// Snaps a block whose reference times are `edges` (block start and end
    /// for moves, one edge for trims, one time for the playhead) after moving
    /// it by `delta`. `threshold` is inclusive, in ticks.
    ///
    /// A moved edge never snaps back to where it started: a target there
    /// (a magnetic neighbour that moves with the ripple, or the dragged
    /// item itself) would only make small drags stick.
    pub fn snap(&self, edges: &[Tick], delta: Tick, threshold: u64) -> Option<Snap> {
        let mut best: Option<(u64, SnapKind, usize, Tick)> = None;
        for (ei, &edge) in edges.iter().enumerate() {
            let Ok(want) = edge.checked_add(delta) else {
                continue;
            };
            let i = self.points.partition_point(|&(t, _)| t < want);
            let lo = i.saturating_sub(2);
            let hi = (i + 2).min(self.points.len());
            for &(t, kind) in &self.points[lo..hi] {
                if t == edge && delta != Tick::ZERO {
                    continue;
                }
                let dist = t.abs_diff(want);
                if dist <= threshold {
                    let candidate = (dist, kind, ei, t);
                    if best.is_none_or(|b| candidate < b) {
                        best = Some(candidate);
                    }
                }
            }
        }
        let (_, _, ei, target) = best?;
        let delta = target.checked_sub(edges[ei]).ok()?;
        let mut guides: Vec<Tick> = edges
            .iter()
            .filter_map(|e| e.checked_add(delta).ok())
            .filter(|t| self.contains(*t))
            .collect();
        guides.sort();
        guides.dedup();
        Some(Snap { delta, guides })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(v: i64) -> Tick {
        Tick::from_raw(v)
    }

    fn index() -> SnapIndex {
        SnapIndex::from_points(vec![
            (t(0), SnapKind::Bound),
            (t(100), SnapKind::ClipEdge),
            (t(105), SnapKind::Playhead),
            (t(200), SnapKind::ClipEdge),
        ])
    }

    #[test]
    fn nearest_target_wins() {
        // 90 + 12 = 102: the clip edge at 100 is 2 away, the playhead 3.
        let snap = index().snap(&[t(90)], t(12), 10).unwrap();
        assert_eq!(snap.delta, t(10));
        assert_eq!(snap.guides, vec![t(100)]);
    }

    #[test]
    fn ties_go_to_the_higher_priority_kind() {
        let idx = SnapIndex::from_points(vec![
            (t(98), SnapKind::ClipEdge),
            (t(102), SnapKind::Playhead),
        ]);
        let snap = idx.snap(&[t(100)], t(0), 5).unwrap();
        assert_eq!(snap.delta, t(2));
    }

    #[test]
    fn threshold_is_inclusive_and_misses_return_none() {
        assert!(index().snap(&[t(150)], t(0), 10).is_none());
        assert_eq!(index().snap(&[t(190)], t(0), 10).unwrap().delta, t(10));
    }

    #[test]
    fn a_moved_edge_never_snaps_back_to_where_it_started() {
        let idx = SnapIndex::from_points(vec![(t(10), SnapKind::ClipEdge)]);
        assert_eq!(idx.snap(&[t(10)], t(2), 4), None);
        assert_eq!(idx.snap(&[t(10)], t(-3), 4), None);
        // The next target over still catches it.
        let idx =
            SnapIndex::from_points(vec![(t(10), SnapKind::ClipEdge), (t(13), SnapKind::Marker)]);
        assert_eq!(idx.snap(&[t(10)], t(2), 4).unwrap().delta, t(3));
        // Unmoved, the edge sits on its own target.
        assert_eq!(idx.snap(&[t(10)], t(0), 4).unwrap().delta, t(0));
    }

    #[test]
    fn either_edge_of_a_block_can_snap_and_guides_cover_both() {
        // Block [90, 190) moved by +11 -> [101, 201): end is 1 from 200,
        // start is 1 from 100; tie goes to the earlier edge.
        let snap = index().snap(&[t(90), t(190)], t(11), 5).unwrap();
        assert_eq!(snap.delta, t(10));
        assert_eq!(snap.guides, vec![t(100), t(200)]);
    }
}
