//! The Adjust tab of the asset panel (CL-044, ui-concept §8.1's Grade
//! group): **New** holds the *Adjustment clip* tile, **Saved** the user's
//! saved looks (CL-026) and **LUTs** FreeCut's bundled LUTs (CL-045) and
//! the imported ones (CL-016).
//!
//! - *Adjustment clip:* a click, its hover `+` or `Enter` adds a blank
//!   adjustment clip with CL-006's span (the selection's, else 3 s from
//!   the playhead) and selects it, so the inspector shows its Look lens.
//! - *Saved look:* a click applies it to the selected clips; `+` adds an
//!   adjustment clip carrying it. Right-click: Apply, Add as adjustment
//!   clip, Delete… (the app asks first: FreeCut has no trash of its own).
//! - *LUT:* a click puts it in the selected clips' LUT slot; `+` adds an
//!   adjustment clip carrying it. The LUT rows' toolbar has **Import**.
//!
//! With nothing selected a click does nothing and the status line says how
//! to go on (CL-044). Every apply is one undo entry.
//!
//! *Build note (K4):* tiles show a glyph, not a rendered thumbnail, and
//! hovering a tile doesn't preview it in the player yet; dragging a tile
//! onto the timeline waits for the drop rules of adjustment tracks.

use eframe::egui::{self, Id, Sense, UiBuilder, Vec2};
use freecut_timeline::model::LutRef;

use super::assets::{self, Group};
use crate::looks::{self, SavedLook};
use crate::luts::LutChoice;
use crate::session::Project;
use crate::theme::{self, icons, surface};
use crate::widgets::empty::{empty_state, empty_state_with};
use crate::widgets::import_button::{import_button, import_label_width};
use crate::widgets::tile::{
    Corner, Picture, TileSpec, cell_rect, corner_button_rect, grid_layout, paint_tile, row_pitch,
    thumb_rect,
};

/// The side list's rows, in order (flat across groups).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AdjustFilter {
    /// New › Blank: the Adjustment clip tile.
    #[default]
    New,
    /// Saved › Looks.
    Saved,
    /// LUTs › All.
    Luts,
    /// LUTs › Bundled.
    Bundled,
    /// LUTs › Imported.
    Imported,
}

impl AdjustFilter {
    pub const ALL: [AdjustFilter; 5] = [
        AdjustFilter::New,
        AdjustFilter::Saved,
        AdjustFilter::Luts,
        AdjustFilter::Bundled,
        AdjustFilter::Imported,
    ];

    pub fn index(self) -> usize {
        AdjustFilter::ALL
            .iter()
            .position(|f| *f == self)
            .unwrap_or(0)
    }

    fn is_luts(self) -> bool {
        matches!(
            self,
            AdjustFilter::Luts | AdjustFilter::Bundled | AdjustFilter::Imported
        )
    }
}

/// The tab's view state.
#[derive(Debug, Default)]
pub struct AdjustState {
    pub filter: AdjustFilter,
    pub search: String,
}

/// What the tab asks the app to do.
#[derive(Debug, Default)]
pub struct AdjustOutput {
    pub status: Option<String>,
    /// **Import** on the LUT rows (CL-016).
    pub import_luts: bool,
    /// Delete… on a saved look: `(id, name)`, to confirm.
    pub delete_look: Option<(String, String)>,
}

/// The Adjust Drawer's side list (CL-044).
pub fn groups(project: &Project) -> Vec<Group> {
    let bundled = project.luts.bundled.len();
    let imported = project.luts.imported.len();
    vec![
        Group {
            title: "New",
            rows: vec![("Blank", 1)],
        },
        Group {
            title: "Saved",
            rows: vec![("Looks", project.looks.looks.len())],
        },
        Group {
            title: "LUTs",
            rows: vec![
                ("All", bundled + imported),
                ("Bundled", bundled),
                ("Imported", imported),
            ],
        },
    ]
}

/// One tile of the grid.
struct Item {
    name: String,
    glyph: &'static str,
    tag: Option<&'static str>,
    /// The tooltip.
    hint: String,
    /// It can't be used (a look file FreeCut can't read).
    unusable: bool,
    action: Action,
}

#[derive(Debug, Clone)]
enum Action {
    Blank,
    Look(SavedLook),
    Lut(LutRef),
    Nothing,
}

/// What happened to a tile this frame.
enum Event {
    Click,
    Plus,
    Delete,
}

pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut AdjustState,
    importing: usize,
    chips: Option<&[Group]>,
) -> AdjustOutput {
    let mut out = AdjustOutput::default();
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let gap = ui.spacing().item_spacing.y.max(6.0);
    let h = look.metrics.control_height;
    let filter = state.filter;

    // The toolbar: Import and search on the LUT rows, search on Saved, a
    // hint on New.
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            match filter {
                AdjustFilter::New => {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                "Click to add an adjustment clip over the selection or at the \
                                 playhead",
                            )
                            .text_style(egui::TextStyle::Small)
                            .color(p.text_tertiary),
                        )
                        .wrap(),
                    );
                }
                f => {
                    if f.is_luts() {
                        let label_w = import_label_width(ui, "Import", importing);
                        let icon_only = ui.available_width() - label_w < assets::SEARCH_MIN + 40.0;
                        let import = import_button(ui, "Import", importing, icon_only);
                        if importing == 0
                            && import
                                .on_hover_text("Import .cube and .3dl LUT files")
                                .clicked()
                        {
                            out.import_luts = true;
                        }
                    }
                    let hint = if f.is_luts() {
                        "Search LUTs"
                    } else {
                        "Search saved looks"
                    };
                    assets::search_field(ui, Id::new("adjust-search"), &mut state.search, hint);
                }
            }
        },
    );
    if let Some(groups) = chips {
        ui.add_space(gap);
        if let Some(i) = assets::chip_row(ui, Id::new("adjust-chips"), groups, filter.index())
            && let Some(f) = AdjustFilter::ALL.get(i)
        {
            state.filter = *f;
        }
    }
    ui.add_space(gap);

    let items = items(project, filter, &state.search);
    if items.is_empty() {
        empty(ui, project, state, &mut out);
        return out;
    }
    let events = grid(ui, &items);
    for (i, event) in events {
        let item = &items[i];
        let result = match (&item.action, event) {
            (Action::Blank, Event::Click | Event::Plus) => {
                looks::add_adjustment(project, None).map(|_| "Adjustment clip added".to_owned())
            }
            (Action::Look(l), Event::Click) => looks::apply_look(project, l),
            (Action::Look(l), Event::Plus) => {
                looks::add_adjustment(project, Some(looks::stack_of_look(l)))
                    .map(|_| format!("Adjustment clip added with {}", l.name))
            }
            (Action::Look(l), Event::Delete) => {
                out.delete_look = Some((l.id.clone(), l.name.clone()));
                continue;
            }
            (Action::Lut(lut), Event::Click) => {
                let luts = std::sync::Arc::clone(&project.luts);
                looks::apply_lut(project, lut, |l| luts.name_of(l))
            }
            (Action::Lut(lut), Event::Plus) => {
                looks::add_adjustment(project, Some(looks::stack_of_lut(lut)))
                    .map(|_| format!("Adjustment clip added with {}", item.name))
            }
            _ => continue,
        };
        out.status = Some(result.unwrap_or_else(|e| e));
    }
    out
}

/// The tiles `filter` shows, matching `search` (any case).
fn items(project: &Project, filter: AdjustFilter, search: &str) -> Vec<Item> {
    let needle = search.trim().to_lowercase();
    let matches = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);
    let lut_item = |c: &LutChoice, bundled: bool| Item {
        name: c.name.clone(),
        glyph: icons::LENS_LOOK,
        tag: if project.luts.is_missing(&c.lut) {
            Some("LUT missing")
        } else if bundled {
            Some("FreeCut")
        } else {
            None
        },
        hint: if c.hint.is_empty() {
            c.lut.file_name.clone()
        } else {
            c.hint.clone()
        },
        unusable: false,
        action: Action::Lut(c.lut.clone()),
    };
    match filter {
        AdjustFilter::New => vec![Item {
            name: "Adjustment clip".to_owned(),
            glyph: icons::ADJUST,
            tag: None,
            hint: "A clip that grades everything below it for as long as it spans".to_owned(),
            unusable: false,
            action: Action::Blank,
        }],
        AdjustFilter::Saved => {
            let mut out: Vec<Item> = project
                .looks
                .looks
                .iter()
                .filter(|l| matches(&l.name))
                .map(|l| Item {
                    name: l.name.clone(),
                    glyph: icons::PALETTE,
                    tag: l
                        .lut
                        .as_ref()
                        .filter(|lut| project.luts.is_missing(lut))
                        .map(|_| "LUT missing"),
                    hint: l.summary(),
                    unusable: false,
                    action: Action::Look(l.clone()),
                })
                .collect();
            out.extend(
                project
                    .looks
                    .unavailable
                    .iter()
                    .filter(|(id, _)| matches(id))
                    .map(|(id, why)| Item {
                        name: id.clone(),
                        glyph: icons::WARNING,
                        tag: Some("Unavailable"),
                        hint: why.clone(),
                        unusable: true,
                        action: Action::Nothing,
                    }),
            );
            out
        }
        f => {
            let mut out = Vec::new();
            if f != AdjustFilter::Imported {
                out.extend(
                    project
                        .luts
                        .bundled
                        .iter()
                        .filter(|c| matches(&c.name))
                        .map(|c| lut_item(c, true)),
                );
            }
            if f != AdjustFilter::Bundled {
                out.extend(
                    project
                        .luts
                        .imported
                        .iter()
                        .filter(|c| matches(&c.name))
                        .map(|c| lut_item(c, false)),
                );
            }
            out
        }
    }
}

/// The empty states of CL-044.
fn empty(ui: &mut egui::Ui, project: &Project, state: &AdjustState, out: &mut AdjustOutput) {
    let searching = !state.search.trim().is_empty();
    if searching {
        empty_state(ui, icons::SEARCH, "No matches", "Try another word");
        return;
    }
    match state.filter {
        AdjustFilter::Saved if project.looks.loading => {
            empty_state(ui, icons::PALETTE, "Reading your saved looks…", "");
        }
        AdjustFilter::Saved => empty_state(
            ui,
            icons::PALETTE,
            "No saved looks yet",
            "Grades you save with Save look… appear here",
        ),
        AdjustFilter::Imported | AdjustFilter::Luts => {
            let e = empty_state_with(
                ui,
                icons::LENS_LOOK,
                "No imported LUTs yet",
                "Import .cube or .3dl files to use them here",
                Some("Import…"),
                None,
            );
            out.import_luts |= e.primary;
        }
        AdjustFilter::Bundled | AdjustFilter::New => {}
    }
}

/// Tile `i`'s id (its name keeps a tile's state when the list changes).
pub fn tile_id(i: usize, name: &str) -> Id {
    Id::new(("adjust-tile", i, name))
}

/// Lays out and draws the tiles; returns what happened to which.
fn grid(ui: &mut egui::Ui, items: &[Item]) -> Vec<(usize, Event)> {
    let look = theme::look(ui.ctx());
    let compact = look.metrics.control_height < 28.0;
    let (min, tile_gap) = if compact { (80.0, 6.0) } else { (96.0, 8.0) };
    let mut events = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("adjust-grid")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (cols, tile_w) = grid_layout(ui.available_width(), min, tile_gap);
            let rows = items.len().div_ceil(cols.max(1));
            let origin = ui.cursor().min;
            for (i, item) in items.iter().enumerate() {
                let cell = cell_rect(origin, i, cols, tile_w, tile_gap);
                let id = tile_id(i, &item.name);
                let mut spec = TileSpec {
                    name: &item.name,
                    mark: surface::CLIP_ADJUST,
                    picture: Picture::Glyph(item.glyph),
                    badge: None,
                    tag: item.tag,
                    offline: false,
                    selected: false,
                    hovered: false,
                    corner: None,
                };
                let response = ui
                    .scope_builder(UiBuilder::new().max_rect(cell), |ui| {
                        let sense = if item.unusable {
                            Sense::hover()
                        } else {
                            Sense::click()
                        };
                        let (rect, _) = ui.allocate_exact_size(cell.size(), Sense::hover());
                        let resp = ui.interact(rect, id, sense);
                        let hovered = !item.unusable && ui.rect_contains_pointer(rect);
                        if hovered {
                            let r = corner_button_rect(thumb_rect(rect));
                            let button = ui
                                .interact(r, id.with("add"), Sense::click())
                                .on_hover_text("Add as an adjustment clip");
                            if button.clicked() {
                                events.push((i, Event::Plus));
                            }
                            spec.corner = Some(Corner::Add {
                                hot: button.hovered(),
                            });
                        }
                        spec.hovered = hovered;
                        let painter = if item.unusable {
                            let mut faded = ui.painter().clone();
                            faded.multiply_opacity(0.55);
                            faded
                        } else {
                            ui.painter().clone()
                        };
                        paint_tile(ui, &painter, rect, &spec);
                        resp
                    })
                    .inner;
                let response = response.on_hover_text(&item.hint);
                let plus_taken = events.last().is_some_and(|(j, _)| *j == i);
                if !plus_taken
                    && (response.clicked()
                        || (response.has_focus()
                            && ui.input(|inp| inp.key_pressed(egui::Key::Enter))))
                {
                    events.push((i, Event::Click));
                }
                if let Action::Look(_) = item.action {
                    response.context_menu(|ui| {
                        if ui.button("Apply to the selection").clicked() {
                            events.push((i, Event::Click));
                            ui.close();
                        }
                        if ui.button("Add as an adjustment clip").clicked() {
                            events.push((i, Event::Plus));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Delete…").clicked() {
                            events.push((i, Event::Delete));
                            ui.close();
                        }
                    });
                }
            }
            ui.allocate_space(Vec2::new(
                ui.available_width(),
                rows as f32 * row_pitch(tile_w, tile_gap),
            ));
        });
    events
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use std::sync::Arc;

    use freecut_timeline::model::{
        ColorParamId, ColorStack, ProjectSettings, Section, SectionMask, SectionSnapshot,
    };

    use super::*;
    use crate::looks::SavedLooks;
    use crate::luts::AppLuts;

    fn project() -> Project {
        let mut p = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        p.luts = Arc::new(AppLuts::new().choices(&Default::default()));
        let stack = ColorStack::default().with(ColorParamId::Saturation, -40);
        p.looks = Arc::new(SavedLooks {
            looks: vec![SavedLook {
                id: "x".into(),
                name: "Muted teal".into(),
                snapshot: Arc::new(SectionSnapshot::new(
                    &stack,
                    SectionMask::of(&[Section::Color]),
                )),
                lut: None,
            }],
            unavailable: vec![("old".into(), "Can't be read".into())],
            loading: false,
        });
        p
    }

    #[test]
    fn the_side_list_counts_each_group() {
        let p = project();
        let g = groups(&p);
        assert_eq!(g[0].rows, [("Blank", 1)]);
        assert_eq!(g[1].rows, [("Looks", 1)]);
        assert_eq!(g[2].rows[1], ("Bundled", 8));
        assert_eq!(AdjustFilter::ALL.len(), 5);
        assert_eq!(AdjustFilter::Imported.index(), 4);
    }

    use crate::ui::drag::tests::{Frames, button};

    fn run(
        frames: &mut Frames,
        p: &mut Project,
        state: &mut AdjustState,
        events: Vec<egui::Event>,
    ) -> AdjustOutput {
        let mut out = AdjustOutput::default();
        frames.run(events, |ui| out = show(ui, p, state, 0, None));
        out
    }

    fn click(
        frames: &mut Frames,
        p: &mut Project,
        state: &mut AdjustState,
        id: Id,
    ) -> AdjustOutput {
        let at = frames
            .ctx
            .read_response(id)
            .expect("the tile is shown")
            .rect
            .center();
        run(frames, p, state, vec![egui::Event::PointerMoved(at)]);
        run(frames, p, state, vec![button(at, true)]);
        run(frames, p, state, vec![button(at, false)])
    }

    #[test]
    fn clicks_add_an_adjustment_clip_or_apply_a_look_as_one_entry_each() {
        let mut p = project();
        let mut state = AdjustState::default();
        let mut frames = Frames::new();
        run(&mut frames, &mut p, &mut state, Vec::new());
        run(&mut frames, &mut p, &mut state, Vec::new());
        let out = click(
            &mut frames,
            &mut p,
            &mut state,
            tile_id(0, "Adjustment clip"),
        );
        assert_eq!(out.status.as_deref(), Some("Adjustment clip added"));
        assert_eq!(p.editor.history().len(), 1);
        let adj = p.selection[0];

        state.filter = AdjustFilter::Saved;
        run(&mut frames, &mut p, &mut state, Vec::new());
        run(&mut frames, &mut p, &mut state, Vec::new());
        let out = click(&mut frames, &mut p, &mut state, tile_id(0, "Muted teal"));
        assert_eq!(out.status.as_deref(), Some("Look applied to 1 clip"));
        assert_eq!(p.editor.history().len(), 2, "one entry");
        let s = p.doc().clip(adj).unwrap().color.clone().unwrap();
        assert_eq!(s.basic.saturation, -40);

        // Nothing selected: nothing happens, and the status line says why.
        p.selection.clear();
        let out = click(&mut frames, &mut p, &mut state, tile_id(0, "Muted teal"));
        assert_eq!(out.status.as_deref(), Some(looks::SELECT_FIRST));
        assert_eq!(p.editor.history().len(), 2);
    }

    #[test]
    fn filters_and_search_pick_the_tiles() {
        let p = project();
        assert_eq!(items(&p, AdjustFilter::New, "").len(), 1);
        let saved = items(&p, AdjustFilter::Saved, "");
        assert_eq!(saved.len(), 2, "unusable files are listed too");
        assert!(saved[1].unusable && saved[1].tag == Some("Unavailable"));
        assert_eq!(items(&p, AdjustFilter::Saved, "TEAL").len(), 1);
        assert_eq!(items(&p, AdjustFilter::Bundled, "").len(), 8);
        assert!(items(&p, AdjustFilter::Imported, "").is_empty());
        assert_eq!(items(&p, AdjustFilter::Luts, "mono").len(), 2);
    }
}
