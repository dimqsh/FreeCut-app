//! The Text tab of the asset panel (TX-001, UI-004, UI-005; ui-concept
//! §8.2): the Drawer's grid with an **Add text** tile. A click (or the
//! tile's hover `+`) adds a text clip at the playhead; dragging the tile
//! onto the timeline drops one at that track and time.
//!
//! The tile shows the factory default style as a sample (white, bold) on
//! the text clip's well; the clip itself is drawn by `freecut_render` like
//! every other clip. Saved looks, looks, templates and captions (TX-019,
//! TX-020, TX-024, TX-031) aren't built yet, so the side list has only
//! *All*.

use eframe::egui::{self, Color32, FontId, Id, Sense, UiBuilder, Vec2};
use freecut_timeline::edit::{EditRequest, InsertReq, InsertTarget, NewClip};
use freecut_timeline::history::Coalesce;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{TextClipData, default_text_duration};
use freecut_timeline::time::Tick;

use super::assets::{self, AssetDrag, Group};
use crate::session::Project;
use crate::theme::{self, surface};
use crate::widgets::tile::{
    Corner, Picture, TileSpec, cell_rect, corner_button_rect, grid_layout, paint_tile, thumb_rect,
};

#[derive(Debug, Default)]
pub struct TextPanelOutput {
    /// The tile was clicked: add a text clip at the playhead.
    pub add_clicked: bool,
}

/// The Text Drawer's side list: only *All* until looks and templates exist.
pub fn groups() -> Vec<Group> {
    vec![Group {
        title: "Text",
        rows: vec![("All", 1)],
    }]
}

pub fn show(ui: &mut egui::Ui, chips: Option<&[Group]>) -> TextPanelOutput {
    let mut out = TextPanelOutput::default();
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let gap = ui.spacing().item_spacing.y.max(6.0);
    let h = look.metrics.control_height;
    // The toolbar row: a hint where Media has its Import button.
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new("Click to add at the playhead, or drag to the timeline")
                        .text_style(egui::TextStyle::Small)
                        .color(p.text_tertiary),
                )
                .wrap(),
            );
        },
    );
    if let Some(groups) = chips {
        ui.add_space(gap);
        let _ = assets::chip_row(ui, Id::new("text-chips"), groups, 0);
    }
    ui.add_space(gap);

    let compact = look.metrics.control_height < 28.0;
    let (min, tile_gap) = if compact { (80.0, 6.0) } else { (96.0, 8.0) };
    let (cols, tile_w) = grid_layout(ui.available_width(), min, tile_gap);
    let cell = cell_rect(ui.cursor().min, 0, cols, tile_w, tile_gap);
    let drag_id = Id::new("add-text-tile");
    let dragged = ui.ctx().is_being_dragged(drag_id);
    let mut spec = TileSpec {
        name: "Add text",
        mark: surface::CLIP_TEXT,
        picture: Picture::Custom,
        badge: None,
        tag: None,
        offline: false,
        selected: false,
        hovered: false,
        corner: None,
    };
    if dragged {
        let mut faded = ui.painter().clone();
        faded.multiply_opacity(0.45);
        paint_tile(ui, &faded, cell, &spec);
        sample(&faded, thumb_rect(cell));
    }
    let mut plus = false;
    let response = ui
        .scope_builder(UiBuilder::new().max_rect(cell), |ui| {
            ui.dnd_drag_source(drag_id, AssetDrag::Text, |ui| {
                let (rect, resp) = ui.allocate_exact_size(cell.size(), Sense::click());
                let hovered = !dragged && ui.rect_contains_pointer(rect);
                if hovered {
                    let r = corner_button_rect(thumb_rect(rect));
                    let button = ui
                        .interact(r, drag_id.with("add"), Sense::click())
                        .on_hover_text("Add at the playhead");
                    plus = button.clicked();
                    spec.corner = Some(Corner::Add {
                        hot: button.hovered(),
                    });
                }
                spec.hovered = hovered;
                paint_tile(ui, ui.painter(), rect, &spec);
                sample(ui.painter(), thumb_rect(rect));
                resp
            })
            .inner
        })
        .inner;
    if response
        .on_hover_text("Add a text clip at the playhead")
        .clicked()
        || plus
    {
        out.add_clicked = true;
    }
    out
}

/// The factory default style (TX-001): white, bold, centred.
fn sample(painter: &egui::Painter, thumb: egui::Rect) {
    let size = (thumb.height() * 0.24).clamp(11.0, 18.0);
    let galley = painter.layout_no_wrap(
        TextClipData::PLACEHOLDER.to_owned(),
        FontId::proportional(size),
        Color32::WHITE,
    );
    let at = thumb.center() - galley.size() / 2.0;
    // The UI font has no bold face here: a fine double strike.
    painter.galley(at, galley.clone(), Color32::WHITE);
    painter.galley(at + Vec2::new(0.6, 0.0), galley, Color32::WHITE);
}

/// Inserts a text clip with the factory default style and the default
/// length (TX-001: 3 s, on the project's frame grid) as one undo step, and
/// returns it. It becomes the selection.
pub fn insert_text(
    project: &mut Project,
    target: InsertTarget,
    at: Tick,
) -> Result<ClipId, String> {
    let rate = project.doc().settings().frame_rate;
    let duration = default_text_duration(rate).map_err(|e| e.to_string())?;
    let req = EditRequest::Insert(InsertReq {
        seq: project.doc().root(),
        target,
        at,
        clips: vec![NewClip::text(TextClipData::default(), duration)],
    });
    project.apply(&req, Coalesce::Never)?;
    project
        .selection
        .first()
        .copied()
        .ok_or_else(|| "FreeCut couldn't add the text".to_owned())
}

/// The tile's click (TX-001): a text clip at the playhead on the first
/// free unlocked Text track (a new one when none has room).
pub fn add_text_at_playhead(project: &mut Project) -> Result<ClipId, String> {
    let at = project.playhead;
    insert_text(project, InsertTarget::FirstFree, at)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use freecut_timeline::model::{ClipContent, ProjectSettings, TrackKind};

    use super::*;

    #[test]
    fn adding_text_puts_a_default_clip_on_a_text_track_at_the_playhead() {
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        project.playhead = Tick::from_secs(2).unwrap();
        let clip = add_text_at_playhead(&mut project).unwrap();
        assert_eq!(project.selection, [clip]);
        let doc = project.doc().clone();
        let c = doc.clip(clip).unwrap();
        assert_eq!(c.start(), Tick::from_secs(2).unwrap());
        assert_eq!(c.duration(), Tick::from_secs(3).unwrap());
        let ClipContent::Text(data) = &c.content else {
            panic!("not a text clip");
        };
        assert_eq!(**data, TextClipData::default());
        let (_, track) = doc.sequence(doc.root()).unwrap().find_clip(clip).unwrap();
        assert_eq!(track.kind, TrackKind::Text);
        assert_eq!(project.editor.history().len(), 1, "one undo step");

        // A second one at the same time can't share the track.
        let other = add_text_at_playhead(&mut project).unwrap();
        let doc = project.doc().clone();
        let (_, second) = doc.sequence(doc.root()).unwrap().find_clip(other).unwrap();
        assert_eq!(second.kind, TrackKind::Text);
        assert_ne!(second.id, track.id);
    }
}
