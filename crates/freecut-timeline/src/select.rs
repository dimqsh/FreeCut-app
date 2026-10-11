//! Selection helpers (TL-015, TL-039, TL-057). Pure: the app keeps the
//! selection and passes the expanded set to the existing ops, so no op
//! changes because of linked selection.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::edit::Edge;
use crate::ids::{ClipId, GroupId, LinkId};
use crate::model::{Clip, Sequence};

/// What a picked clip pulls into the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Expand {
    /// Members of the clip's group (TL-039).
    pub groups: bool,
    /// Link partners: a video and its detached sound or the streams added
    /// from it (TL-057's *Select linked clips together*).
    pub links: bool,
}

/// `picked` plus everything [`Expand`] pulls in, repeated until nothing new
/// is added (partners' groups, groups' partners, …). Partners and members
/// on locked tracks are skipped (TL-015); picked clips are kept as they are.
/// Every link today is a detached-sound link; speech links (TX-043), when
/// they exist, must not expand.
pub fn expand_selection(
    seq: &Sequence,
    picked: &BTreeSet<ClipId>,
    rules: Expand,
) -> BTreeSet<ClipId> {
    let mut out = picked.clone();
    if !rules.groups && !rules.links {
        return out;
    }
    let mut groups: BTreeMap<GroupId, Vec<ClipId>> = BTreeMap::new();
    let mut links: BTreeMap<LinkId, Vec<ClipId>> = BTreeMap::new();
    let mut info: BTreeMap<ClipId, (Option<GroupId>, Option<LinkId>)> = BTreeMap::new();
    for (_, track) in seq.tracks() {
        let locked = track.is_locked();
        for clip in track.clips() {
            if clip.group.is_none() && clip.link.is_none() {
                continue;
            }
            info.insert(clip.id, (clip.group, clip.link));
            if locked {
                continue;
            }
            if let Some(g) = clip.group {
                groups.entry(g).or_default().push(clip.id);
            }
            if let Some(l) = clip.link {
                links.entry(l).or_default().push(clip.id);
            }
        }
    }
    let mut queue: Vec<ClipId> = out.iter().copied().collect();
    while let Some(id) = queue.pop() {
        let Some(&(group, link)) = info.get(&id) else {
            continue;
        };
        let mut pull = |members: Option<&Vec<ClipId>>| {
            for m in members.into_iter().flatten() {
                if out.insert(*m) {
                    queue.push(*m);
                }
            }
        };
        if rules.groups {
            pull(group.and_then(|g| groups.get(&g)));
        }
        if rules.links {
            pull(link.and_then(|l| links.get(&l)));
        }
    }
    out
}

/// The link partners of `clip` (on unlocked tracks) that trim with it when
/// `edge` is dragged: their same edge is at the same time and they are in
/// sync with it (TL-057, TL-051). In track order, then by start.
pub fn linked_edges(seq: &Sequence, clip: ClipId, edge: Edge) -> Vec<ClipId> {
    let Some(this) = find(seq, clip) else {
        return Vec::new();
    };
    let Some(link) = this.link else {
        return Vec::new();
    };
    let at = |c: &Clip| match edge {
        Edge::In => c.start(),
        Edge::Out => c.end(),
    };
    seq.tracks()
        .filter(|(_, t)| !t.is_locked())
        .flat_map(|(_, t)| t.clips().iter())
        .filter(|c| c.id != clip && c.link == Some(link))
        .filter(|c| at(c) == at(&this) && in_sync(c, &this))
        .map(|c| c.id)
        .collect()
}

fn find(seq: &Sequence, id: ClipId) -> Option<Arc<Clip>> {
    seq.find_clip(id).and_then(|(_, t)| t.clip(id).cloned())
}

/// Same source time under the same sequence time (TL-030's sync rule);
/// clips without a source range compare by start.
fn in_sync(a: &Clip, b: &Clip) -> bool {
    match (a.content.source(), b.content.source()) {
        (Some(sa), Some(sb)) => {
            i128::from(a.start().raw()) - i128::from(sa.start().raw())
                == i128::from(b.start().raw()) - i128::from(sb.start().raw())
        }
        _ => a.start() == b.start(),
    }
}
