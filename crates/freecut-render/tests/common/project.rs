//! Documents built through the timeline's editor (plan and commit, as the
//! app does), for tests that render real plans.

use std::collections::BTreeSet;
use std::sync::Arc;

use freecut_render::cpu::Frame;
use freecut_timeline::edit::{
    EditCtx, EditRequest, InsertReq, InsertTarget, KeyEdit, NewClip, NewMedia, TextLayoutEdit,
    TextStyleEdit, TransformEdit,
};
use freecut_timeline::history::HistoryLimits;
use freecut_timeline::ids::{ClipId, MediaId, SeqId};
use freecut_timeline::model::{
    AnimationSpec, Document, MediaInfo, MediaKind, ProjectSettings, PropPath, PropValue, RichText,
    Slot, StoredPath, TextClipData,
};
use freecut_timeline::plan::{FramePlan, PlanInputs, frame_plan};
use freecut_timeline::time::{FrameRate, Tick, TimeRange};
use freecut_timeline::{CommitMeta, Committed, Editor};

pub struct Project {
    pub editor: Editor,
}

impl Project {
    /// An empty project with a `width × height` canvas at `fps`.
    pub fn new(width: u32, height: u32, fps: u32) -> Project {
        let settings = ProjectSettings {
            width,
            height,
            frame_rate: FrameRate::new(fps, 1).unwrap(),
            ..ProjectSettings::new_default().unwrap()
        };
        Project {
            editor: Editor::with_settings(settings, HistoryLimits::default()).unwrap(),
        }
    }

    pub fn doc(&self) -> Arc<Document> {
        Arc::new(self.editor.document().clone())
    }

    pub fn seq(&self) -> SeqId {
        self.editor.document().root()
    }

    /// Plans and commits `req`; returns the selection it leaves.
    pub fn apply(&mut self, req: EditRequest) -> Vec<ClipId> {
        let plan = self.editor.plan(&EditCtx::default(), &req).unwrap();
        match self.editor.commit(plan, CommitMeta::default()).unwrap() {
            Committed::Applied { selection, .. } => selection.unwrap_or_default(),
            Committed::NoOp => panic!("{req:?} changed nothing"),
        }
    }

    /// Inserts a text clip with `data` on the first free text track.
    pub fn add_text(&mut self, data: TextClipData, at: Tick, duration: Tick) -> ClipId {
        let seq = self.seq();
        self.apply(EditRequest::Insert(InsertReq {
            seq,
            target: InsertTarget::FirstFree,
            at,
            clips: vec![NewClip::text(data, duration)],
        }))[0]
    }

    pub fn set_content(&mut self, clip: ClipId, text: &str) {
        let seq = self.seq();
        self.apply(EditRequest::SetTextContent {
            seq,
            clip,
            content: RichText::new(text),
        });
    }

    pub fn style(&mut self, clip: ClipId, edit: TextStyleEdit) {
        let seq = self.seq();
        self.apply(EditRequest::SetTextStyle {
            seq,
            clips: vec![clip],
            edit,
        });
    }

    pub fn layout(&mut self, clip: ClipId, edit: TextLayoutEdit) {
        let seq = self.seq();
        self.apply(EditRequest::SetTextLayout {
            seq,
            clips: vec![clip],
            edit,
        });
    }

    pub fn transform(&mut self, clip: ClipId, edit: TransformEdit) {
        let seq = self.seq();
        self.apply(EditRequest::SetTransform {
            seq,
            clips: vec![clip],
            edit,
        });
    }

    /// Sets `slot`'s animation on `clip` (`SetClipAnimation`).
    pub fn animate(&mut self, clip: ClipId, slot: Slot, spec: Option<AnimationSpec>) {
        let seq = self.seq();
        self.apply(EditRequest::SetClipAnimation {
            seq,
            clips: vec![clip],
            slot,
            spec,
        });
    }

    /// Adds a key of `prop` on `clip` at sequence time `at` (`EditKeys`).
    pub fn key(&mut self, clip: ClipId, prop: PropPath, at: Tick, value: PropValue) {
        let seq = self.seq();
        self.apply(EditRequest::EditKeys {
            seq,
            clips: vec![clip],
            prop,
            edit: KeyEdit::Add {
                at,
                value: Some(value),
            },
        });
    }

    /// Imports a probed file and returns its library id.
    pub fn import(&mut self, path: &str, info: MediaInfo) -> MediaId {
        let req = EditRequest::ImportMedia(vec![NewMedia {
            path: StoredPath {
                absolute: path.into(),
                relative: None,
            },
            info,
            fingerprint: None,
            display_name: None,
        }]);
        let plan = self.editor.plan(&EditCtx::default(), &req).unwrap();
        let id = plan.imported()[0];
        self.editor.commit(plan, CommitMeta::default()).unwrap();
        id
    }

    /// Places `source` of video `media` on `target` at `at`.
    pub fn place_video(
        &mut self,
        target: InsertTarget,
        at: Tick,
        media: MediaId,
        source: TimeRange,
    ) -> ClipId {
        let seq = self.seq();
        self.apply(EditRequest::Insert(InsertReq {
            seq,
            target,
            at,
            clips: vec![NewClip::media(media, MediaKind::Video, source)],
        }))[0]
    }

    /// The plan of the frame at `t`, nothing offline.
    pub fn plan_at(&self, t: Tick) -> FramePlan {
        let offline = BTreeSet::new();
        frame_plan(
            self.editor.document(),
            self.seq(),
            t,
            &PlanInputs { offline: &offline },
        )
        .unwrap()
    }
}

/// The bounding box `(min_x, min_y, max_x, max_y)` (inclusive) of every
/// pixel that differs from `bg`.
pub fn ink_box(frame: &Frame, bg: [u8; 4]) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in 0..frame.height {
        for x in 0..frame.width {
            if frame.pixel(x, y) != Some(bg) {
                found = Some(match found {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    found
}

/// The centre of an inclusive pixel box.
pub fn centre(b: (u32, u32, u32, u32)) -> (f64, f64) {
    (
        f64::from(b.0 + b.2 + 1) / 2.0,
        f64::from(b.1 + b.3 + 1) / 2.0,
    )
}
