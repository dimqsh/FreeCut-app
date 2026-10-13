//! The Motion lens (ui-concept §6.3, §7.2; FX-018, FX-019, TX-021 …
//! TX-023, TX-056): "what moves, and when?".
//!
//! - **Animation** card: the In / Loop / Out timing strip (§13.11), whose
//!   joins drag the In and Out lengths (one undo entry per drag, `Esc`
//!   cancels); the slot picker; the slot's preset grid (§13.7) with still
//!   posters rendered off the UI thread ([`crate::posters`]) and, on
//!   hover, a short loop at a low frame rate, one tile at a time (Reduce
//!   motion keeps the still); then the chosen preset's settings: Pieces,
//!   Overlap, Order with its dice, Length or Pace, Easing, Direction,
//!   Travel and Intensity.
//! - **Keyframes** card: every animated property of the clip with its
//!   navigator, and each key with its easing (TR-031) and a remove button.
//!   Text clips also get **Reveal** here (TX-021), which has no other
//!   control.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, Sense, Stroke, StrokeKind, TextureHandle};
use freecut_timeline::edit::KeyEdit;
use freecut_timeline::model::{
    AnimOrder, AnimUnit, AnimationSpec, Clip, Direction, Interp, PerMille, PresetId, PropPath,
    PropValue, Slot, Travel,
};
use freecut_timeline::motion::{
    ChannelUnit, PresetCatalog, PresetManifest, PresetTable, count_units, effective_unit,
    pace_duration,
};
use freecut_timeline::time::{TICKS_PER_SECOND, Tick, Timecode};

use super::lens::{CardId, Detail, SelectionKind, widget_id};
use super::{Env, status};
use crate::anim::{self, PresetChoice, PresetGrids, UnavailableChoice};
use crate::posters::{PosterImage, PosterKey, PosterWorker, TILE_FPS, sample_times};
use crate::prop_keys;
use crate::theme::{self, Type, icons};
use crate::ui::props::{self, Edit, Merge, Steps};
use crate::ui::rows::{self, At, Numeric};
use crate::widgets::empty::empty_state;
use crate::widgets::icon_button;
use crate::widgets::row::PropRow;
use crate::widgets::segmented::{Segment, Segmented};
use crate::widgets::select::{Select, SelectOption};
use crate::widgets::timing_strip::TimingStrip;
use crate::widgets::toggle::toggle;

/// The Motion lens's state that outlives a frame.
#[derive(Default)]
pub struct MotionState {
    /// The slot whose grid shows; `None` is In.
    slot: Option<Slot>,
    /// The Pieces filter per slot (TX-022), when the user picked one.
    pieces: HashMap<Slot, AnimUnit>,
    /// The hovered tile and since when (UI-007's preview delay).
    hovered: Option<(PresetId, Slot, Instant)>,
    /// The tile pictures received so far.
    textures: HashMap<PosterKey, TextureHandle>,
    requested: HashSet<PosterKey>,
    /// Renders the tiles; `None` shows neutral placeholders.
    pub posters: Option<PosterWorker>,
    /// The presets the grids offer.
    pub presets: Arc<PresetTable>,
    /// The grids of `presets`, built once per catalogue ([`Self::grids`]).
    grids: Option<(Arc<PresetTable>, Arc<PresetGrids>)>,
    /// UI-043: tiles don't play on hover.
    pub reduce_motion: bool,
}

impl std::fmt::Debug for MotionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MotionState")
            .field("slot", &self.slot)
            .field("reduce_motion", &self.reduce_motion)
            .finish_non_exhaustive()
    }
}

impl MotionState {
    /// The slot whose grid shows.
    pub fn slot(&self) -> Slot {
        self.slot.unwrap_or(Slot::Entrance)
    }

    pub fn set_slot(&mut self, slot: Slot) {
        self.slot = Some(slot);
    }

    /// Sets the Pieces filter of `slot`, as its control does.
    #[cfg(test)]
    pub fn set_pieces(&mut self, slot: Slot, unit: AnimUnit) {
        self.pieces.insert(slot, unit);
    }

    /// Whether a hovered tile plays its loop: not under Reduce motion
    /// (UI-043), and only with a worker to render it.
    fn loops_play(&self) -> bool {
        !self.reduce_motion && self.posters.is_some()
    }

    /// The grids of the current catalogue: built on first use after the
    /// catalogue changes, never per repaint.
    pub fn grids(&mut self) -> Arc<PresetGrids> {
        if let Some((table, grids)) = &self.grids
            && Arc::ptr_eq(table, &self.presets)
        {
            return Arc::clone(grids);
        }
        let grids = Arc::new(PresetGrids::new(&self.presets));
        self.grids = Some((Arc::clone(&self.presets), Arc::clone(&grids)));
        grids
    }

    /// Takes the worker's finished pictures as textures.
    fn receive(&mut self, ctx: &egui::Context) {
        let Some(worker) = &self.posters else {
            return;
        };
        for PosterImage {
            key,
            width,
            height,
            rgba,
        } in worker.poll()
        {
            let size = [width as usize, height as usize];
            if rgba.len() != size[0] * size[1] * 4 {
                continue;
            }
            let image = ColorImage::from_rgba_unmultiplied(size, &rgba);
            let name = format!("freecut-tile-{}-{:?}-{}", key.preset, key.slot, key.frame);
            let tex = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
            self.textures.insert(key, tex);
        }
    }

    /// The picture to show for a tile: the hovered loop's frame when it
    /// has arrived, else the still. Asks the worker for what's missing.
    fn picture(&mut self, preset: &PresetId, slot: Slot, frame: u16) -> Option<&TextureHandle> {
        let still = PosterKey {
            preset: preset.clone(),
            slot,
            frame: 0,
        };
        if !self.requested.contains(&still)
            && let Some(w) = &self.posters
        {
            w.want_still(still.clone());
            self.requested.insert(still.clone());
        }
        let key = PosterKey {
            frame,
            ..still.clone()
        };
        if frame != 0 && self.textures.contains_key(&key) {
            return self.textures.get(&key);
        }
        self.textures.get(&still)
    }
}

/// The Motion lens's cards.
pub(super) fn cards(ui: &mut egui::Ui, env: &mut Env<'_, '_>) {
    let [id] = env.selection.as_slice() else {
        return;
    };
    let Some(clip) = env.doc.clip(*id).cloned() else {
        return;
    };
    env.motion.receive(ui.ctx());
    if anim::strip_layout(&clip).is_some() {
        animation_card(ui, env, &clip);
    }
    keyframes_card(ui, env, &clip);
}

fn slot_name(slot: Slot) -> &'static str {
    match slot {
        Slot::Entrance => "In",
        Slot::Continuous => "Loop",
        Slot::Exit => "Out",
    }
}

/// The Animation card's chip: "Fade · Wave · none".
fn summary(clip: &Clip) -> String {
    let name = |s: Slot| {
        clip.animations
            .get(s)
            .map_or_else(|| "none".to_owned(), |a| anim::preset_name(&a.preset))
    };
    format!(
        "In {} · Loop {} · Out {}",
        name(Slot::Entrance),
        name(Slot::Continuous),
        name(Slot::Exit)
    )
}

fn animation_card(ui: &mut egui::Ui, env: &mut Env<'_, '_>, clip: &Arc<Clip>) {
    let kind = env.kind;
    let mut memory = *env.workspace.card(kind, CardId::Animation);
    let shown = crate::widgets::card::PropertyCard::new(
        widget_id(kind, CardId::Animation, "card"),
        "Animation",
    )
    .summary(summary(clip))
    .show(ui, &mut memory, |ui, _| animation_body(ui, env, clip));
    *env.workspace.card(kind, CardId::Animation) = memory;
    status(env, shown.inner.flatten());
}

fn animation_body(ui: &mut egui::Ui, env: &mut Env<'_, '_>, clip: &Arc<Clip>) -> Option<String> {
    let layout = anim::strip_layout(clip)?;
    let kind = env.kind;
    let text = clip.content.text().is_some();
    let rate = env.doc.settings().frame_rate;
    let at = At::new(kind, CardId::Animation);
    let mut steps = Steps::default();
    let mut slot = env.motion.slot();

    // The timing strip.
    let names: Vec<Option<String>> = [Slot::Entrance, Slot::Continuous, Slot::Exit]
        .iter()
        .map(|s| {
            clip.animations
                .get(*s)
                .map(|a| anim::preset_name(&a.preset))
        })
        .collect();
    let strip = TimingStrip {
        id: widget_id(kind, CardId::Animation, "strip"),
        layout,
        selected: slot,
        names: [
            names[0].as_deref(),
            names[1].as_deref(),
            names[2].as_deref(),
        ],
    }
    .show(ui);
    if let Some(s) = strip.picked {
        slot = s;
    }
    if strip.cancelled {
        steps.cancel();
    }
    if let Some((s, x, response)) = strip.dragged {
        let specs = anim::join_drag(clip, &layout, s, x, rate);
        if specs.is_empty() {
            steps.finish(response);
        } else {
            // Every edit of the drag merges into its one undo entry.
            for (slot, spec) in specs {
                steps.edit(spec_edit(slot, spec), Merge::Widget(response.clone()));
            }
        }
    }
    for r in strip.idle {
        steps.finish(r);
    }
    let tc = |t: Tick| format!("{:.2} s", t.as_secs_f64());
    let any_caption = layout.in_set || layout.out_set || layout.shortened;
    if any_caption {
        ui.horizontal(|ui| {
            let p = theme::look(ui.ctx()).palette;
            let style = Type::Caption.style(ui.style());
            let caption = |s: String| {
                egui::RichText::new(s)
                    .text_style(style.clone())
                    .color(p.text_secondary)
            };
            if layout.in_set {
                ui.label(caption(format!("In {}", tc(layout.in_len))));
            }
            if layout.out_set {
                ui.label(caption(format!("Out {}", tc(layout.out_len))));
            }
            if layout.shortened {
                ui.label(caption("Shortened to fit the clip".to_owned()));
            }
        });
    }

    // The slot picker.
    let items = [
        Segment::new(None, slot_name(Slot::Entrance)),
        Segment::new(None, slot_name(Slot::Continuous)),
        Segment::new(None, slot_name(Slot::Exit)),
    ];
    let order = [Slot::Entrance, Slot::Continuous, Slot::Exit];
    let current = order.iter().position(|s| *s == slot).unwrap_or(0);
    let picked = Segmented::new(widget_id(kind, CardId::Animation, "slot"), &items, current)
        .fill_width()
        .show(ui)
        .picked;
    if let Some(s) = picked.and_then(|i| order.get(i)) {
        slot = *s;
    }
    env.motion.set_slot(slot);

    let spec = clip.animations.get(slot).cloned();
    let presets = Arc::clone(&env.motion.presets);
    let manifest = spec.as_ref().and_then(|s| presets.resolve(slot, &s.preset));
    let grids = env.motion.grids();

    // Pieces (TX-022): narrows the grid; sets the unit when the chosen
    // preset moves it.
    let mut pieces = AnimUnit::Block;
    if text {
        pieces =
            env.motion
                .pieces
                .get(&slot)
                .copied()
                .unwrap_or_else(|| match (&spec, &manifest) {
                    (Some(s), Some(m)) => effective_unit(m, s, true),
                    _ => AnimUnit::Block,
                });
        let units = [
            AnimUnit::Block,
            AnimUnit::Line,
            AnimUnit::Word,
            AnimUnit::Letter,
        ];
        let labels = [
            Segment::new(None, "Block"),
            Segment::new(None, "Lines"),
            Segment::new(None, "Words"),
            Segment::new(None, "Letters"),
        ];
        let mut row = PropRow::new(at.id("pieces"), "Pieces")
            .hint("Move the whole text, or line by line, word by word or letter by letter");
        if Segmented::new(at.id("pieces"), &labels, 0).natural(ui) > rows::form_room(ui) {
            row = row.stacked();
        }
        let row = row.show(ui, |ui| {
            let i = units.iter().position(|u| *u == pieces).unwrap_or(0);
            Segmented::new(at.id("pieces"), &labels, i)
                .fill_width()
                .show(ui)
        });
        if let Some(u) = row.inner.picked.and_then(|i| units.get(i)) {
            pieces = *u;
            env.motion.pieces.insert(slot, pieces);
            let changed = match (&spec, &manifest) {
                (Some(s), Some(m)) => anim::with_unit(s, m, pieces),
                _ => None,
            };
            steps.button(
                row.inner.response,
                changed.map(|s| Edit::Animation {
                    slot,
                    spec: Some(s),
                }),
            );
        }
        if let (Some(m), Some(_)) = (&manifest, &spec)
            && !anim::pieces_of(m).contains(&pieces)
        {
            let offered: Vec<&str> = anim::pieces_of(m).iter().map(|u| unit_name(*u)).collect();
            ui.weak(format!("This preset moves: {}", offered.join(", ")));
        }
    }

    // The preset grid, in FX-019's kinds on Loop.
    let visible: Vec<Group<'_>> = grids
        .groups(slot, text)
        .iter()
        .map(|g| Group {
            title: g.title(),
            choices: g
                .choices
                .iter()
                .filter(|c| !text || anim::pieces_of(&c.manifest).contains(&pieces))
                .collect(),
            unavailable: &g.unavailable,
        })
        .filter(|g| !g.choices.is_empty() || !g.unavailable.is_empty())
        .collect();
    // Choosing what is already chosen changes nothing (no undo entry).
    if let Some(choice) = grid(ui, env, slot, spec.as_ref(), &visible)
        && choice.map(|c| &c.id) != spec.as_ref().map(|s| &s.preset)
    {
        // TX-022: the Pieces shown sets the unit of the preset chosen (the
        // grid only offers presets that move it).
        let new = choice.map(|c| {
            let spec = anim::chosen_spec(c);
            if text {
                anim::with_unit(&spec, &c.manifest, pieces).unwrap_or(spec)
            } else {
                spec
            }
        });
        steps.edit(Edit::Animation { slot, spec: new }, Merge::Own);
    }
    if clip.duration() < Tick::from_raw(rate.frame_ticks().raw().saturating_mul(2)) {
        ui.weak("This clip is too short to show its animation");
    }

    // The chosen preset's settings.
    if let Some(spec) = &spec {
        match &manifest {
            Some(m) => settings_rows(ui, env, at, &mut steps, clip, slot, spec, m),
            None => {
                let text = format!("Unavailable animation ({})", spec.preset);
                let mut job = crate::widgets::icon_then_text(ui, icons::WARNING, &text);
                let p = theme::look(ui.ctx()).palette;
                for section in &mut job.sections {
                    section.format.color = p.text_primary;
                }
                ui.label(job);
                if ui.button("Remove").clicked() {
                    steps.edit(Edit::Animation { slot, spec: None }, Merge::Own);
                }
            }
        }
    }
    let clips = env.selection.clone();
    props::commit(env.project, env.drag, &clips, steps, props::held(ui))
}

fn unit_name(u: AnimUnit) -> &'static str {
    match u {
        AnimUnit::Block => "the whole text",
        AnimUnit::Line => "lines",
        AnimUnit::Word => "words",
        AnimUnit::Letter => "letters",
    }
}

const TILE: egui::Vec2 = egui::vec2(88.0, 66.0);

/// The id of `preset`'s tile in `slot`'s grid (the empty id is None).
pub(crate) fn tile_id(kind: SelectionKind, slot: Slot, preset: &PresetId) -> egui::Id {
    widget_id(kind, CardId::Animation, "tile").with((slot, preset.as_str()))
}

/// One group of the grid as shown: its heading, the tiles Pieces lets
/// through, and the presets that can't play.
struct Group<'g> {
    title: Option<&'static str>,
    choices: Vec<&'g PresetChoice>,
    unavailable: &'g [UnavailableChoice],
}

/// A tile of the grid.
#[derive(Clone, Copy)]
enum Tile<'g> {
    None,
    Preset(&'g PresetChoice),
    Unavailable(&'g UnavailableChoice),
}

/// The preset grid (ui-concept §13.7): "None", then the slot's presets,
/// under FX-019's kind headings when there is more than one group, each
/// group ending with its **Unavailable animation** tiles (FX-018), which
/// can't be chosen. Returns the tile clicked: `Some(None)` for None.
fn grid<'c>(
    ui: &mut egui::Ui,
    env: &mut Env<'_, '_>,
    slot: Slot,
    spec: Option<&AnimationSpec>,
    groups: &[Group<'c>],
) -> Option<Option<&'c PresetChoice>> {
    let mut clicked = None;
    let mut hovered_now: Option<PresetId> = None;
    let spacing = 6.0;
    let per_row = ((ui.available_width() + spacing) / (TILE.x + spacing))
        .floor()
        .max(1.0) as usize;
    let headed = groups.len() > 1;
    let mut sections: Vec<(Option<&'static str>, Vec<Tile<'c>>)> = groups
        .iter()
        .map(|g| {
            let tiles = g
                .choices
                .iter()
                .map(|c| Tile::Preset(c))
                .chain(g.unavailable.iter().map(Tile::Unavailable))
                .collect();
            (g.title.filter(|_| headed), tiles)
        })
        .collect();
    // None leads the grid: in its own row above headed groups.
    match sections.first_mut() {
        Some((_, tiles)) if !headed => tiles.insert(0, Tile::None),
        _ => sections.insert(0, (None, vec![Tile::None])),
    }
    let kind = env.kind;
    let motion = &mut *env.motion;
    let hovered = motion.hovered.clone();
    for (title, tiles) in &sections {
        if let Some(title) = title {
            let p = theme::look(ui.ctx()).palette;
            ui.label(
                egui::RichText::new(*title)
                    .text_style(Type::Caption.style(ui.style()))
                    .color(p.text_secondary),
            );
        }
        for row in tiles.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = spacing;
                for tile in row {
                    let preset = match tile {
                        Tile::Preset(c) => Some(*c),
                        Tile::None | Tile::Unavailable(_) => None,
                    };
                    let selected = match (tile, spec) {
                        (Tile::None, None) => true,
                        (Tile::Preset(c), Some(s)) => c.id == s.preset,
                        _ => false,
                    };
                    let id = preset.map(|c| c.id.clone());
                    let frame = match (&hovered, preset) {
                        (Some((p, s, since)), Some(c))
                            if *p == c.id && *s == slot && motion.loops_play() =>
                        {
                            hover_frame(since.elapsed(), c, slot)
                        }
                        _ => 0,
                    };
                    let tile_id = match tile {
                        Tile::Preset(c) => tile_id(kind, slot, &c.id),
                        Tile::None => tile_id(kind, slot, &PresetId::new("")),
                        Tile::Unavailable(u) => tile_id(kind, slot, &u.id),
                    };
                    let r = tile_ui(ui, motion, tile_id, *tile, slot, selected, frame);
                    if r.hovered() || r.has_focus() {
                        hovered_now = id.clone();
                    }
                    if r.clicked() {
                        match tile {
                            Tile::None => clicked = Some(None),
                            Tile::Preset(c) => clicked = Some(Some(*c)),
                            Tile::Unavailable(_) => {}
                        }
                    }
                }
            });
            ui.add_space(spacing);
        }
    }
    // One tile plays at a time (UI-007): a new hover asks for its frames.
    match (&hovered_now, &motion.hovered) {
        (Some(now), Some((was, s, _))) if now == was && *s == slot => {
            // Only a playing loop needs the next frame; a still doesn't.
            if motion.loops_play() {
                ui.ctx()
                    .request_repaint_after(Duration::from_millis(1000 / TILE_FPS as u64));
            }
        }
        (Some(now), _) => {
            motion.hovered = Some((now.clone(), slot, Instant::now()));
            // Only the hovered tile's loop is kept (stills stay): pictures
            // of tiles hovered before are dropped.
            motion
                .textures
                .retain(|k, _| k.frame == 0 || (k.preset == *now && k.slot == slot));
            let choice = groups
                .iter()
                .flat_map(|g| &g.choices)
                .find(|c| c.id == *now);
            if !motion.reduce_motion
                && let (Some(w), Some(c)) = (&motion.posters, choice)
            {
                let n = sample_times(&c.manifest, slot).frames;
                w.want_motion(
                    (1..=n)
                        .map(|frame| PosterKey {
                            preset: now.clone(),
                            slot,
                            frame,
                        })
                        .collect(),
                );
            }
            ui.ctx().request_repaint();
        }
        (None, Some(_)) => {
            motion.hovered = None;
            if let Some(w) = &motion.posters {
                w.want_motion(Vec::new());
            }
        }
        (None, None) => {}
    }
    clicked
}

/// The frame of a hovered tile's loop `elapsed` after the hover began:
/// the still (0) for UI-007's first 150 ms, then 1-based frames at
/// [`TILE_FPS`], round and round. The loop is the sample's, as the poster
/// worker renders it (the preset's default length), also on the chosen
/// tile.
fn hover_frame(elapsed: Duration, c: &PresetChoice, slot: Slot) -> u16 {
    let delay = Duration::from_millis(150);
    if elapsed < delay {
        return 0;
    }
    let k = ((elapsed - delay).as_secs_f64() * TILE_FPS as f64) as u64;
    let n = sample_times(&c.manifest, slot).frames;
    u16::try_from(k % u64::from(n.max(1))).unwrap_or(0) + 1
}

/// One tile: the picture (or a neutral placeholder), the name below, an
/// `accent` ring and a check badge when selected. An unavailable preset
/// is a dimmed tile with a warning sign that can't be chosen; its tooltip
/// says why (FX-018).
fn tile_ui(
    ui: &mut egui::Ui,
    motion: &mut MotionState,
    id: egui::Id,
    tile: Tile<'_>,
    slot: Slot,
    selected: bool,
    frame: u16,
) -> egui::Response {
    let p = theme::look(ui.ctx()).palette;
    let name = match tile {
        Tile::None => "None",
        Tile::Preset(c) => c.name.as_str(),
        Tile::Unavailable(u) => u.name.as_str(),
    };
    let enabled = !matches!(tile, Tile::Unavailable(_));
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, _) = ui.allocate_exact_size(TILE + egui::vec2(0.0, 16.0), Sense::hover());
    let response = ui.interact(rect, id, sense);
    let thumb = egui::Rect::from_min_size(rect.min, TILE);
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(thumb, theme::R_SM, p.bg_well);
        let sign = |glyph: &str, color| {
            ui.painter().text(
                thumb.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                crate::theme::fonts::icon_font(ui.ctx(), 20.0, false),
                color,
            );
        };
        match tile {
            Tile::Preset(c) => {
                if let Some(tex) = motion.picture(&c.id, slot, frame) {
                    let image = egui::Image::new(tex).fit_to_exact_size(TILE);
                    image.paint_at(ui, thumb);
                }
            }
            Tile::None => sign(icons::CLOSE, p.text_tertiary),
            Tile::Unavailable(_) => sign(icons::WARNING, p.text_tertiary),
        }
        if enabled && response.hovered() {
            ui.painter()
                .rect_filled(thumb, theme::R_SM, p.overlay_hover);
        }
        if selected {
            ui.painter().rect_stroke(
                thumb,
                theme::R_SM,
                Stroke::new(2.0, p.accent),
                StrokeKind::Inside,
            );
            let badge = egui::Rect::from_center_size(
                thumb.right_top() + egui::vec2(-9.0, 9.0),
                egui::vec2(14.0, 14.0),
            );
            ui.painter().circle_filled(badge.center(), 7.0, p.accent);
            ui.painter().text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                "✓",
                egui::FontId::proportional(10.0),
                p.text_on_accent,
            );
        }
        if response.has_focus() {
            crate::widgets::focus_ring(ui, thumb, theme::R_SM);
        }
        let font = Type::Caption.style(ui.style()).resolve(ui.style());
        ui.painter().text(
            egui::pos2(thumb.center().x, thumb.bottom() + 2.0),
            egui::Align2::CENTER_TOP,
            name,
            font,
            if selected {
                p.text_primary
            } else if enabled {
                p.text_secondary
            } else {
                p.text_tertiary
            },
        );
    }
    let label = match tile {
        Tile::Unavailable(_) => format!("{name} ({}), unavailable", slot_name(slot)),
        _ => format!("{name} ({})", slot_name(slot)),
    };
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, &label)
    });
    response.on_hover_text(match tile {
        Tile::Preset(_) => format!("{name}: choose for {}", slot_name(slot)),
        Tile::None => format!("No {} animation", slot_name(slot)),
        Tile::Unavailable(u) => unavailable_hint(u),
    })
}

/// The tooltip of an unavailable tile: FX-018's name for it and why.
fn unavailable_hint(u: &UnavailableChoice) -> String {
    format!(
        "Unavailable animation: this build of FreeCut can't play {} ({})",
        u.name, u.reason
    )
}

/// A spec edit of `slot`.
fn spec_edit(slot: Slot, spec: AnimationSpec) -> Edit {
    Edit::Animation {
        slot,
        spec: Some(spec),
    }
}

fn secs(t: Tick) -> f64 {
    t.as_secs_f64()
}

fn tick_of(secs: f64) -> Tick {
    let raw = (secs * TICKS_PER_SECOND as f64).round();
    if raw.is_nan() {
        return Tick::ZERO;
    }
    // Clamped into range first, so the cast is exact.
    Tick::from_raw(raw.clamp(i64::MIN as f64, i64::MAX as f64) as i64)
}

#[allow(clippy::too_many_arguments)]
fn settings_rows(
    ui: &mut egui::Ui,
    env: &mut Env<'_, '_>,
    at: At<'_>,
    steps: &mut Steps,
    clip: &Clip,
    slot: Slot,
    spec: &AnimationSpec,
    m: &PresetManifest,
) {
    let text = clip.content.text();
    let rate = env.doc.settings().frame_rate;
    let per_unit = text.is_some() && m.is_per_unit();

    // Overlap and Order (TX-023).
    if per_unit {
        let stored = spec.params.overlap;
        rows::percent(
            ui,
            at,
            steps,
            "overlap",
            "Overlap",
            stored,
            m.defaults.overlap,
            (0, 1000),
            |v| {
                let mut s = spec.clone();
                s.params.overlap = v;
                spec_edit(slot, s)
            },
        );
        let orders = [
            AnimOrder::Forward,
            AnimOrder::Backward,
            AnimOrder::CenterOut,
            AnimOrder::EdgesIn,
            AnimOrder::Random,
        ];
        let items = [
            Segment::new(None, "Forward"),
            Segment::new(None, "Back"),
            Segment::new(None, "Centre"),
            Segment::new(None, "Edges"),
            Segment::new(None, "Random"),
        ];
        rows::choice(
            ui,
            at,
            steps,
            "order",
            "Order",
            &items,
            &orders,
            spec.params.order,
            m.defaults.order,
            false,
            |o| {
                let mut s = spec.clone();
                s.params.order = o;
                spec_edit(slot, s)
            },
        );
    }

    // The dice (TX-023, TX-056): a random order, or a motion drawn from
    // the seed (Scatter, Shake, Decode, a clip's Jitter …).
    let random_order = per_unit && spec.params.order == AnimOrder::Random;
    if random_order || m.uses_seed() {
        let hint = if random_order {
            "New seed: another random order"
        } else {
            "New seed: another random motion"
        };
        let row = PropRow::new(at.id("seed"), "Seed")
            .parameter(120.0)
            .show(ui, |ui| {
                let dice = icon_button(ui, icons::DICE, 18.0, hint);
                ui.label(egui::RichText::new(spec.params.seed.to_string()).monospace());
                dice
            });
        let edit = row.inner.clicked().then(|| {
            let mut s = spec.clone();
            s.params.seed = next_seed(spec.params.seed);
            spec_edit(slot, s)
        });
        steps.button(row.inner, edit);
    }

    // Typewriter's Caret (TX-056): on a stepped reveal in In or Out.
    if text.is_some() && slot != Slot::Continuous && freecut_render::motion::is_stepped(m) {
        let id = at.id("caret");
        let on = spec.params.caret;
        let row = PropRow::new(id, "Caret")
            .parameter(32.0)
            .hint("A blinking bar after the last typed letter")
            .show(ui, |ui| {
                let mut state = on;
                toggle(ui, id.with("toggle"), &mut state, "Caret")
            });
        let edit = row.inner.changed().then(|| {
            let mut s = spec.clone();
            s.params.caret = !on;
            spec_edit(slot, s)
        });
        steps.button(row.inner, edit);
    }

    // Length, or Pace (TX-022), or the Loop's cycle (FX-019).
    let paceable = text.is_some() && anim::can_pace(slot, m, spec);
    if paceable && let Some(data) = text {
        let id = at.id("pace_switch");
        let on = spec.params.pace.is_some();
        let row = PropRow::new(id, "Pace")
            .parameter(32.0)
            .hint("Set a time per piece instead of a total length")
            .show(ui, |ui| {
                let mut state = on;
                toggle(ui, id.with("toggle"), &mut state, "Pace")
            });
        let edit = row
            .inner
            .changed()
            .then(|| spec_edit(slot, anim::with_pace(spec, m, data.content.as_str(), !on)));
        steps.button(row.inner, edit);
    }
    match (spec.params.pace, slot) {
        (Some(pace), Slot::Entrance | Slot::Exit) if paceable => {
            let n = Numeric::new("pace", "Per piece", secs(pace), 0.06, 0.02..=1.0)
                .suffix(" s")
                .decimals(2)
                .no_keys();
            rows::numeric(ui, at, n, steps, |v| {
                let mut s = spec.clone();
                s.params.pace = Some(tick_of(v));
                (s.params.pace != spec.params.pace).then(|| spec_edit(slot, s))
            });
            let unit = effective_unit(m, spec, true);
            let units = text.map_or(0, |d| count_units(d.content.as_str(), unit));
            let total =
                pace_duration(units, pace, spec.params.overlap, rate).unwrap_or(spec.duration);
            PropRow::new(at.id("paced_length"), "Length").show(ui, |ui| {
                ui.weak(format!("{:.2} s for {units} pieces", secs(total)));
            });
        }
        (_, Slot::Continuous) => {
            let max = secs(AnimationSpec::MAX_CYCLE);
            let n = Numeric::new(
                "cycle",
                "Cycle",
                secs(spec.duration),
                secs(m.default_duration),
                0.1..=max,
            )
            .suffix(" s")
            .decimals(2)
            .no_keys();
            rows::numeric(ui, at, n, steps, |v| {
                let t = rate.quantize(tick_of(v)).unwrap_or(spec.duration);
                (t != spec.duration).then(|| {
                    let mut s = spec.clone();
                    s.duration = t;
                    spec_edit(slot, s)
                })
            });
            if spec.duration.is_positive() {
                PropRow::new(at.id("speed"), "Speed").show(ui, |ui| {
                    ui.weak(format!("{:.2} cycles a second", 1.0 / secs(spec.duration)));
                });
            }
        }
        _ => {
            let other = match slot {
                Slot::Entrance => clip.animations.exit.as_ref(),
                _ => clip.animations.entrance.as_ref(),
            };
            let room = clip.duration().raw() - other.map_or(0, |o| o.duration.raw());
            let max = (room as f64 / TICKS_PER_SECOND as f64).max(0.1);
            let n = Numeric::new(
                "length",
                "Length",
                secs(spec.duration),
                secs(m.default_duration),
                0.1..=max,
            )
            .suffix(" s")
            .decimals(2)
            .no_keys();
            rows::numeric(ui, at, n, steps, |v| {
                let t = rate.quantize(tick_of(v)).unwrap_or(spec.duration);
                (t != spec.duration).then(|| {
                    let mut s = spec.clone();
                    s.duration = t;
                    spec_edit(slot, s)
                })
            });
        }
    }

    // Easing (FX-018: any TR-031 preset but Hold, or the preset's own).
    let id = at.id("easing");
    let easings: Vec<Option<Interp>> = std::iter::once(None)
        .chain(
            Interp::PRESETS
                .into_iter()
                .filter(|e| *e != Interp::Hold)
                .map(Some),
        )
        .collect();
    let own = format!("Preset's own ({})", prop_keys::interp_label(m.easing));
    let options: Vec<SelectOption<'_>> = easings
        .iter()
        .map(|e| {
            SelectOption::new(match e {
                Some(e) => prop_keys::interp_label(*e),
                None => own.as_str(),
            })
        })
        .collect();
    let current = easings.iter().position(|e| *e == spec.easing);
    let row = PropRow::new(id, "Easing").show(ui, |ui| {
        Select::new(id.with("select"), &options, current)
            .fill_width()
            .show(ui)
            .picked
            .and_then(|i| easings.get(i).copied())
    });
    if let Some(easing) = row.inner.filter(|e| *e != spec.easing) {
        let mut s = spec.clone();
        s.easing = easing;
        steps.edit(spec_edit(slot, s), Merge::Own);
    }

    // Direction, where the preset has one (FX-018).
    if let Some(dirs) = &m.directions {
        let id = at.id("direction");
        let choices: Vec<Option<Direction>> = std::iter::once(None)
            .chain(dirs.iter().copied().map(Some))
            .collect();
        let options: Vec<SelectOption<'_>> = choices
            .iter()
            .map(|d| SelectOption::new(d.map_or("Preset's own", direction_name)))
            .collect();
        let current = choices.iter().position(|d| *d == spec.direction);
        let row = PropRow::new(id, "Direction").show(ui, |ui| {
            Select::new(id.with("select"), &options, current)
                .fill_width()
                .show(ui)
                .picked
                .and_then(|i| choices.get(i).copied())
        });
        if let Some(direction) = row.inner.filter(|d| *d != spec.direction) {
            let mut s = spec.clone();
            s.direction = direction;
            steps.edit(spec_edit(slot, s), Merge::Own);
        }
    }

    // Travel, for slides (FX-030).
    if m.channels.iter().any(|c| c.unit == ChannelUnit::Travel) {
        let items = [Segment::new(None, "From edge"), Segment::new(None, "Short")];
        rows::choice(
            ui,
            at,
            steps,
            "travel",
            "Travel",
            &items,
            &[Travel::FromEdge, Travel::Short],
            spec.params.travel,
            Travel::FromEdge,
            false,
            |t| {
                let mut s = spec.clone();
                s.params.travel = t;
                spec_edit(slot, s)
            },
        );
    }

    // Intensity, on presets that move (TX-022: fades have none).
    if m.channels.iter().any(|c| c.scales_with_intensity) {
        let stored = spec.params.intensity;
        let n = Numeric::new(
            "intensity",
            "Intensity",
            f64::from(stored.get()) / 10.0,
            50.0,
            0.0..=100.0,
        )
        .decimals(0)
        .no_keys();
        rows::numeric(ui, at, n, steps, |v| {
            let wanted = rows::per_mille_in(v * 10.0, 0, 1000);
            (wanted != stored).then(|| {
                let mut s = spec.clone();
                s.params.intensity = wanted;
                spec_edit(slot, s)
            })
        });
    }
}

/// A new seed for the dice (TX-023): a fixed scramble of the old one, so
/// it is deterministic and never repeats at once.
pub(crate) fn next_seed(seed: u32) -> u32 {
    let mut x = seed.wrapping_add(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    if x == seed { x.wrapping_add(1) } else { x }
}

fn direction_name(d: Direction) -> &'static str {
    match d {
        Direction::Left => "From the left",
        Direction::Right => "From the right",
        Direction::Up => "From the top",
        Direction::Down => "From the bottom",
        Direction::Clockwise => "Clockwise",
        Direction::CounterClockwise => "Counter-clockwise",
    }
}

// ------------------------------------------------------- keyframes

/// The Keyframes card (ui-concept §7.2): every animated property of the
/// clip with its navigator; each key with its time, value, easing and a
/// remove button. Text clips list Reveal (TX-021) always.
fn keyframes_card(ui: &mut egui::Ui, env: &mut Env<'_, '_>, clip: &Arc<Clip>) {
    let kind = env.kind;
    let mut props: Vec<PropPath> = clip.keys.tracks().iter().map(|t| t.prop()).collect();
    if clip.content.text().is_some() && !props.contains(&PropPath::Reveal) {
        props.push(PropPath::Reveal);
    }
    let count: usize = clip.keys.tracks().iter().map(|t| t.keys().len()).sum();
    let summary = match count {
        0 => "none".to_owned(),
        1 => "1 key".to_owned(),
        n => format!("{n} keys"),
    };
    let mut memory = *env.workspace.card(kind, CardId::Keyframes);
    let keys_view = env.keys;
    let everything = env.workspace.detail == Detail::Everything;
    let shown = crate::widgets::card::PropertyCard::new(
        widget_id(kind, CardId::Keyframes, "card"),
        "Keyframes",
    )
    .summary(summary)
    .keyed(count > 0)
    .show(ui, &mut memory, |ui, memory| {
        let at = At::new(kind, CardId::Keyframes).with_keys(keys_view);
        let mut steps = Steps::default();
        if count == 0 && clip.content.text().is_none() {
            empty_state(
                ui,
                icons::DIAMOND,
                "No keyframes yet",
                "Use the diamond next to a setting to key it at the playhead.",
            );
        }
        let rate = env.doc.settings().frame_rate;
        for prop in &props {
            let label = prop_keys::prop_label(*prop);
            let keys = prop_keys::shown_keys(clip, *prop);
            if *prop == PropPath::Reveal {
                reveal_row(ui, at, &mut steps, clip, &label);
            } else {
                let nav = at.nav(&label, Some(*prop));
                let mut row = PropRow::new(at.id(&prop.to_path_string()), &label);
                if let Some((_, n)) = nav {
                    row = row.keys(n);
                }
                let out = row.show(ui, |ui| {
                    ui.weak(match keys.len() {
                        1 => "1 key".to_owned(),
                        n => format!("{n} keys"),
                    });
                });
                if let Some((p, _)) = nav {
                    at.keys_clicked(p, out.keys, &mut steps);
                }
            }
            if everything || memory.more {
                for (i, k) in keys.iter().enumerate() {
                    key_row(ui, at, &mut steps, *prop, i, k, rate);
                }
            }
        }
        if !props.is_empty() {
            crate::widgets::card::more(ui, at.id("more"), memory, everything, count, |_| {});
        }
        let clips = env.selection.clone();
        props::commit(env.project, env.drag, &clips, steps, props::held(ui))
    });
    *env.workspace.card(kind, CardId::Keyframes) = memory;
    status(env, shown.inner.flatten());
}

/// Reveal (TX-021): the share of characters shown, keyed at the playhead
/// whenever it is changed (it has no static value).
fn reveal_row(ui: &mut egui::Ui, at: At<'_>, steps: &mut Steps, clip: &Clip, label: &str) {
    let shown = at
        .keys
        .and_then(|v| clip.content_time_at(v.playhead))
        .map_or(PerMille::ONE, |ct| clip.reveal_at(ct));
    let playhead = at.keys.map_or(Tick::ZERO, |v| v.playhead);
    rows::percent(
        ui,
        at,
        steps,
        "reveal",
        label,
        shown,
        PerMille::ONE,
        (0, 1000),
        |v| Edit::Keys {
            prop: PropPath::Reveal,
            edit: KeyEdit::Add {
                at: playhead,
                value: Some(PropValue::Ratio(v)),
            },
        },
    );
}

/// One key: its time and value, the easing of the segment it starts, and
/// remove.
fn key_row(
    ui: &mut egui::Ui,
    at: At<'_>,
    steps: &mut Steps,
    prop: PropPath,
    i: usize,
    k: &prop_keys::ShownKey,
    rate: freecut_timeline::time::FrameRate,
) {
    let id = at.id(&format!("{}#{i}", prop.to_path_string()));
    let time =
        Timecode::from_tick(k.at, rate, false).map_or_else(|_| "--".into(), |t| t.to_string());
    let options: Vec<SelectOption<'_>> = Interp::PRESETS
        .iter()
        .map(|e| SelectOption::new(prop_keys::interp_label(*e)))
        .collect();
    let current = Interp::PRESETS.iter().position(|e| *e == k.interp);
    let row = PropRow::new(id, &time).show(ui, |ui| {
        let p = theme::look(ui.ctx()).palette;
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add(
            egui::Label::new(
                egui::RichText::new(prop_keys::value_label(k.value)).color(p.text_tertiary),
            )
            .truncate(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let jump = icon_button(
                ui,
                icons::CARET_RIGHT,
                18.0,
                "Move the playhead to this keyframe",
            );
            let remove = icon_button(ui, icons::MINUS, 18.0, "Remove this keyframe");
            ui.add_space(4.0);
            let picked = Select::new(id.with("easing"), &options, current)
                .show(ui)
                .picked
                .and_then(|i| Interp::PRESETS.get(i).copied());
            (picked, remove, jump)
        })
        .inner
    });
    let (picked, remove, jump) = row.inner;
    if let Some(interp) = picked.filter(|e| *e != k.interp) {
        steps.edit(
            Edit::Keys {
                prop,
                edit: KeyEdit::SetInterp { at: k.at, interp },
            },
            Merge::Own,
        );
    }
    if remove.clicked() {
        steps.edit(
            Edit::Keys {
                prop,
                edit: KeyEdit::Remove { at: k.at },
            },
            Merge::Own,
        );
    }
    if jump.clicked()
        && let Some(view) = at.keys
    {
        view.seek.set(Some(k.at));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grids are built once per catalogue, not per repaint, and again
    /// when the catalogue changes.
    #[test]
    fn grids_are_built_once_per_catalogue() {
        let mut state = MotionState {
            presets: Arc::new(PresetTable::samples()),
            ..MotionState::default()
        };
        let first = state.grids();
        assert!(Arc::ptr_eq(&first, &state.grids()), "kept between frames");
        state.presets = Arc::new(PresetTable::samples());
        let second = state.grids();
        assert!(!Arc::ptr_eq(&first, &second), "rebuilt for a new catalogue");
        assert_eq!(second.groups(Slot::Entrance, true)[0].choices.len(), 5);
    }

    #[test]
    fn the_dice_always_gives_another_seed() {
        let mut seen = std::collections::HashSet::new();
        let mut s = 0;
        for _ in 0..1000 {
            let n = super::next_seed(s);
            assert_ne!(n, s);
            seen.insert(n);
            s = n;
        }
        assert!(seen.len() > 990);
    }
}
