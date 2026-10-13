//! The timeline toolbar (UI-011, ui-concept §13.26): a 40 pt strip (34
//! Compact) of icon buttons in groups split by 1 × 18 pt rules.
//!
//! Left: the **Add** `+` menu | the **tool button** (Select / Blade / Hand,
//! TL-024) | undo, redo | split, cut away before / after the playhead
//! (TL-026), delete, the selection count | the **marker** split button
//! (TL-037) | the Mixer. Right: **Record voiceover** (AU-027) | the magnetic
//! main track, **Follow main track** (TL-029, with TL-057 in its menu),
//! snapping, **hover skim** (TL-014) | zoom out, the zoom slider, zoom in,
//! zoom to fit | the `⋯` menu.
//!
//! Toggles show the *checked* chip, never an accent fill; disabled buttons
//! say why. When space is short the groups fold into `⋯` in UI-011's order
//! ([`fold_toolbar`]): the Mixer (in the clip-action slot) first, then the
//! marker button, the magnet and *Follow main track*, snapping and skim,
//! the two cut-away buttons, the zoom slider, and the Add menu. The tool
//! button, undo / redo, split, delete and record never fold. A folded split
//! button becomes a submenu with its main command first.
//!
//! Every button runs a command the app already has: the toolbar reports
//! [`ToolAction`]s for the app's commands, and runs the timeline's own ones
//! (tools, markers, sequence flags, zoom) through the same undoable edits
//! as the menus and keys.

use eframe::egui::{self, Align, Align2, Color32, Layout, Rect, Sense, Stroke, UiBuilder, Vec2};
use freecut_timeline::edit::{AddTrackReq, EditRequest, MarkerSelector, TrackPosition};
use freecut_timeline::history::Coalesce;
use freecut_timeline::model::{
    FollowKind, FollowKinds, MarkerRef, PaletteColor, Sequence, TrackKind,
};
use freecut_timeline::time::{Tick, Timecode};

use super::timeline::{TimelineOutput, TimelineView};
use super::timeline_layout::{MAX_PPS, MIN_PPS, ZOOM_STEP};
use super::timeline_tools::TimelineTool;
use crate::session::{Project, op_name};
use crate::theme::{self, Type, fonts, icons, surface};
use crate::timeline_commands::{
    MarkerTarget, follow_kind_label, follow_summary, next_marker_color,
};
use crate::widgets::icon_button::IconButton;

/// What a toolbar button asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAction {
    Undo,
    Redo,
    Split,
    Delete,
    AddText,
    ToggleMixer,
    /// *Import media…* from the Add menu (PM-030).
    Import,
    /// Cut away the part before the playhead (TL-026, `Q`); `Shift` held
    /// ripples non-main tracks.
    CutBefore {
        ripple_others: bool,
    },
    /// Cut away the part after the playhead (`W`).
    CutAfter {
        ripple_others: bool,
    },
    /// *Record voiceover*: opens the recorder panel (AU-027), or stops the
    /// take that is running.
    Record,
}

/// What the Record button shows (AU-027, UI-011).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordButton {
    /// A take is counting down or recording: the button takes the
    /// `danger` fill with this elapsed time beside it, and a click stops.
    pub running: Option<String>,
    /// The recorder panel is open.
    pub open: bool,
}

/// Room between buttons, around the toolbar, and a group rule's slot.
const GAP: f32 = 4.0;
const PAD: f32 = 8.0;
const RULE_SLOT: f32 = 9.0;
/// The zoom slider's width.
const SLIDER_W: f32 = 96.0;
/// A split button's parts (UI-011): the icon part and the chevron.
pub const SPLIT_ICON_W: f32 = 28.0;
pub const SPLIT_CHEVRON_W: f32 = 12.0;
const SPLIT_W: f32 = SPLIT_ICON_W + SPLIT_CHEVRON_W;

/// One control of the toolbar, in UI-011's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarItem {
    Add,
    Tool,
    Undo,
    Redo,
    Split,
    CutBefore,
    CutAfter,
    Delete,
    Marker,
    /// The Mixer toggle, in the clip-action group's slot.
    Mixer,
    Record,
    Magnet,
    Follow,
    Snap,
    Skim,
    ZoomOut,
    ZoomSlider,
    ZoomIn,
    ZoomFit,
    More,
}

/// Which foldable groups show; the rest are in the `⋯` menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    pub add: bool,
    pub cut_away: bool,
    pub marker: bool,
    pub clip_actions: bool,
    pub magnet_follow: bool,
    pub snap_skim: bool,
    pub slider: bool,
}

impl Fold {
    pub const ALL: Fold = Fold {
        add: true,
        cut_away: true,
        marker: true,
        clip_actions: true,
        magnet_follow: true,
        snap_skim: true,
        slider: true,
    };

    /// Whether anything went into the `⋯` menu (the slider shrinking to
    /// its buttons doesn't count: zoom stays on the strip).
    pub fn folded(self) -> bool {
        !(self.add
            && self.cut_away
            && self.marker
            && self.clip_actions
            && self.magnet_follow
            && self.snap_skim)
    }

    /// UI-011's fold sequence, from everything shown to the least.
    pub fn steps() -> [Fold; 8] {
        let a = Fold::ALL;
        let s1 = Fold {
            clip_actions: false,
            ..a
        };
        let s2 = Fold {
            marker: false,
            ..s1
        };
        let s3 = Fold {
            magnet_follow: false,
            ..s2
        };
        let s4 = Fold {
            snap_skim: false,
            ..s3
        };
        let s5 = Fold {
            cut_away: false,
            ..s4
        };
        let s6 = Fold {
            slider: false,
            ..s5
        };
        let s7 = Fold { add: false, ..s6 };
        [a, s1, s2, s3, s4, s5, s6, s7]
    }

    /// The groups shown, left half then right half, as item lists.
    fn groups(self) -> Vec<Vec<ToolbarItem>> {
        use ToolbarItem as I;
        let mut g: Vec<Vec<ToolbarItem>> = Vec::new();
        if self.add {
            g.push(vec![I::Add]);
        }
        g.push(vec![I::Tool]);
        g.push(vec![I::Undo, I::Redo]);
        let mut edit = vec![I::Split];
        if self.cut_away {
            edit.extend([I::CutBefore, I::CutAfter]);
        }
        edit.push(I::Delete);
        g.push(edit);
        if self.marker {
            g.push(vec![I::Marker]);
        }
        if self.clip_actions {
            g.push(vec![I::Mixer]);
        }
        g.push(vec![I::Record]);
        let mut toggles = Vec::new();
        if self.magnet_follow {
            toggles.extend([I::Magnet, I::Follow]);
        }
        if self.snap_skim {
            toggles.extend([I::Snap, I::Skim]);
        }
        if !toggles.is_empty() {
            g.push(toggles);
        }
        let mut zoom = vec![I::ZoomOut];
        if self.slider {
            zoom.push(I::ZoomSlider);
        }
        zoom.extend([I::ZoomIn, I::ZoomFit]);
        g.push(zoom);
        if self.folded() {
            g.push(vec![I::More]);
        }
        g
    }

    /// Every item shown, in order.
    #[cfg(test)]
    pub fn items(self) -> Vec<ToolbarItem> {
        self.groups().into_iter().flatten().collect()
    }
}

/// The widths the toolbar is measured with: a chip `chip` pt wide, and the
/// selection count's label (0 when hidden).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Widths {
    pub chip: f32,
    pub count: f32,
}

impl Widths {
    fn item(self, item: ToolbarItem) -> f32 {
        match item {
            ToolbarItem::Tool | ToolbarItem::Marker | ToolbarItem::Follow | ToolbarItem::Skim => {
                SPLIT_W
            }
            ToolbarItem::ZoomSlider => SLIDER_W,
            _ => self.chip,
        }
    }

    /// The toolbar's width with `fold`.
    pub fn needed(self, fold: Fold) -> f32 {
        let groups = fold.groups();
        let mut total = 2.0 * PAD;
        for group in &groups {
            let n = group.len() as f32;
            total += group.iter().map(|i| self.item(*i)).sum::<f32>() + (n - 1.0) * GAP;
            if group.contains(&ToolbarItem::Delete) && self.count > 0.0 {
                total += GAP + self.count;
            }
        }
        // A rule between groups, and room either side of it.
        total += (groups.len() as f32 - 1.0) * (RULE_SLOT + 2.0 * GAP);
        total
    }
}

/// What fits in `width`: UI-011's fold sequence, stopping at the first
/// step that fits (the last one when nothing does).
pub fn fold_toolbar(width: f32, widths: Widths) -> Fold {
    let steps = Fold::steps();
    steps
        .into_iter()
        .find(|f| widths.needed(*f) <= width)
        .unwrap_or(steps[steps.len() - 1])
}

/// The zoom slider's position (0..=1) for `pps`: logarithmic, so each
/// step of the slider zooms by the same factor.
pub fn slider_of_zoom(pps: f32) -> f32 {
    let span = (MAX_PPS / MIN_PPS).ln();
    ((pps.clamp(MIN_PPS, MAX_PPS) / MIN_PPS).ln() / span).clamp(0.0, 1.0)
}

/// The inverse of [`slider_of_zoom`].
pub fn zoom_of_slider(t: f32) -> f32 {
    let span = (MAX_PPS / MIN_PPS).ln();
    (MIN_PPS * (t.clamp(0.0, 1.0) * span).exp()).clamp(MIN_PPS, MAX_PPS)
}

/// The zoom that fits `duration_secs` (with a little room after it) into
/// `width` points of lanes.
pub fn fit_zoom(duration_secs: f64, width: f32) -> f32 {
    if duration_secs <= 0.0 || width <= 0.0 {
        return 60.0_f32.clamp(MIN_PPS, MAX_PPS);
    }
    let pps = f64::from(width) / (duration_secs * 1.05);
    (pps as f32).clamp(MIN_PPS, MAX_PPS)
}

/// What the toolbar shows that the app knows: availability and the Mixer.
#[derive(Debug, Clone)]
pub struct ToolbarInput {
    /// The lanes' width, for *Zoom to fit*.
    pub lanes_width: f32,
    pub mixer_open: bool,
    /// Whether Delete has something to delete (clips, markers or volume
    /// keys).
    pub can_delete: bool,
}

/// A plain key in a tooltip: "(Q)".
fn key(k: &str) -> String {
    format!("({k})")
}

/// The platform's spelling of `mods`+`k` ("⇧⌘M" / "Ctrl+Shift+M").
fn chord(shift: bool, cmd: bool, alt: bool, k: &str) -> String {
    if cfg!(target_os = "macos") {
        let mut s = String::new();
        if alt {
            s.push('⌥');
        }
        if shift {
            s.push('⇧');
        }
        if cmd {
            s.push('⌘');
        }
        s + k
    } else {
        let mut parts = Vec::new();
        if cmd {
            parts.push("Ctrl");
        }
        if shift {
            parts.push("Shift");
        }
        if alt {
            parts.push("Alt");
        }
        parts.push(k);
        parts.join("+")
    }
}

/// Shows the toolbar in `rect`.
pub fn show(
    ui: &mut egui::Ui,
    rect: Rect,
    project: &mut Project,
    view: &mut TimelineView,
    input: &ToolbarInput,
    out: &mut TimelineOutput,
) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, p.border_subtle),
    );
    let selected = project.selection.len();
    let count = (selected > 1).then(|| format!("{selected} selected"));
    let micro = Type::Caption.style(ui.style()).resolve(ui.style());
    let count_w = count.as_ref().map_or(0.0, |c| {
        ui.painter()
            .layout_no_wrap(c.clone(), micro.clone(), p.accent_text)
            .size()
            .x
    });
    // The elapsed time beside a running Record button takes room too.
    let record_w = view.record.running.as_ref().map_or(0.0, |t| {
        GAP + ui
            .painter()
            .layout_no_wrap(t.clone(), micro.clone(), p.danger)
            .size()
            .x
    });
    let fold = fold_toolbar(
        rect.width(),
        Widths {
            chip: look.metrics.icon_chip,
            count: count_w + record_w,
        },
    );
    let history = project.editor.history();
    let undo = (
        history.undo_label().map(|l| op_name(l.op)),
        history.redo_label().map(|l| op_name(l.op)),
    );
    let split = split_available(project);
    let duration = project
        .doc()
        .sequence(project.doc().root())
        .map_or(0.0, |s| s.duration().as_secs_f64());
    let shift = ui.input(|i| i.modifiers.shift);

    let inner = rect.shrink2(Vec2::new(PAD, 0.0));
    let mut bar = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center))
            .id_salt("timeline-toolbar"),
    );
    let ui = &mut bar;
    ui.spacing_mut().item_spacing.x = GAP;

    if fold.add {
        let response = IconButton::new(icons::PLUS, "Add media, a track or a title").show(ui);
        egui::Popup::menu(&response).show(|ui| add_menu_items(ui, project, out));
        rule(ui);
    }
    tool_button(ui, view);
    rule(ui);
    let undo_tip = undo.0.as_ref().map_or_else(
        || "Nothing to undo".to_owned(),
        |l| format!("Undo {l} ({})", crate::app::shortcut_label("Z")),
    );
    if IconButton::new(icons::UNDO, &undo_tip)
        .enabled(undo.0.is_some())
        .show(ui)
        .clicked()
    {
        out.action = Some(ToolAction::Undo);
    }
    let redo_tip = undo.1.as_ref().map_or_else(
        || "Nothing to redo".to_owned(),
        |l| format!("Redo {l} ({})", chord(true, true, false, "Z")),
    );
    if IconButton::new(icons::REDO, &redo_tip)
        .enabled(undo.1.is_some())
        .show(ui)
        .clicked()
    {
        out.action = Some(ToolAction::Redo);
    }
    rule(ui);
    let split_tip = match split {
        Ok(()) => format!(
            "Split at the playhead ({})",
            crate::app::shortcut_label("B")
        ),
        Err(reason) => reason.to_owned(),
    };
    if IconButton::new(icons::SPLIT, &split_tip)
        .enabled(split.is_ok())
        .show(ui)
        .clicked()
    {
        out.action = Some(ToolAction::Split);
    }
    if fold.cut_away {
        let available = project.cut_available();
        for before in [true, false] {
            let tip = match available {
                Ok(()) if before => format!("Cut away the part before the playhead {}", key("Q")),
                Ok(()) => format!("Cut away the part after the playhead {}", key("W")),
                Err(reason) => reason.to_owned(),
            };
            if cut_away_button(ui, before, &tip, available.is_ok()).clicked() {
                out.action = Some(if before {
                    ToolAction::CutBefore {
                        ripple_others: shift,
                    }
                } else {
                    ToolAction::CutAfter {
                        ripple_others: shift,
                    }
                });
            }
        }
    }
    let delete_tip = if input.can_delete {
        "Delete the selection (Delete)"
    } else {
        "Select a clip"
    };
    if IconButton::new(icons::DELETE, delete_tip)
        .danger(true)
        .enabled(input.can_delete)
        .show(ui)
        .clicked()
    {
        out.action = Some(ToolAction::Delete);
    }
    if let Some(count) = count {
        ui.label(egui::RichText::new(count).font(micro).color(p.accent_text));
    }
    if fold.marker {
        rule(ui);
        marker_button(ui, project, view, out);
    }
    if fold.clip_actions {
        rule(ui);
        let mixer_tip = if input.mixer_open {
            "Hide the Mixer"
        } else {
            "Show the Mixer"
        };
        if IconButton::new(icons::MIXER, mixer_tip)
            .checked(input.mixer_open)
            .show(ui)
            .clicked()
        {
            out.action = Some(ToolAction::ToggleMixer);
        }
    }

    // The right end, laid out from the right edge inwards.
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        if fold.folded() {
            more_menu(ui, project, view, input, fold, out);
            rule(ui);
        }
        if IconButton::new(icons::ZOOM_FIT, "Zoom to fit the whole sequence")
            .show(ui)
            .clicked()
        {
            view.px_per_sec = fit_zoom(duration, input.lanes_width);
            view.scroll_secs = 0.0;
        }
        if IconButton::new(icons::ZOOM_IN, "Zoom in (+)")
            .enabled(view.px_per_sec < MAX_PPS)
            .show(ui)
            .clicked()
        {
            view.px_per_sec = (view.px_per_sec * ZOOM_STEP).min(MAX_PPS);
        }
        if fold.slider {
            zoom_slider(ui, &mut view.px_per_sec);
        }
        if IconButton::new(icons::ZOOM_OUT, "Zoom out (−)")
            .enabled(view.px_per_sec > MIN_PPS)
            .show(ui)
            .clicked()
        {
            view.px_per_sec = (view.px_per_sec / ZOOM_STEP).max(MIN_PPS);
        }
        if fold.magnet_follow || fold.snap_skim {
            rule(ui);
        }
        // Right to left: skim, snapping, follow, magnet.
        if fold.snap_skim {
            skim_button(ui, view);
            let snap_tip = if view.snapping {
                "Snapping is on: clips snap to edges, markers and the playhead (N)"
            } else {
                "Snapping is off (N)"
            };
            if IconButton::new(icons::SNAP, snap_tip)
                .checked(view.snapping)
                .show(ui)
                .clicked()
            {
                view.snapping = !view.snapping;
            }
        }
        if fold.magnet_follow {
            follow_button(ui, project, view, out);
            magnet_button(ui, project, out);
        }
        rule(ui);
        if record_button(ui, view) {
            out.action = Some(ToolAction::Record);
        }
    });
}

/// A 1 × 18 pt `border.subtle` rule in a 9 pt slot.
fn rule(ui: &mut egui::Ui) {
    let p = theme::look(ui.ctx()).palette;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(RULE_SLOT - 2.0 * GAP, 18.0), Sense::hover());
    let x = rect.center().x.round() + 0.5;
    ui.painter().vline(
        x,
        egui::Rangef::new(rect.center().y - 9.0, rect.center().y + 9.0),
        Stroke::new(1.0, p.border_subtle),
    );
}

/// What a split button's two parts reported.
pub struct SplitResponse {
    pub main: egui::Response,
    pub menu: egui::Response,
}

/// UI-011's split button: a 28 pt icon part and a 12 pt chevron sharing
/// one chip. Each part has its own hover and tooltip; *checked* covers
/// both; `dot` puts a small accent dot in the icon part's corner.
pub struct SplitButton<'a> {
    pub icon: &'a str,
    pub tooltip: &'a str,
    pub menu_tooltip: &'a str,
    pub checked: bool,
    pub enabled: bool,
    pub dot: bool,
}

impl SplitButton<'_> {
    pub fn show(self, ui: &mut egui::Ui) -> SplitResponse {
        let salt = self.tooltip.to_owned();
        self.show_with_ids(ui, &salt)
    }

    /// As [`SplitButton::show`], with the parts' ids made from `salt`
    /// rather than the tooltip, for a button whose tooltip changes with
    /// its state (its menu then stays open across the change).
    pub fn show_with_ids(self, ui: &mut egui::Ui, salt: &str) -> SplitResponse {
        let look = theme::look(ui.ctx());
        let p = look.palette;
        let h = look.metrics.icon_chip;
        let enabled = self.enabled && ui.is_enabled();
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(SPLIT_W, h), Sense::hover());
        let icon_rect = Rect::from_min_size(rect.min, Vec2::new(SPLIT_ICON_W, h));
        let chevron_rect = Rect::from_min_max(
            egui::pos2(icon_rect.right(), rect.top()),
            rect.right_bottom(),
        );
        let main = ui.interact(icon_rect, ui.id().with((salt, "main")), sense);
        let menu = ui.interact(chevron_rect, ui.id().with((salt, "menu")), sense);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };
            if self.checked {
                painter.rect_filled(rect, theme::R_SM, dim(p.bg_well));
            }
            for (part, r, corners) in [
                (
                    &main,
                    icon_rect,
                    egui::CornerRadius {
                        nw: 6,
                        sw: 6,
                        ne: 0,
                        se: 0,
                    },
                ),
                (
                    &menu,
                    chevron_rect,
                    egui::CornerRadius {
                        nw: 0,
                        sw: 0,
                        ne: 6,
                        se: 6,
                    },
                ),
            ] {
                if enabled && part.hovered() {
                    let fill = if part.is_pointer_button_down_on() {
                        p.bg_active
                    } else if self.checked {
                        p.bg_hover
                    } else {
                        p.bg_field
                    };
                    painter.rect_filled(r, corners, fill);
                }
            }
            if enabled && (main.hovered() || menu.hovered()) {
                painter.vline(
                    icon_rect.right(),
                    egui::Rangef::new(rect.top() + 5.0, rect.bottom() - 5.0),
                    Stroke::new(1.0, p.border_subtle),
                );
            }
            crate::widgets::control_outline(ui, rect, theme::R_SM);
            let glyph = if self.checked {
                p.accent_text
            } else if enabled && main.hovered() {
                p.text_primary
            } else {
                p.text_secondary
            };
            painter.text(
                icon_rect.center(),
                Align2::CENTER_CENTER,
                self.icon,
                fonts::icon_font(ui.ctx(), 16.0, self.checked),
                dim(glyph),
            );
            let caret = if enabled && menu.hovered() {
                p.text_primary
            } else {
                p.text_tertiary
            };
            painter.text(
                chevron_rect.center() - Vec2::new(1.0, 0.0),
                Align2::CENTER_CENTER,
                icons::CARET_DOWN,
                fonts::icon_font(ui.ctx(), 10.0, false),
                dim(caret),
            );
            if self.dot {
                painter.circle_filled(
                    icon_rect.right_top() + Vec2::new(-5.0, 5.0),
                    2.5,
                    dim(p.accent),
                );
            }
            for part in [&main, &menu] {
                if crate::widgets::icon_button::keyboard_focus(ui, part) {
                    crate::widgets::focus_ring(ui, part.rect, theme::R_SM);
                }
            }
        }
        let (label, checked) = (self.tooltip.to_owned(), self.checked);
        main.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, checked, &label)
        });
        let menu_label = self.menu_tooltip.to_owned();
        menu.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &menu_label)
        });
        SplitResponse {
            main: main.on_hover_text(self.tooltip),
            menu: menu.on_hover_text(self.menu_tooltip),
        }
    }
}

/// The tool button (TL-024): the active tool's icon; a click switches
/// Select ↔ the last other tool; the chevron lists the three tools.
fn tool_button(ui: &mut egui::Ui, view: &mut TimelineView) {
    let tip = view.tool.tooltip();
    let r = SplitButton {
        icon: view.tool.active.icon(),
        tooltip: &tip,
        menu_tooltip: "Choose a tool",
        checked: view.tool.active != TimelineTool::Select,
        enabled: true,
        dot: false,
    }
    .show(ui);
    if r.main.clicked() {
        view.tool.toggle();
    }
    egui::Popup::menu(&r.menu).show(|ui| tool_items(ui, view));
}

fn tool_items(ui: &mut egui::Ui, view: &mut TimelineView) {
    for tool in TimelineTool::ALL {
        let label = crate::widgets::icon_then_text(ui, tool.icon(), tool.label());
        if ui
            .add(
                egui::Button::new(label)
                    .shortcut_text(tool.key())
                    .selected(view.tool.active == tool),
            )
            .clicked()
        {
            view.tool.set(tool);
            ui.close();
        }
    }
}

/// A cut-away button (TL-026): FreeCut's own glyph of a clip cut at the
/// playhead with the side that goes drawn faint.
fn cut_away_button(ui: &mut egui::Ui, before: bool, tip: &str, enabled: bool) -> egui::Response {
    let response = IconButton::new("", tip).enabled(enabled).show(ui);
    let p = theme::look(ui.ctx()).palette;
    let mut color = if enabled && response.hovered() {
        p.text_primary
    } else {
        p.text_secondary
    };
    if !enabled {
        color = color.gamma_multiply(0.4);
    }
    paint_cut_glyph(ui.painter(), response.rect.center(), before, color);
    response
}

/// The cut-away glyph centred on `c`: a 14 × 9 pt clip and a playhead
/// line; the part that goes is a faint outline, the part that stays is
/// filled.
pub fn paint_cut_glyph(painter: &egui::Painter, c: egui::Pos2, before: bool, color: Color32) {
    let clip = Rect::from_center_size(c, Vec2::new(14.0, 9.0));
    let (gone, kept) = if before {
        (
            Rect::from_min_max(clip.min, egui::pos2(c.x - 1.0, clip.bottom())),
            Rect::from_min_max(egui::pos2(c.x + 1.0, clip.top()), clip.max),
        )
    } else {
        (
            Rect::from_min_max(egui::pos2(c.x + 1.0, clip.top()), clip.max),
            Rect::from_min_max(clip.min, egui::pos2(c.x - 1.0, clip.bottom())),
        )
    };
    painter.rect_stroke(
        gone,
        1.0,
        Stroke::new(1.0, color.gamma_multiply(0.45)),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(kept, 1.0, color);
    painter.vline(
        c.x,
        egui::Rangef::new(clip.top() - 3.0, clip.bottom() + 3.0),
        Stroke::new(1.5, color),
    );
}

/// The marker split button (TL-037): the flag adds a marker at the
/// playhead; the chevron holds next colour, navigation, the bulk deletes
/// and the list of markers.
fn marker_button(
    ui: &mut egui::Ui,
    project: &mut Project,
    view: &mut TimelineView,
    out: &mut TimelineOutput,
) {
    let tip = match project.marker_target() {
        MarkerTarget::Clip(_) => "Add a marker to the clip (M)",
        MarkerTarget::Timeline => "Add a marker to the timeline (M)",
    };
    let r = SplitButton {
        icon: icons::FLAG,
        tooltip: tip,
        menu_tooltip: "Marker options",
        checked: false,
        enabled: true,
        dot: false,
    }
    .show(ui);
    if r.main.clicked() {
        add_marker(project, view, None, out);
    }
    egui::Popup::menu(&r.menu).show(|ui| marker_items(ui, project, view, out));
}

/// Adds a marker at the playhead in `color` (Blue when `None`) and
/// remembers the colour for *next colour*.
pub fn add_marker(
    project: &mut Project,
    view: &mut TimelineView,
    color: Option<PaletteColor>,
    out: &mut TimelineOutput,
) {
    match project.add_marker(color) {
        Ok(_) => view.last_marker_color = Some(color.unwrap_or_default()),
        Err(e) => out.status = Some(e),
    }
}

/// The marker menu's items (the chevron, and the folded `⋯` submenu).
fn marker_items(
    ui: &mut egui::Ui,
    project: &mut Project,
    view: &mut TimelineView,
    out: &mut TimelineOutput,
) {
    let item = |ui: &mut egui::Ui, label: &str, keys: String, enabled: bool| {
        ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys))
            .clicked()
    };
    if item(ui, "Add marker", "M".to_owned(), true) {
        add_marker(project, view, None, out);
        ui.close();
    }
    if item(
        ui,
        "Add marker with next colour",
        chord(false, false, true, "M"),
        true,
    ) {
        let color = next_marker_color(view.last_marker_color);
        add_marker(project, view, Some(color), out);
        ui.close();
    }
    if item(ui, "Previous marker", chord(true, true, false, "M"), true) {
        jump(project, false, out);
        ui.close();
    }
    if item(ui, "Next marker", chord(true, false, false, "M"), true) {
        jump(project, true, out);
        ui.close();
    }
    ui.separator();
    let on_clip = project.clip_with_markers();
    if ui
        .add_enabled(
            on_clip.is_some(),
            egui::Button::new("Delete markers on this clip"),
        )
        .clicked()
        && let Some(clip) = on_clip
    {
        if let Err(e) = project.delete_markers(MarkerSelector::OnClip(clip)) {
            out.status = Some(e);
        }
        ui.close();
    }
    if ui
        .add_enabled(
            project.has_timeline_markers(),
            egui::Button::new("Delete all timeline markers"),
        )
        .clicked()
    {
        if let Err(e) = project.delete_markers(MarkerSelector::AllTimeline) {
            out.status = Some(e);
        }
        ui.close();
    }
    marker_list(ui, project, out);
}

/// Every marker of the sequence by time, each a row that jumps there and
/// selects it.
fn marker_list(ui: &mut egui::Ui, project: &mut Project, out: &mut TimelineOutput) {
    let doc = project.doc().clone();
    let Some(seq) = doc.sequence(doc.root()) else {
        return;
    };
    let rows = listed_markers(seq);
    if rows.is_empty() {
        return;
    }
    ui.separator();
    let p = theme::look(ui.ctx()).palette;
    ui.label(
        egui::RichText::new("Markers")
            .small()
            .color(p.text_tertiary),
    );
    let rate = doc.settings().frame_rate;
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .show(ui, |ui| {
            for (t, r, name, color) in rows {
                let tc = Timecode::from_tick(t, rate, false)
                    .map_or_else(|_| String::new(), |tc| tc.to_string());
                let mut label = crate::widgets::icon_then_text(ui, icons::FLAG, &name);
                for section in &mut label.sections {
                    section.format.color = surface::palette(color);
                }
                let selected = project.markers.contains(&r);
                if ui
                    .add(
                        egui::Button::new(label)
                            .shortcut_text(tc)
                            .selected(selected),
                    )
                    .clicked()
                {
                    project.playhead = t;
                    project.markers = std::collections::BTreeSet::from([r]);
                    out.seeked = true;
                    ui.close();
                }
            }
        });
}

/// The markers a list shows: timeline markers and the shown clip markers,
/// by sequence time.
pub fn listed_markers(seq: &Sequence) -> Vec<(Tick, MarkerRef, String, PaletteColor)> {
    let mut rows: Vec<(Tick, MarkerRef, String, PaletteColor)> = seq
        .markers
        .iter()
        .map(|m| {
            (
                m.at.start(),
                MarkerRef::Timeline(m.id),
                m.name.clone(),
                m.color,
            )
        })
        .collect();
    for (_, track) in seq.tracks() {
        for clip in track.clips() {
            for m in &clip.markers {
                if let Some(t) = clip.marker_start(m) {
                    rows.push((t, MarkerRef::Clip(clip.id, m.id), m.name.clone(), m.color));
                }
            }
        }
    }
    rows.sort_by_key(|(t, r, ..)| (*t, *r));
    rows
}

fn jump(project: &mut Project, forward: bool, out: &mut TimelineOutput) {
    match project.jump_to_marker(forward) {
        Ok(()) => out.seeked = true,
        Err(e) => out.status = Some(e),
    }
}

/// Record voiceover (AU-027, UI-011): opens the recorder panel; while a
/// take counts down or records it has the `danger` fill with the elapsed
/// time beside it, and a click stops the take. Returns whether it was
/// clicked. Laid out right to left: the time first, then the button.
fn record_button(ui: &mut egui::Ui, view: &mut TimelineView) -> bool {
    let p = theme::look(ui.ctx()).palette;
    if let Some(elapsed) = &view.record.running {
        let micro = Type::Caption.style(ui.style());
        ui.label(
            egui::RichText::new(elapsed)
                .font(micro.resolve(ui.style()))
                .color(p.danger),
        );
    }
    let tip = if view.record.running.is_some() {
        "Stop recording"
    } else if view.record.open {
        "Record voiceover: the recorder is open"
    } else {
        "Record voiceover…"
    };
    let fill = ui.painter().add(egui::Shape::Noop);
    let response = IconButton::new(icons::MICROPHONE, tip)
        .checked(view.record.open && view.record.running.is_none())
        .show(ui);
    if view.record.running.is_some() {
        ui.painter().set(
            fill,
            egui::Shape::rect_filled(response.rect, theme::R_SM, p.danger.gamma_multiply(0.55)),
        );
    }
    view.record_button = response.rect;
    response.clicked()
}

fn magnet_button(ui: &mut egui::Ui, project: &mut Project, out: &mut TimelineOutput) {
    let magnet = project.flags().main_magnet;
    let tip = if magnet {
        "Magnetic main track is on: the main track closes its gaps (Shift+N)"
    } else {
        "Magnetic main track is off (Shift+N)"
    };
    if IconButton::new(icons::MAGNET, tip)
        .checked(magnet)
        .show(ui)
        .clicked()
    {
        toggle_magnet(project, out);
    }
}

/// The Timeline menu's *Magnetic main track*: one undoable edit.
pub fn toggle_magnet(project: &mut Project, out: &mut TimelineOutput) {
    let req = EditRequest::SetMagnet {
        seq: project.doc().root(),
        on: !project.flags().main_magnet,
    };
    if let Err(e) = project.apply(&req, Coalesce::Never) {
        out.status = Some(e);
    }
}

/// *Follow main track* (TL-029): the icon part toggles the flag; the
/// chevron holds *Item kinds* and TL-057's *Select linked clips together*.
fn follow_button(
    ui: &mut egui::Ui,
    project: &mut Project,
    view: &mut TimelineView,
    out: &mut TimelineOutput,
) {
    let flags = project.flags();
    let tip = if flags.follow_main {
        format!(
            "Follow main track is on: items above follow the main-track clip under their start. \
             Kinds: {} (Alt+N)",
            follow_summary(flags.follow_kinds)
        )
    } else {
        "Follow main track is off: items stay where they are (Alt+N)".to_owned()
    };
    let r = SplitButton {
        icon: icons::FOLLOW,
        tooltip: &tip,
        menu_tooltip: "Follow main track options",
        checked: flags.follows(),
        enabled: true,
        dot: flags.follow_main
            && !flags.follow_kinds.is_empty()
            && flags.follow_kinds != FollowKinds::ALL,
    }
    .show(ui);
    if r.main.clicked()
        && let Err(e) = project.toggle_follow_main()
    {
        out.status = Some(e);
    }
    egui::Popup::menu(&r.menu)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| follow_items(ui, project, view, out));
}

/// The *Follow main track* menu: *Item kinds* (one checkbox per kind, All,
/// None) and *Select linked clips together*.
pub fn follow_items(
    ui: &mut egui::Ui,
    project: &mut Project,
    view: &mut TimelineView,
    out: &mut TimelineOutput,
) {
    let flags = project.flags();
    let mut on = flags.follow_main;
    if ui
        .add(egui::Checkbox::new(&mut on, "Follow main track"))
        .clicked()
        && let Err(e) = project.toggle_follow_main()
    {
        out.status = Some(e);
    }
    ui.menu_button("Item kinds", |ui| {
        let kinds = project.flags().follow_kinds;
        let mut set = |project: &mut Project, k: FollowKinds| {
            if let Err(e) = project.set_follow_kinds(k) {
                out.status = Some(e);
            }
        };
        ui.horizontal(|ui| {
            if ui
                .add_enabled(kinds != FollowKinds::ALL, egui::Button::new("All"))
                .clicked()
            {
                set(project, FollowKinds::ALL);
            }
            if ui
                .add_enabled(!kinds.is_empty(), egui::Button::new("None"))
                .clicked()
            {
                set(project, FollowKinds::NONE);
            }
        });
        ui.separator();
        for kind in FollowKind::ALL {
            let mut checked = kinds.contains(kind);
            if ui.checkbox(&mut checked, follow_kind_label(kind)).changed() {
                set(project, kinds.with(kind, checked));
            }
        }
    });
    ui.separator();
    ui.checkbox(&mut view.linked_selection, "Select linked clips together");
}

/// Hover skim (TL-014): the icon part toggles Off ↔ the last on-mode
/// (what `S` does); the chevron picks a mode.
fn skim_button(ui: &mut egui::Ui, view: &mut TimelineView) {
    let tip = view.skim.tooltip();
    let r = SplitButton {
        icon: icons::SKIM,
        tooltip: &tip,
        menu_tooltip: "Hover skim options",
        checked: view.skim.is_on(),
        enabled: true,
        dot: false,
    }
    .show_with_ids(ui, "hover-skim");
    if r.main.clicked() {
        view.skim.toggle();
    }
    egui::Popup::menu(&r.menu).show(|ui| skim_items(ui, view));
}

/// The hover-skim menu: one radio item per mode this build can show
/// (*Picture and sound* and the scrub-sound item wait for AU-016).
pub fn skim_items(ui: &mut egui::Ui, view: &mut TimelineView) {
    for mode in crate::skim::SkimMode::offered() {
        if ui.radio(view.skim.mode() == *mode, mode.label()).clicked() {
            view.skim.set_mode(*mode);
            ui.close();
        }
    }
}

/// The track kinds the Add menu offers (TL-006), in band order.
const NEW_TRACKS: [(TrackKind, &str, &str); 6] = [
    (TrackKind::Video, icons::VIDEO, "Video track"),
    (TrackKind::Text, icons::TEXT, "Text track"),
    (TrackKind::Sticker, icons::STICKER, "Sticker track"),
    (TrackKind::Effect, icons::EFFECT, "Effect track"),
    (TrackKind::Adjustment, icons::ADJUST, "Adjustment track"),
    (TrackKind::Audio, icons::WAVEFORM, "Audio track"),
];

/// The Add `+` menu: import, add a track of any kind, add a title, record
/// a voiceover. *Import folder…* waits for PM-032 (UI-030: items whose
/// feature isn't built are not shown).
fn add_menu_items(ui: &mut egui::Ui, project: &mut Project, out: &mut TimelineOutput) {
    let import = crate::widgets::icon_then_text(ui, icons::IMPORT, "Import media…");
    if ui
        .add(egui::Button::new(import).shortcut_text(crate::app::shortcut_label("I")))
        .clicked()
    {
        out.action = Some(ToolAction::Import);
        ui.close();
    }
    ui.menu_button("Add track", |ui| add_track_items(ui, project, out));
    let text = crate::widgets::icon_then_text(ui, icons::TEXT, "Add text at playhead");
    if ui
        .add(egui::Button::new(text).shortcut_text(crate::app::shortcut_label("T")))
        .clicked()
    {
        out.action = Some(ToolAction::AddText);
        ui.close();
    }
    let record = crate::widgets::icon_then_text(ui, icons::MICROPHONE, "Record voiceover…");
    if ui.button(record).clicked() {
        out.action = Some(ToolAction::Record);
        ui.close();
    }
}

fn add_track_items(ui: &mut egui::Ui, project: &mut Project, out: &mut TimelineOutput) {
    for (kind, icon, label) in NEW_TRACKS {
        if ui
            .button(crate::widgets::icon_then_text(ui, icon, label))
            .clicked()
        {
            let req = EditRequest::AddTrack(AddTrackReq {
                seq: project.doc().root(),
                kind,
                position: TrackPosition::Default,
            });
            if let Err(e) = project.apply(&req, Coalesce::Never) {
                out.status = Some(e);
            }
            ui.close();
        }
    }
}

/// The `⋯` menu with what folded away: toggles keep their check marks,
/// folded split buttons are submenus with their main command first.
fn more_menu(
    ui: &mut egui::Ui,
    project: &mut Project,
    view: &mut TimelineView,
    input: &ToolbarInput,
    fold: Fold,
    out: &mut TimelineOutput,
) {
    let response = IconButton::new(icons::DOTS_THREE, "More timeline commands").show(ui);
    egui::Popup::menu(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            if !fold.add {
                ui.menu_button("Add", |ui| add_menu_items(ui, project, out));
            }
            if !fold.cut_away {
                let available = project.cut_available();
                let shift = ui.input(|i| i.modifiers.shift);
                for (label, k, before) in [
                    ("Cut away before the playhead", "Q", true),
                    ("Cut away after the playhead", "W", false),
                ] {
                    let response = ui
                        .add_enabled(available.is_ok(), egui::Button::new(label).shortcut_text(k))
                        .on_disabled_hover_text(available.err().unwrap_or_default());
                    if response.clicked() {
                        out.action = Some(if before {
                            ToolAction::CutBefore {
                                ripple_others: shift,
                            }
                        } else {
                            ToolAction::CutAfter {
                                ripple_others: shift,
                            }
                        });
                        ui.close();
                    }
                }
            }
            if !fold.marker {
                ui.menu_button("Markers", |ui| marker_items(ui, project, view, out));
            }
            if !fold.clip_actions {
                let mut mixer = input.mixer_open;
                if ui.checkbox(&mut mixer, "Mixer").clicked() {
                    out.action = Some(ToolAction::ToggleMixer);
                    ui.close();
                }
            }
            if !fold.magnet_follow || !fold.snap_skim {
                ui.separator();
            }
            if !fold.magnet_follow {
                let mut on = project.flags().main_magnet;
                if ui.checkbox(&mut on, "Magnetic main track").clicked() {
                    toggle_magnet(project, out);
                }
                ui.menu_button("Follow main track", |ui| {
                    follow_items(ui, project, view, out);
                });
            }
            if !fold.snap_skim {
                ui.checkbox(&mut view.snapping, "Snapping");
                ui.menu_button("Hover skim", |ui| {
                    let mut on = view.skim.is_on();
                    if ui.checkbox(&mut on, "Hover skim (S)").clicked() {
                        view.skim.toggle();
                    }
                    ui.separator();
                    skim_items(ui, view);
                });
            }
        });
}

/// The 96 pt zoom slider: a rail and a round thumb, logarithmic.
fn zoom_slider(ui: &mut egui::Ui, pps: &mut f32) {
    let look = theme::look(ui.ctx());
    let p = look.palette;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(SLIDER_W, look.metrics.icon_chip),
        Sense::click_and_drag(),
    );
    let rail = Rect::from_center_size(rect.center(), Vec2::new(SLIDER_W - 12.0, 4.0));
    if (response.dragged() || response.clicked())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let t = (pos.x - rail.left()) / rail.width().max(1.0);
        *pps = zoom_of_slider(t);
    }
    let response = response.on_hover_text("Zoom");
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rail, theme::R_XS, p.bg_field);
        let t = slider_of_zoom(*pps);
        let x = rail.left() + t * rail.width();
        let filled = Rect::from_min_max(rail.min, egui::pos2(x, rail.bottom()));
        painter.rect_filled(filled, theme::R_XS, p.border_strong);
        let hot = response.hovered() || response.dragged();
        let r = if hot { 6.0 } else { 5.0 };
        painter.circle(
            egui::pos2(x, rail.center().y),
            r,
            p.text_primary,
            Stroke::new(1.0, egui::Color32::from_black_alpha(100)),
        );
        crate::widgets::control_outline(ui, rail, theme::R_XS);
    }
    response
        .widget_info(|| egui::WidgetInfo::slider(true, f64::from(slider_of_zoom(*pps)), "Zoom"));
}

/// Whether *Split at playhead* would cut anything: a selected clip (or,
/// with nothing selected, any clip) strictly around the playhead.
pub fn split_available(project: &Project) -> Result<(), &'static str> {
    let doc = project.doc();
    let Some(seq) = doc.sequence(doc.root()) else {
        return Err("Nothing to split");
    };
    let at = project.playhead;
    let inside = |c: &freecut_timeline::model::Clip| c.start() < at && at < c.end();
    let mut clips = seq.tracks().flat_map(|(_, t)| t.clips().iter());
    let any = if project.selection.is_empty() {
        clips.any(|c| inside(c))
    } else {
        clips.any(|c| project.selection.contains(&c.id) && inside(c))
    };
    if any {
        Ok(())
    } else if project.selection.is_empty() {
        Err("Move the playhead over a clip to split it")
    } else {
        Err("Move the playhead inside the selected clip to split it")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // exact geometry

    use super::*;

    const W: Widths = Widths {
        chip: 26.0,
        count: 0.0,
    };

    /// Whether `small` keeps `big`'s order (a subsequence of it).
    fn keeps_order(big: &[ToolbarItem], small: &[ToolbarItem]) -> bool {
        let mut it = big.iter();
        small
            .iter()
            .filter(|i| **i != ToolbarItem::More)
            .all(|s| it.any(|b| b == s))
    }

    #[test]
    fn groups_fold_in_ui011s_order_and_keep_their_order() {
        use ToolbarItem as I;
        let all = fold_toolbar(4_000.0, W);
        assert_eq!(all, Fold::ALL);
        assert!(!all.folded());
        let full = all.items();
        assert_eq!(
            full,
            vec![
                I::Add,
                I::Tool,
                I::Undo,
                I::Redo,
                I::Split,
                I::CutBefore,
                I::CutAfter,
                I::Delete,
                I::Marker,
                I::Mixer,
                I::Record,
                I::Magnet,
                I::Follow,
                I::Snap,
                I::Skim,
                I::ZoomOut,
                I::ZoomSlider,
                I::ZoomIn,
                I::ZoomFit,
            ]
        );
        // Shrinking a point at a time: each width shows the first fold step
        // that fits, so the steps come in UI-011's order, items only go
        // away, and the ones that always stay are always there.
        let steps = Fold::steps();
        let mut last_step = 0;
        let mut last = full.clone();
        let mut width = W.needed(Fold::ALL);
        while width > 0.0 {
            let fold = fold_toolbar(width, W);
            let step = steps.iter().position(|s| *s == fold).unwrap();
            assert!(step >= last_step, "{width}: back to step {step}");
            last_step = step;
            let items = fold.items();
            assert!(keeps_order(&full, &items), "{width}: {items:?}");
            assert!(keeps_order(&last, &items), "only removals at {width}");
            for always in [I::Tool, I::Undo, I::Redo, I::Split, I::Delete, I::Record] {
                assert!(items.contains(&always), "{always:?} at {width}");
            }
            assert_eq!(items.contains(&I::More), fold.folded());
            last = items;
            width -= 1.0;
        }
        assert_eq!(last_step, steps.len() - 1, "every step reached");
        // Each step folds exactly UI-011's next group.
        let gone: Vec<Vec<ToolbarItem>> = steps
            .windows(2)
            .map(|w| {
                let now = w[1].items();
                w[0].items()
                    .into_iter()
                    .filter(|i| !now.contains(i) && *i != I::More)
                    .collect()
            })
            .collect();
        assert_eq!(
            gone,
            vec![
                vec![I::Mixer],
                vec![I::Marker],
                vec![I::Magnet, I::Follow],
                vec![I::Snap, I::Skim],
                vec![I::CutBefore, I::CutAfter],
                vec![I::ZoomSlider],
                vec![I::Add],
            ]
        );
        // Nothing fits: the last step anyway.
        assert_eq!(fold_toolbar(10.0, W), steps[7]);
        // The selection count needs more room.
        let counted = Widths { count: 60.0, ..W };
        assert!(counted.needed(Fold::ALL) > W.needed(Fold::ALL));
        assert!(Fold::steps()[1].folded(), "the ⋯ menu appears");
        assert!(
            !Fold {
                slider: false,
                ..Fold::ALL
            }
            .folded()
        );
    }

    #[test]
    fn the_zoom_slider_is_logarithmic_and_round_trips() {
        assert_eq!(slider_of_zoom(MIN_PPS), 0.0);
        assert_eq!(slider_of_zoom(MAX_PPS), 1.0);
        assert_eq!(slider_of_zoom(1.0), 0.0, "clamped");
        for pps in [2.0, 7.5, 60.0, 400.0, 2_000.0] {
            assert!((zoom_of_slider(slider_of_zoom(pps)) - pps).abs() / pps < 1e-4);
        }
        // Equal steps zoom by equal factors.
        let a = zoom_of_slider(0.2) / zoom_of_slider(0.1);
        let b = zoom_of_slider(0.9) / zoom_of_slider(0.8);
        assert!((a - b).abs() < 1e-3);
    }

    #[test]
    fn zoom_to_fit_shows_the_whole_sequence() {
        let pps = fit_zoom(60.0, 1_000.0);
        assert!(60.0 * pps <= 1_000.0 && 60.0 * pps > 900.0);
        assert_eq!(fit_zoom(0.0, 1_000.0), 60.0);
        assert_eq!(fit_zoom(1.0e9, 1_000.0), MIN_PPS);
        assert_eq!(fit_zoom(0.001, 1_000.0), MAX_PPS);
    }

    #[test]
    fn chords_are_spelled_for_the_platform() {
        let c = chord(true, true, false, "M");
        if cfg!(target_os = "macos") {
            assert_eq!(c, "⇧⌘M");
            assert_eq!(chord(false, false, true, "M"), "⌥M");
        } else {
            assert_eq!(c, "Ctrl+Shift+M");
            assert_eq!(chord(false, false, true, "M"), "Alt+M");
        }
    }
}
