//! The Home screen (UI-016, PM-001, PM-017; ui-concept §13.29) and the
//! new-project sheet (§13.28).
//!
//! - **Rail:** a panel on the ground, 8 pt from the window's edges: 200 pt
//!   wide, 56 pt (icons only, labels in tooltips) under a 960 pt window,
//!   hidden under 720 pt, when its actions sit beside the heading as 34 pt
//!   round buttons ([`home_rail_width`]). Rows are 36 pt: *New project* and
//!   *Projects* at the top, *Open project…* at the bottom.
//! - **Page:** a column at most 1120 pt wide with side margins
//!   `clamp(width / 16, 24, 64)` and a top margin `clamp(height / 10, 32,
//!   96)` ([`home_margins`]): the "Projects" heading in `type.display`, a
//!   subtitle, the quick-start chips (PM-001's canvases), then the grid.
//! - **Grid:** columns at least 216 pt wide, 16 pt apart; each card a 16:9
//!   poster and 64 pt of text ([`home_grid`]). *New project* comes first.
//!   A card shows the project's cover (the frame the Stage showed at its
//!   last save) or, without one, a neutral tile at its canvas's shape; a
//!   project whose file is gone is dimmed and marked "Not found". Hover
//!   lifts the card and shows × (remove from the list; the file stays).
//! - **Footer:** the version and About.

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Id, Layout, Rect, Sense, Stroke, StrokeKind,
    TextureHandle, Vec2,
};

use crate::recent::{RecentEntry, canvas_badge, edited_ago};
use crate::theme::{self, Type, fonts, icons};
use crate::widgets::icon_button::keyboard_focus;
use crate::widgets::panel::{Beyond, Sides, ground_frame, show_panel};
use crate::widgets::sheet::{self, ButtonKind, Sheet};

/// PM-001's quick-start canvases, also the new-project sheet's choices.
pub const CANVAS_PRESETS: &[(&str, u32, u32)] = &[
    ("16:9 Landscape", 1920, 1080),
    ("9:16 Portrait", 1080, 1920),
    ("1:1 Square", 1080, 1080),
    ("4:5 Portrait", 1080, 1350),
    ("21:9 Cinema", 2560, 1080),
];
/// The frame rates a new project can have.
pub const RATE_PRESETS: &[(&str, u32, u32)] = &[
    ("23.976", 24_000, 1001),
    ("24", 24, 1),
    ("25", 25, 1),
    ("29.97", 30_000, 1001),
    ("30", 30, 1),
    ("50", 50, 1),
    ("59.94", 60_000, 1001),
    ("60", 60, 1),
];
/// The rate a new project starts with (30 fps).
pub const DEFAULT_RATE: usize = 4;

/// The page column's widest.
pub const PAGE_MAX: f32 = 1120.0;
/// A grid card's narrowest, and the gap between cards.
pub const CARD_MIN: f32 = 216.0;
pub const CARD_GAP: f32 = 16.0;
/// The text block under a card's poster.
pub const CARD_TEXT: f32 = 64.0;
/// A rail row's height.
pub const RAIL_ROW: f32 = 36.0;

/// The rail's width in a window `w` pt wide: 200 from 960 pt, 56 from
/// 720 pt, else hidden.
pub fn home_rail_width(w: f32) -> f32 {
    if w >= 960.0 {
        200.0
    } else if w >= 720.0 {
        56.0
    } else {
        0.0
    }
}

/// The page's side and top margins in a `w` × `h` page.
pub fn home_margins(w: f32, h: f32) -> (f32, f32) {
    ((w / 16.0).clamp(24.0, 64.0), (h / 10.0).clamp(32.0, 96.0))
}

/// The grid in a column `width` pt wide: `(columns, card width, card
/// height)`. Columns are at least [`CARD_MIN`] wide and fill the width
/// exactly.
pub fn home_grid(width: f32) -> (usize, f32, f32) {
    let width = width.max(1.0);
    let cols = (((width + CARD_GAP) / (CARD_MIN + CARD_GAP)).floor() as usize).max(1);
    let card_w = (width - (cols - 1) as f32 * CARD_GAP) / cols as f32;
    (cols, card_w, card_w * 9.0 / 16.0 + CARD_TEXT)
}

/// "16:9 · 1080p · 0:42 · 30 fps": a card's details line.
pub fn card_details(entry: &RecentEntry) -> String {
    let secs = (entry.duration_ms + 500) / 1000;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let length = if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    };
    let rate = if entry.rate.1 <= 1 {
        entry.rate.0.to_string()
    } else {
        let fps = f64::from(entry.rate.0) / f64::from(entry.rate.1);
        let t = format!("{fps:.3}");
        t.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    format!("{} · {length} · {rate} fps", canvas_badge(entry.canvas))
}

/// What Home asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeAction {
    /// The new-project sheet.
    NewProject,
    /// A project with quick-start canvas `n` at 30 fps, at once.
    Quick(usize),
    /// The OS file picker.
    Open,
    /// Open this project.
    OpenRecent(PathBuf),
    /// Forget this project (the file stays).
    Forget(PathBuf),
    /// Show the file in the file manager.
    Reveal(PathBuf),
    About,
}

/// What Home shows.
pub struct HomeView<'a> {
    pub recent: &'a [RecentEntry],
    pub covers: &'a HashMap<PathBuf, TextureHandle>,
    pub opening: bool,
    /// Now, in Unix seconds.
    pub now: u64,
    pub version: &'a str,
}

impl std::fmt::Debug for HomeView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HomeView")
            .field("recent", &self.recent.len())
            .finish_non_exhaustive()
    }
}

/// Draws Home under the top bar and returns what was asked for.
pub fn show(ui: &mut egui::Ui, view: &HomeView<'_>) -> Vec<HomeAction> {
    let mut actions = Vec::new();
    let look = theme::look(ui.ctx());
    let window = ui.ctx().content_rect();
    let rail = home_rail_width(window.width());
    if rail > 0.0 {
        let sides = Sides {
            left: Beyond::Window,
            right: Beyond::Ground,
            top: Beyond::Ground,
            bottom: Beyond::Window,
        };
        egui::Panel::left("home-rail")
            .exact_size(rail + look.metrics.gutter)
            .resizable(false)
            .frame(ground_frame(&look, sides))
            .show_separator_line(false)
            .show(ui, |ui| {
                show_panel(ui, egui::Margin::same(8), |ui| {
                    rail_rows(ui, rail < 100.0, &mut actions);
                });
            });
    }
    let ground = egui::Frame::NONE.fill(look.palette.bg_app);
    egui::CentralPanel::default().frame(ground).show(ui, |ui| {
        page(ui, view, rail == 0.0, &mut actions);
    });
    if view.opening {
        opening_card(ui.ctx());
    }
    actions
}

fn rail_rows(ui: &mut egui::Ui, narrow: bool, actions: &mut Vec<HomeAction>) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let full = ui.available_rect_before_wrap();
    if rail_row(ui, icons::FILE_PLUS, "New project", narrow, false).clicked() {
        actions.push(HomeAction::NewProject);
    }
    rail_row(ui, icons::GRID, "Projects", narrow, true);
    // Open project… sits at the bottom.
    let bottom = Rect::from_min_max(
        egui::pos2(full.left(), full.bottom() - RAIL_ROW),
        full.right_bottom(),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bottom));
    if rail_row(
        &mut child,
        icons::FOLDER_OPEN,
        "Open project…",
        narrow,
        false,
    )
    .clicked()
    {
        actions.push(HomeAction::Open);
    }
}

/// A rail row: 36 pt, `r.md`, a 16 pt icon and a `type.label` label
/// (only the icon when `narrow`, the label in its tooltip). Idle
/// `text.secondary`, hover `bg.field`, current `bg.active`.
fn rail_row(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    narrow: bool,
    current: bool,
) -> egui::Response {
    let p = theme::look(ui.ctx()).palette;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), RAIL_ROW), Sense::click());
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(
            response.id.with("hover"),
            response.hovered(),
            ui.style().animation_time,
        );
        let fill = if current {
            p.bg_active
        } else {
            Color32::TRANSPARENT.lerp_to_gamma(p.bg_field, t)
        };
        ui.painter().rect_filled(rect, theme::R_MD, fill);
        let ink = if current {
            p.text_primary
        } else {
            p.text_secondary.lerp_to_gamma(p.text_primary, t)
        };
        let icon_x = if narrow {
            rect.center().x
        } else {
            rect.left() + 12.0 + 8.0
        };
        ui.painter().text(
            egui::pos2(icon_x, rect.center().y),
            Align2::CENTER_CENTER,
            icon,
            fonts::icon_font(ui.ctx(), 16.0, current),
            ink,
        );
        if !narrow {
            let font = Type::Label.style(ui.style()).resolve(ui.style());
            ui.painter().text(
                egui::pos2(rect.left() + 40.0, rect.center().y),
                Align2::LEFT_CENTER,
                label,
                font,
                ink,
            );
        }
        if keyboard_focus(ui, &response) {
            crate::widgets::focus_ring(ui, rect, theme::R_MD);
        }
    }
    let name = label.to_owned();
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, current, &name));
    if narrow {
        response.on_hover_text(label)
    } else {
        response
    }
}

fn page(ui: &mut egui::Ui, view: &HomeView<'_>, no_rail: bool, actions: &mut Vec<HomeAction>) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let area = ui.available_rect_before_wrap();
    let (side, top) = home_margins(area.width(), area.height());
    egui::ScrollArea::vertical()
        .id_salt("home-page")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let avail = ui.available_width();
            let column = (avail - 2.0 * side).clamp(1.0, PAGE_MAX);
            let left = (avail - column) / 2.0;
            ui.add_space(top);
            ui.horizontal(|ui| {
                ui.add_space(left);
                ui.vertical(|ui| {
                    ui.set_width(column);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    heading(ui, no_rail, actions);
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Pick up where you left off, or start something new.")
                            .size(14.0)
                            .color(p.text_secondary),
                    );
                    ui.add_space(20.0);
                    quick_start(ui, actions);
                    ui.add_space(28.0);
                    grid(ui, view, column, actions);
                    if view.recent.is_empty() {
                        ui.add_space(16.0);
                        ui.label(
                            egui::RichText::new(
                                "Projects you open or save show here. You can also drop a \
                                 .freecut file on this window.",
                            )
                            .text_style(egui::TextStyle::Small)
                            .color(p.text_tertiary),
                        );
                    }
                    ui.add_space(40.0);
                    footer(ui, view.version, actions);
                    ui.add_space(24.0);
                });
            });
        });
}

/// "Projects" in `type.display` with −0.6 pt tracking; without the rail,
/// its two actions as 34 pt round buttons at the right.
fn heading(ui: &mut egui::Ui, no_rail: bool, actions: &mut Vec<HomeAction>) {
    let p = theme::look(ui.ctx()).palette;
    ui.horizontal(|ui| {
        let mut job = egui::text::LayoutJob::default();
        job.append(
            "Projects",
            0.0,
            egui::TextFormat {
                font_id: Type::Display.style(ui.style()).resolve(ui.style()),
                color: p.text_primary,
                extra_letter_spacing: -0.6,
                ..egui::TextFormat::default()
            },
        );
        ui.label(job);
        if no_rail {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if round_button(ui, icons::FOLDER_OPEN, "Open project…", false).clicked() {
                    actions.push(HomeAction::Open);
                }
                if round_button(ui, icons::PLUS, "New project", true).clicked() {
                    actions.push(HomeAction::NewProject);
                }
            });
        }
    });
}

fn round_button(ui: &mut egui::Ui, icon: &str, tip: &str, accent: bool) -> egui::Response {
    let p = theme::look(ui.ctx()).palette;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), Sense::click());
    let hovered = response.hovered();
    let (fill, ink) = match (accent, hovered) {
        (true, false) => (p.accent, p.text_on_accent),
        (true, true) => (p.accent_hover, p.text_on_accent),
        (false, false) => (p.bg_field, p.text_primary),
        (false, true) => (p.bg_hover, p.text_primary),
    };
    ui.painter().circle_filled(rect.center(), 17.0, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        fonts::icon_font(ui.ctx(), 16.0, false),
        ink,
    );
    if keyboard_focus(ui, &response) {
        crate::widgets::focus_ring(ui, rect, 17);
    }
    let name = tip.to_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    response.on_hover_text(tip)
}

/// PM-001's quick-start canvases as chips with their shapes.
fn quick_start(ui: &mut egui::Ui, actions: &mut Vec<HomeAction>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        let job = sheet::eyebrow(ui, "Quick start");
        let galley = ui.painter().layout_job(job);
        let h = theme::look(ui.ctx()).metrics.control_height;
        let (cell, _) =
            ui.allocate_exact_size(egui::vec2(galley.size().x + 8.0, h), Sense::hover());
        ui.painter().galley(
            egui::pos2(cell.left(), cell.center().y - galley.size().y / 2.0),
            galley,
            Color32::PLACEHOLDER,
        );
        for (i, (name, w, h)) in CANVAS_PRESETS.iter().enumerate() {
            let chip = sheet::choice_chip(ui, name, Some((*w as f32, *h as f32)), false)
                .on_hover_text(format!("A new {w} × {h} project at 30 fps"));
            if chip.clicked() {
                actions.push(HomeAction::Quick(i));
            }
        }
    });
}

fn grid(ui: &mut egui::Ui, view: &HomeView<'_>, width: f32, actions: &mut Vec<HomeAction>) {
    let (cols, card_w, card_h) = home_grid(width);
    let n = view.recent.len() + 1;
    let rows = n.div_ceil(cols);
    let height = rows as f32 * card_h + rows.saturating_sub(1) as f32 * CARD_GAP;
    let (area, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    for i in 0..n {
        let (row, col) = (i / cols, i % cols);
        let cell = Rect::from_min_size(
            area.min
                + egui::vec2(
                    col as f32 * (card_w + CARD_GAP),
                    row as f32 * (card_h + CARD_GAP),
                ),
            egui::vec2(card_w, card_h),
        );
        if !ui.is_rect_visible(cell) {
            continue;
        }
        if i == 0 {
            if new_card(ui, cell).clicked() {
                actions.push(HomeAction::NewProject);
            }
        } else {
            let entry = &view.recent[i - 1];
            let cover = view.covers.get(&entry.path);
            project_card(ui, cell, entry, cover, view.now, actions);
        }
    }
}

fn paint_card_ground(ui: &egui::Ui, rect: Rect, t: f32) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let fill = p.bg_panel.lerp_to_gamma(p.bg_card, t);
    let stroke = p.border_subtle.lerp_to_gamma(p.border_strong, t);
    ui.painter().rect_filled(rect, theme::R_LG, fill);
    ui.painter().rect_stroke(
        rect,
        theme::R_LG,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
}

fn new_card(ui: &mut egui::Ui, rect: Rect) -> egui::Response {
    let p = theme::look(ui.ctx()).palette;
    let id = Id::new("home-new-project");
    let response = ui.interact(rect, id, Sense::click());
    let t = ui.ctx().animate_bool_with_time(
        id.with("hover"),
        response.hovered(),
        ui.style().animation_time,
    );
    paint_card_ground(ui, rect, t);
    let centre = rect.center() - egui::vec2(0.0, 12.0);
    ui.painter().text(
        centre,
        Align2::CENTER_CENTER,
        icons::PLUS,
        fonts::icon_font(ui.ctx(), 28.0, false),
        p.accent_text,
    );
    ui.painter().text(
        centre + egui::vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        "New project",
        Type::Label.style(ui.style()).resolve(ui.style()),
        p.text_primary,
    );
    if keyboard_focus(ui, &response) {
        crate::widgets::focus_ring(ui, rect, theme::R_LG);
    }
    // At launch the keyboard starts on New project (UI-016).
    let started = id.with("focused-once");
    if !ui.data(|d| d.get_temp::<bool>(started).unwrap_or(false)) {
        ui.data_mut(|d| d.insert_temp(started, true));
        response.request_focus();
    }
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New project"));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The poster part of a card.
pub fn poster_rect(card: Rect) -> Rect {
    Rect::from_min_size(
        card.min,
        egui::vec2(card.width(), card.width() * 9.0 / 16.0),
    )
}

/// Home's × on a hovered card: a 24 pt circle 6 pt in from the poster's
/// top-right corner.
pub fn forget_rect(poster: Rect) -> Rect {
    Rect::from_min_size(
        egui::pos2(poster.right() - 6.0 - 24.0, poster.top() + 6.0),
        Vec2::splat(24.0),
    )
}

/// How opaque the cover of a project whose file is gone is.
const MISSING_COVER_OPACITY: f32 = 0.6;

/// A card's name colour: `text.secondary` when the file is gone (the badge
/// and the dimmed cover say so too), else `text.primary`.
fn name_colour(p: &theme::Palette, found: bool) -> Color32 {
    if found {
        p.text_primary
    } else {
        p.text_secondary
    }
}

fn project_card(
    ui: &mut egui::Ui,
    rect: Rect,
    entry: &RecentEntry,
    cover: Option<&TextureHandle>,
    now: u64,
    actions: &mut Vec<HomeAction>,
) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let id = Id::new(("home-card", &entry.path));
    let response = ui.interact(rect, id, Sense::click());
    let hovered = response.hovered() || ui.rect_contains_pointer(rect);
    let t = ui
        .ctx()
        .animate_bool_with_time(id.with("hover"), hovered, ui.style().animation_time);
    // A project whose file is gone dims its cover only: its text keeps
    // its contrast (UI-013).
    let painter = if entry.found {
        ui.painter().clone()
    } else {
        let mut dim = ui.painter().clone();
        dim.multiply_opacity(MISSING_COVER_OPACITY);
        dim
    };
    paint_card_ground(ui, rect, t);
    let poster = poster_rect(rect).shrink(1.0);
    let top = CornerRadius {
        nw: theme::R_LG - 1,
        ne: theme::R_LG - 1,
        sw: 0,
        se: 0,
    };
    painter.rect_filled(poster, top, p.bg_well);
    match cover {
        Some(tex) => {
            let size = tex.size_vec2();
            let uv =
                crate::widgets::tile::cover_uv(size.x / size.y.max(1.0), poster.aspect_ratio());
            let mut image = egui::Image::from_texture((tex.id(), size))
                .uv(uv)
                .corner_radius(top);
            if !entry.found {
                image = image.tint(Color32::from_white_alpha(150));
            }
            image.paint_at(ui, poster);
        }
        None => {
            // PM-001: a neutral tile at the project's shape.
            let side = poster.height() * 0.62;
            let tile = sheet::aspect_glyph(
                poster.center(),
                entry.canvas.0 as f32,
                entry.canvas.1 as f32,
                side,
            );
            painter.rect_filled(tile, theme::R_SM, p.bg_field);
            painter.rect_stroke(
                tile,
                theme::R_SM,
                Stroke::new(1.0, p.border_strong),
                StrokeKind::Inside,
            );
            painter.text(
                tile.center(),
                Align2::CENTER_CENTER,
                icons::FILE_VIDEO,
                fonts::icon_font(ui.ctx(), 20.0, false),
                p.text_tertiary,
            );
        }
    }
    painter.rect_filled(
        Rect::from_min_size(
            egui::pos2(rect.left(), poster.bottom()),
            egui::vec2(rect.width(), 1.0),
        ),
        0.0,
        p.border_subtle,
    );
    if !entry.found {
        badge(ui, poster, "Not found");
    }
    // The text block.
    let x = rect.left() + 12.0;
    let width = rect.width() - 24.0;
    let mut y = poster.bottom() + 11.0;
    let name_font = Type::Label.style(ui.style()).resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());
    for (text, font, colour) in [
        (entry.name.clone(), name_font, name_colour(&p, entry.found)),
        (card_details(entry), small.clone(), p.text_tertiary),
        (
            format!("Edited {}", edited_ago(entry.opened, now).to_lowercase()),
            small,
            p.text_tertiary,
        ),
    ] {
        let galley = crate::widgets::side_list::one_line(ui, &text, font, colour, width);
        let h = galley.size().y;
        ui.painter().galley(egui::pos2(x, y), galley, colour);
        y += h + 3.0;
    }
    // The × to forget it.
    let mut forgot = false;
    if hovered {
        let x_rect = forget_rect(poster);
        let x_resp = ui
            .interact(x_rect, id.with("forget"), Sense::click())
            .on_hover_text("Remove from list (the file stays)");
        ui.painter()
            .circle_filled(x_rect.center(), 12.0, p.overlay_badge);
        if x_resp.hovered() {
            // A white ring says the × itself is under the pointer.
            ui.painter()
                .circle_stroke(x_rect.center(), 11.25, Stroke::new(1.5, Color32::WHITE));
        }
        ui.painter().text(
            x_rect.center(),
            Align2::CENTER_CENTER,
            icons::CLOSE,
            fonts::icon_font(ui.ctx(), 12.0, false),
            Color32::WHITE,
        );
        if x_resp.clicked() {
            forgot = true;
            actions.push(HomeAction::Forget(entry.path.clone()));
        }
    }
    if keyboard_focus(ui, &response) {
        crate::widgets::focus_ring(ui, rect, theme::R_LG);
    }
    let path = entry.path.display().to_string();
    let response = if entry.found {
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&path)
    } else {
        response.on_hover_text(format!("Not found: {path}"))
    };
    if response.clicked() && !forgot && entry.found {
        actions.push(HomeAction::OpenRecent(entry.path.clone()));
    }
    response.context_menu(|ui| {
        if ui
            .add_enabled(entry.found, egui::Button::new("Open"))
            .clicked()
        {
            actions.push(HomeAction::OpenRecent(entry.path.clone()));
        }
        if ui
            .add_enabled(entry.found, egui::Button::new("Show in folder"))
            .clicked()
        {
            actions.push(HomeAction::Reveal(entry.path.clone()));
        }
        ui.separator();
        if ui.button("Remove from list").clicked() {
            actions.push(HomeAction::Forget(entry.path.clone()));
        }
    });
    let name = entry.name.clone();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
}

/// A badge in the poster's bottom-left corner: white `type.micro` on
/// `overlay.badge`.
fn badge(ui: &egui::Ui, poster: Rect, text: &str) {
    let p = theme::look(ui.ctx()).palette;
    let font = Type::Micro.style(ui.style()).resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, Color32::WHITE);
    let rect = Rect::from_min_size(
        egui::pos2(poster.left() + 8.0, poster.bottom() - 8.0 - 18.0),
        egui::vec2(galley.size().x + 12.0, 18.0),
    );
    ui.painter().rect_filled(rect, theme::R_XS, p.overlay_badge);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

fn footer(ui: &mut egui::Ui, version: &str, actions: &mut Vec<HomeAction>) {
    let p = theme::look(ui.ctx()).palette;
    sheet::hairline(ui);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let small = |t: &str| {
            egui::RichText::new(t)
                .text_style(egui::TextStyle::Small)
                .color(p.text_tertiary)
        };
        ui.label(small(&format!(
            "FreeCut {version} · free and open source, GPL-3.0-or-later · no account, no watermark"
        )));
        ui.label(small("·"));
        let about = ui.add(
            egui::Label::new(
                egui::RichText::new("About FreeCut")
                    .text_style(egui::TextStyle::Small)
                    .color(p.accent_text),
            )
            .sense(Sense::click()),
        );
        if about
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            actions.push(HomeAction::About);
        }
    });
}

/// "Opening…" while a project loads (PM-013), centred over Home.
fn opening_card(ctx: &egui::Context) {
    let look = theme::look(ctx);
    let p = look.palette;
    egui::Area::new(Id::new("home-opening"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .interactable(false)
        .show(ctx, |ui| {
            sheet::sheet_frame(&look)
                .fill(p.bg_card)
                .inner_margin(egui::Margin::symmetric(20, 14))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new("Opening project…")
                                .text_style(egui::TextStyle::Body)
                                .color(p.text_primary),
                        );
                    });
                });
        });
}

/// The new-project sheet's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewProjectDialog {
    pub preset: usize,
    pub rate: usize,
}

impl Default for NewProjectDialog {
    fn default() -> NewProjectDialog {
        NewProjectDialog {
            preset: 0,
            rate: DEFAULT_RATE,
        }
    }
}

/// What the new-project sheet decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewProjectChoice {
    Create,
    Cancel,
}

/// The new-project sheet (§13.28): canvas and frame rate as chips in form
/// rows, *Cancel* and the accent *Create*; `Enter` creates.
pub fn new_project_sheet(
    ctx: &egui::Context,
    dialog: &mut NewProjectDialog,
) -> Option<NewProjectChoice> {
    let mut choice = None;
    let response = Sheet::new(Id::new("new-project"), "New project")
        .width(560.0)
        .show(ctx, |s| {
            s.body(|ui| {
                let (_, w, h) = CANVAS_PRESETS[dialog.preset.min(CANVAS_PRESETS.len() - 1)];
                sheet::form_row(ui, "Canvas", false, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                        for (i, (name, w, h)) in CANVAS_PRESETS.iter().enumerate() {
                            if sheet::choice_chip(
                                ui,
                                name,
                                Some((*w as f32, *h as f32)),
                                dialog.preset == i,
                            )
                            .clicked()
                            {
                                dialog.preset = i;
                            }
                        }
                    });
                    readout(ui, &format!("{w} × {h} pixels"));
                });
                sheet::form_row(ui, "Frame rate", true, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                        for (i, (name, _, _)) in RATE_PRESETS.iter().enumerate() {
                            if sheet::choice_chip(ui, name, None, dialog.rate == i).clicked() {
                                dialog.rate = i;
                            }
                        }
                    });
                    readout(ui, "Frames per second. Most phones film at 30.");
                });
            });
            s.footer(|ui| {
                let create =
                    sheet::button(ui, ButtonKind::Primary, None, "Create", 96.0, None, true);
                if create.clicked() {
                    choice = Some(NewProjectChoice::Create);
                }
                if sheet::button(ui, ButtonKind::Secondary, None, "Cancel", 80.0, None, true)
                    .clicked()
                {
                    choice = Some(NewProjectChoice::Cancel);
                }
            });
        });
    if choice.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
    {
        choice = Some(NewProjectChoice::Create);
    }
    if response.close && choice.is_none() {
        choice = Some(NewProjectChoice::Cancel);
    }
    choice
}

fn readout(ui: &mut egui::Ui, text: &str) {
    let p = theme::look(ui.ctx()).palette;
    ui.label(
        egui::RichText::new(text)
            .text_style(egui::TextStyle::Small)
            .color(p.text_tertiary),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rail_folds_at_960_and_hides_at_720() {
        assert_eq!(home_rail_width(1440.0), 200.0);
        assert_eq!(home_rail_width(960.0), 200.0);
        assert_eq!(home_rail_width(959.0), 56.0);
        assert_eq!(home_rail_width(720.0), 56.0);
        assert_eq!(home_rail_width(719.0), 0.0);
    }

    #[test]
    fn margins_follow_the_window() {
        assert_eq!(home_margins(1240.0, 860.0), (64.0, 86.0));
        assert_eq!(home_margins(320.0, 200.0), (24.0, 32.0));
        assert_eq!(home_margins(4000.0, 2000.0), (64.0, 96.0));
        assert_eq!(home_margins(800.0, 500.0), (50.0, 50.0));
    }

    #[test]
    fn the_grid_fills_the_column_with_cards_at_least_216_wide() {
        for width in [100.0, 216.0, 447.0, 448.0, 700.0, 1120.0] {
            let (cols, w, h) = home_grid(width);
            let used = cols as f32 * w + (cols - 1) as f32 * CARD_GAP;
            assert!((used - width.max(1.0)).abs() < 0.01, "{width}");
            assert!(cols == 1 || w >= CARD_MIN, "{width}: {w}");
            assert!((h - (w * 9.0 / 16.0 + CARD_TEXT)).abs() < 0.01);
        }
        assert_eq!(home_grid(1120.0).0, 4);
        assert_eq!(home_grid(447.0).0, 1);
        assert_eq!(home_grid(448.0).0, 2);
    }

    #[test]
    fn cards_say_shape_length_and_rate() {
        let mut e = RecentEntry {
            path: PathBuf::from("/a.freecut"),
            name: "a".into(),
            opened: 0,
            canvas: (1080, 1920),
            rate: (30_000, 1001),
            duration_ms: 42_000,
            found: true,
        };
        assert_eq!(card_details(&e), "9:16 · 1080p · 0:42 · 29.97 fps");
        e.duration_ms = 3_723_400;
        e.rate = (25, 1);
        assert_eq!(card_details(&e), "9:16 · 1080p · 1:02:03 · 25 fps");
        e.duration_ms = 0;
        assert_eq!(card_details(&e), "9:16 · 1080p · 0:00 · 25 fps");
    }

    /// UI-013: a project whose file is gone dims its cover, never its
    /// text: the details stay at `text.tertiary`, 4.5 : 1 on the card.
    #[test]
    fn a_missing_projects_card_keeps_its_text_contrast() {
        use crate::ui::drag::tests::Frames;

        let entry = RecentEntry {
            path: PathBuf::from("/gone.freecut"),
            name: "Old interview".into(),
            opened: 0,
            canvas: (1920, 1080),
            rate: (25, 1),
            duration_ms: 1_520_000,
            found: false,
        };
        let mut frames = Frames::new();
        let out = frames.run_full(Vec::new(), |ui| {
            let rect = Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(320.0, 244.0));
            project_card(ui, rect, &entry, None, 0, &mut Vec::new());
        });
        let p = theme::look(&frames.ctx).palette;
        let mut texts = 0;
        for clipped in &out.shapes {
            let egui::Shape::Text(text) = &clipped.shape else {
                continue;
            };
            if text.galley.job.text.chars().any(char::is_alphanumeric) && text.pos.y > 190.0 {
                texts += 1;
                // Drawn opaque: a dimming painter would have faded it.
                let colour = text.fallback_color;
                assert_eq!(colour.a(), 255, "{:?}", text.galley.job.text);
                assert_eq!(text.opacity_factor, 1.0, "{:?}", text.galley.job.text);
                let (a, b) = (theme::luminance(colour), theme::luminance(p.bg_panel));
                let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(ratio >= 4.5, "{:?}: {ratio}", text.galley.job.text);
            }
        }
        assert_eq!(texts, 3, "the name and two lines of details");
    }

    #[test]
    fn the_forget_button_sits_in_the_posters_corner() {
        let card = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 244.0));
        let poster = poster_rect(card);
        assert_eq!(poster.height(), 180.0);
        let x = forget_rect(poster);
        assert_eq!(x.right(), 314.0);
        assert_eq!(x.top(), 6.0);
        assert_eq!(x.width(), 24.0);
    }
}
