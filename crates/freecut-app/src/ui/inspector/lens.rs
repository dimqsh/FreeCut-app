//! What the inspector shows for a selection (UI-008, ui-concept §6.3,
//! TX-054): its [`SelectionKind`], the [`Lens`]es of that kind, the
//! [`CardId`]s, and the workspace state that remembers the last lens per
//! kind, each card's collapse state per (kind, card) and the Detail switch.
//!
//! Widget ids come from (kind, card, row) only ([`widget_id`]), never from
//! the clip, so collapse state and a drag's bookkeeping survive a selection
//! change between clips of the same kind (ui-concept §16.3). A field's
//! typed text does not: the inspector draws the cards under the selection's
//! own id, and commits typing to the clips it was typed for before showing
//! others (UI-008).

use std::collections::HashMap;

use eframe::egui::Id;
use freecut_timeline::ids::ClipId;
use freecut_timeline::model::{
    ClipContent, ColorPage, Document, GeneratedKind, MediaKind, TextCard, TextLayerKind, TextLens,
};
use serde_json::{Map, Value, json};

use crate::theme::icons;
use crate::widgets::card::CardMemory;

/// The kind of the selection, as far as the inspector cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionKind {
    Nothing,
    Text,
    Video,
    Image,
    Audio,
    /// Stickers, shapes and effect layers.
    Element,
    /// Adjustment clips (CL-006): the Look lens only, Amount reads Mix.
    Adjustment,
    Several,
}

/// The kind of `selection` in `doc`. Clips that no longer exist count as
/// nothing (undo removed them, UI-008).
pub fn selection_kind(doc: &Document, selection: &[ClipId]) -> SelectionKind {
    match selection {
        [] => SelectionKind::Nothing,
        [id] => match doc.clip(*id).map(|c| &c.content) {
            None => SelectionKind::Nothing,
            Some(ClipContent::Text(_)) => SelectionKind::Text,
            Some(ClipContent::Media {
                kind: MediaKind::Video,
                ..
            }) => SelectionKind::Video,
            Some(ClipContent::Media {
                kind: MediaKind::Audio,
                ..
            }) => SelectionKind::Audio,
            Some(ClipContent::Image { .. }) => SelectionKind::Image,
            Some(ClipContent::Generated {
                kind: GeneratedKind::Adjustment,
            }) => SelectionKind::Adjustment,
            Some(ClipContent::Generated { .. }) => SelectionKind::Element,
        },
        _ => SelectionKind::Several,
    }
}

impl SelectionKind {
    /// The kind's icon in the inspector header (never colour alone).
    pub fn icon(self) -> &'static str {
        match self {
            SelectionKind::Nothing => icons::PROJECT,
            SelectionKind::Text => icons::TEXT,
            SelectionKind::Video => icons::MEDIA,
            SelectionKind::Image => icons::LENS_PICTURE,
            SelectionKind::Audio => icons::MUSIC_NOTE,
            SelectionKind::Element | SelectionKind::Several => icons::SHAPES,
            SelectionKind::Adjustment => icons::ADJUST,
        }
    }
}

/// A lens: one tab of the inspector's lens bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lens {
    Project,
    Content,
    Style,
    BoxBubble,
    Motion,
    Presets,
    Picture,
    /// Colour (CL-043): the Basic, HSL, Curves, Wheels and Masks pages.
    Look,
    Sound,
}

impl Lens {
    pub fn label(self) -> &'static str {
        match self {
            Lens::Project => "Project",
            Lens::Content => "Content",
            Lens::Style => "Style",
            Lens::BoxBubble => "Box & bubble",
            Lens::Motion => "Motion",
            Lens::Presets => "Presets",
            Lens::Picture => "Picture",
            Lens::Look => "Look",
            Lens::Sound => "Sound",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Lens::Project => icons::PROJECT,
            Lens::Content => icons::LENS_CONTENT,
            Lens::Style => icons::LENS_STYLE,
            Lens::BoxBubble => icons::LENS_BOX,
            Lens::Motion => icons::LENS_MOTION,
            Lens::Presets => icons::LENS_PRESETS,
            Lens::Picture => icons::LENS_PICTURE,
            Lens::Look => icons::LENS_LOOK,
            Lens::Sound => icons::LENS_SOUND,
        }
    }
}

/// The text lenses (TX-054).
pub const TEXT_LENSES: [Lens; 5] = [
    Lens::Content,
    Lens::Style,
    Lens::BoxBubble,
    Lens::Motion,
    Lens::Presets,
];

/// What a multi-selection holds, for the lenses its kinds share.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Holds {
    pub text: bool,
    pub visual: bool,
    /// A clip that takes a colour stack (CL-001).
    pub color: bool,
}

impl Holds {
    /// What `clips` hold.
    pub fn of(clips: &[&freecut_timeline::model::Clip]) -> Holds {
        Holds {
            text: clips.iter().any(|c| c.content.text().is_some()),
            visual: clips.iter().any(|c| c.content.is_visual()),
            color: clips.iter().any(|c| c.content.takes_color()),
        }
    }
}

/// The lenses of a selection of `kind` (ui-concept §6.3). For several
/// clips, the lenses their kinds share in this build (UI-010): text
/// settings when any is text, the picture when any is visual, the look
/// when any takes colour, and the sound commands.
pub fn lenses(kind: SelectionKind, holds: Holds) -> Vec<Lens> {
    match kind {
        SelectionKind::Nothing => vec![Lens::Project],
        SelectionKind::Text => TEXT_LENSES.to_vec(),
        SelectionKind::Video => vec![Lens::Picture, Lens::Look, Lens::Motion, Lens::Sound],
        SelectionKind::Image => vec![Lens::Picture, Lens::Look, Lens::Motion],
        SelectionKind::Element => vec![Lens::Picture, Lens::Motion],
        SelectionKind::Adjustment => vec![Lens::Look],
        SelectionKind::Audio => vec![Lens::Sound],
        SelectionKind::Several => {
            let mut out = Vec::new();
            if holds.text {
                out.extend([Lens::Content, Lens::Style]);
            }
            if holds.visual {
                out.push(Lens::Picture);
            }
            if holds.color {
                out.push(Lens::Look);
            }
            out.push(Lens::Sound);
            out
        }
    }
}

/// A card of the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardId {
    Project,
    /// Read-only clip details (UI-008's Clip info), collapsed at first.
    ClipInfo,
    Text,
    Paragraph,
    Type,
    Fill,
    Spacing,
    Outline,
    Shadow,
    Glow,
    Background,
    Bubble,
    Curve,
    /// Content lens: convert to captions (TX-035), a stub until captions.
    Convert,
    Placement,
    Volume,
    Fades,
    /// The sound of a video clip whose audio is detached, or that has none.
    SoundState,
    /// Mute and detach for several clips.
    SoundCommands,
    /// Motion lens: the In / Loop / Out timing strip, the preset grid and
    /// the selected animation's settings (FX-018, FX-019, TX-022).
    Animation,
    /// Motion lens: every animated property with its keys (TX-021, TR-028).
    Keyframes,
    /// Look lens header: the master switch and Amount (CL-023, CL-035).
    Grade,
    /// Look lens, Basic page: Light (CL-007).
    Light,
    /// Look lens, Basic page: Color (CL-008).
    ColorBalance,
    /// Look lens, Basic page: LUT (CL-017, CL-030).
    Lut,
    /// Look lens, HSL page (CL-013).
    Hsl,
    /// Look lens, Curves page (CL-014).
    Curves,
    /// Look lens, Wheels page (CL-015).
    Wheels,
}

impl CardId {
    /// Whether the card starts open.
    pub fn starts_open(self) -> bool {
        !matches!(self, CardId::ClipInfo | CardId::Convert)
    }

    /// The model's text card this card shows (TX-054), for **Reset card**.
    pub fn text_card(self) -> Option<TextCard> {
        Some(match self {
            CardId::Text => TextCard::Text,
            CardId::Paragraph => TextCard::Paragraph,
            CardId::Type => TextCard::Type,
            CardId::Fill => TextCard::Fill,
            CardId::Spacing => TextCard::Spacing,
            CardId::Outline => TextCard::Outline,
            CardId::Shadow => TextCard::Shadow,
            CardId::Glow => TextCard::Glow,
            CardId::Background => TextCard::Background,
            CardId::Bubble => TextCard::Bubble,
            CardId::Curve => TextCard::Curve,
            _ => return None,
        })
    }

    /// The optional text layer whose switch is in this card's header.
    pub fn layer(self) -> Option<TextLayerKind> {
        Some(match self {
            CardId::Outline => TextLayerKind::Outline,
            CardId::Shadow => TextLayerKind::Shadow,
            CardId::Glow => TextLayerKind::Glow,
            CardId::Background => TextLayerKind::Background,
            CardId::Bubble => TextLayerKind::Bubble,
            CardId::Curve => TextLayerKind::Curve,
            _ => return None,
        })
    }
}

impl Lens {
    /// The model's text lens, for **Reset lens** (TX-054).
    pub fn text_lens(self) -> Option<TextLens> {
        match self {
            Lens::Content => Some(TextLens::Content),
            Lens::Style => Some(TextLens::Style),
            Lens::BoxBubble => Some(TextLens::BoxAndBubble),
            _ => None,
        }
    }
}

/// The Detail switch (ui-concept §6.1): essential rows only, or all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Detail {
    #[default]
    Essentials,
    Everything,
}

/// The id of row `row` of `card` for a selection of `kind`: the same for
/// every clip of that kind.
pub fn widget_id(kind: SelectionKind, card: CardId, row: &str) -> Id {
    Id::new(("freecut-inspector", kind, card, row))
}

/// The names a type's values are written as in `workspace.json`.
macro_rules! keys {
    ($ty:ident { $($v:ident => $k:literal),* $(,)? }) => {
        impl $ty {
            const ALL: &'static [$ty] = &[$($ty::$v),*];

            fn key(self) -> &'static str {
                match self {
                    $($ty::$v => $k),*
                }
            }

            fn from_key(key: &str) -> Option<$ty> {
                $ty::ALL.iter().copied().find(|v| v.key() == key)
            }
        }
    };
}

keys!(SelectionKind {
    Nothing => "nothing",
    Text => "text",
    Video => "video",
    Image => "image",
    Audio => "audio",
    Element => "element",
    Adjustment => "adjustment",
    Several => "several",
});

keys!(Lens {
    Project => "project",
    Content => "content",
    Style => "style",
    BoxBubble => "box_bubble",
    Motion => "motion",
    Presets => "presets",
    Picture => "picture",
    Look => "look",
    Sound => "sound",
});

keys!(CardId {
    Project => "project",
    ClipInfo => "clip_info",
    Text => "text",
    Paragraph => "paragraph",
    Type => "type",
    Fill => "fill",
    Spacing => "spacing",
    Outline => "outline",
    Shadow => "shadow",
    Glow => "glow",
    Background => "background",
    Bubble => "bubble",
    Curve => "curve",
    Convert => "convert",
    Placement => "placement",
    Volume => "volume",
    Fades => "fades",
    SoundState => "sound_state",
    SoundCommands => "sound_commands",
    Animation => "animation",
    Keyframes => "keyframes",
    Grade => "grade",
    Light => "light",
    ColorBalance => "color_balance",
    Lut => "lut",
    Hsl => "hsl",
    Curves => "curves",
    Wheels => "wheels",
});

/// The name a Look page is written as in `workspace.json`.
fn page_key(page: ColorPage) -> &'static str {
    match page {
        ColorPage::Basic => "basic",
        ColorPage::Hsl => "hsl",
        ColorPage::Curves => "curves",
        ColorPage::Wheels => "wheels",
        ColorPage::Masks => "masks",
    }
}

fn page_from_key(key: &str) -> Option<ColorPage> {
    ColorPage::ALL.into_iter().find(|p| page_key(*p) == key)
}

keys!(Detail {
    Essentials => "essentials",
    Everything => "everything",
});

/// The inspector's workspace state (UI-008: "collapsed sections are
/// remembered per (selection kind, section) in workspace state"): written
/// to `workspace.json` (UI-002, [`crate::workspace`]); not part of the
/// project or undo.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Workspace {
    pub detail: Detail,
    lens: HashMap<SelectionKind, Lens>,
    cards: HashMap<(SelectionKind, CardId), CardMemory>,
    /// The Look lens's page per selection kind (CL-043).
    look_page: HashMap<SelectionKind, ColorPage>,
}

impl Workspace {
    /// The lens shown for `kind`: the remembered one when `available` has
    /// it, else the first (UI-008's tab memory).
    pub fn lens(&self, kind: SelectionKind, available: &[Lens]) -> Option<Lens> {
        self.lens
            .get(&kind)
            .copied()
            .filter(|l| available.contains(l))
            .or_else(|| available.first().copied())
    }

    pub fn set_lens(&mut self, kind: SelectionKind, lens: Lens) {
        self.lens.insert(kind, lens);
    }

    /// The Look lens's page for `kind` (CL-043's per-kind memory); Basic
    /// at first.
    pub fn look_page(&self, kind: SelectionKind) -> ColorPage {
        self.look_page
            .get(&kind)
            .copied()
            .unwrap_or(ColorPage::Basic)
    }

    pub fn set_look_page(&mut self, kind: SelectionKind, page: ColorPage) {
        self.look_page.insert(kind, page);
    }

    /// The remembered state of `card` for `kind`.
    pub fn card(&mut self, kind: SelectionKind, card: CardId) -> &mut CardMemory {
        self.cards
            .entry((kind, card))
            .or_insert_with(|| CardMemory::new(card.starts_open()))
    }

    /// The state as `workspace.json`'s `inspector` object. Cards still in
    /// their first state are left out.
    pub fn to_json(&self) -> Value {
        let mut lens: Vec<_> = self.lens.iter().map(|(k, l)| (k.key(), l.key())).collect();
        lens.sort_unstable();
        let mut cards: Vec<_> = self
            .cards
            .iter()
            .filter(|((_, card), m)| **m != CardMemory::new(card.starts_open()))
            .map(|((kind, card), m)| (kind.key(), card.key(), m.open, m.more))
            .collect();
        cards.sort_unstable();
        let mut pages: Vec<_> = self
            .look_page
            .iter()
            .map(|(k, p)| (k.key(), page_key(*p)))
            .collect();
        pages.sort_unstable();
        json!({
            "detail": self.detail.key(),
            "lens": lens.into_iter().map(|(k, l)| (k.to_owned(), Value::from(l))).collect::<Map<_, _>>(),
            "look_page": pages.into_iter().map(|(k, p)| (k.to_owned(), Value::from(p))).collect::<Map<_, _>>(),
            "cards": cards
                .into_iter()
                .map(|(kind, card, open, more)| {
                    json!({ "kind": kind, "card": card, "open": open, "more": more })
                })
                .collect::<Vec<_>>(),
        })
    }

    /// The state in `value` (an `inspector` object). Unknown or damaged
    /// entries are skipped, keeping their defaults.
    pub fn from_json(value: &Value) -> Workspace {
        let mut out = Workspace::default();
        if let Some(d) = value
            .get("detail")
            .and_then(Value::as_str)
            .and_then(Detail::from_key)
        {
            out.detail = d;
        }
        if let Some(lens) = value.get("lens").and_then(Value::as_object) {
            for (kind, l) in lens {
                if let (Some(kind), Some(l)) = (
                    SelectionKind::from_key(kind),
                    l.as_str().and_then(Lens::from_key),
                ) {
                    out.lens.insert(kind, l);
                }
            }
        }
        if let Some(pages) = value.get("look_page").and_then(Value::as_object) {
            for (kind, p) in pages {
                if let (Some(kind), Some(p)) = (
                    SelectionKind::from_key(kind),
                    p.as_str().and_then(page_from_key),
                ) {
                    out.look_page.insert(kind, p);
                }
            }
        }
        for card in value
            .get("cards")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let key = |name: &str| card.get(name).and_then(Value::as_str);
            let flag = |name: &str| card.get(name).and_then(Value::as_bool);
            if let (Some(kind), Some(id), Some(open), Some(more)) = (
                key("kind").and_then(SelectionKind::from_key),
                key("card").and_then(CardId::from_key),
                flag("open"),
                flag("more"),
            ) {
                out.cards.insert((kind, id), CardMemory { open, more });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_ids_depend_on_kind_card_and_row_only() {
        // Two different text clips selected one after the other get the
        // same ids, so state keyed by them survives the selection change.
        let size = widget_id(SelectionKind::Text, CardId::Type, "size");
        assert_eq!(size, widget_id(SelectionKind::Text, CardId::Type, "size"));
        assert_ne!(size, widget_id(SelectionKind::Text, CardId::Type, "font"));
        assert_ne!(size, widget_id(SelectionKind::Text, CardId::Fill, "size"));
        assert_ne!(
            size,
            widget_id(SelectionKind::Several, CardId::Type, "size")
        );
        let placement = widget_id(SelectionKind::Video, CardId::Placement, "opacity");
        assert_ne!(
            placement,
            widget_id(SelectionKind::Image, CardId::Placement, "opacity")
        );
    }

    #[test]
    fn text_cards_map_to_the_models_cards_and_layers() {
        for card in TextCard::ALL {
            let id = [
                CardId::Text,
                CardId::Paragraph,
                CardId::Type,
                CardId::Fill,
                CardId::Spacing,
                CardId::Outline,
                CardId::Shadow,
                CardId::Glow,
                CardId::Background,
                CardId::Bubble,
                CardId::Curve,
            ]
            .into_iter()
            .find(|c| c.text_card() == Some(card));
            assert!(id.is_some(), "{card:?} has an inspector card");
            let lens = [Lens::Content, Lens::Style, Lens::BoxBubble]
                .into_iter()
                .find(|l| l.text_lens() == Some(card.lens()));
            assert!(lens.is_some_and(|l| TEXT_LENSES.contains(&l)));
        }
        assert_eq!(CardId::Glow.layer(), Some(TextLayerKind::Glow));
        assert_eq!(CardId::Type.layer(), None);
        assert_eq!(CardId::Placement.text_card(), None);
    }

    /// UI-002 / UI-008: the workspace state goes through `workspace.json`
    /// and comes back the same; damaged entries keep their defaults.
    #[test]
    fn workspace_state_round_trips_through_json() {
        let mut ws = Workspace {
            detail: Detail::Everything,
            ..Workspace::default()
        };
        ws.set_lens(SelectionKind::Text, Lens::BoxBubble);
        ws.set_lens(SelectionKind::Video, Lens::Sound);
        ws.card(SelectionKind::Text, CardId::Type).open = false;
        ws.card(SelectionKind::Text, CardId::Shadow).more = true;
        // Looked at but unchanged: not written.
        ws.card(SelectionKind::Text, CardId::Fill);
        let json = ws.to_json();
        assert_eq!(json["cards"].as_array().map(Vec::len), Some(2));
        let back = Workspace::from_json(&json);
        let mut expected = ws.clone();
        expected.cards.retain(|(_, c), _| *c != CardId::Fill);
        assert_eq!(back, expected);

        let damaged = json!({
            "detail": "sometimes",
            "lens": { "text": "colour", "hologram": "style", "image": "picture" },
            "cards": [{ "kind": "text", "card": "type" }, 7],
        });
        let mut want = Workspace::default();
        want.set_lens(SelectionKind::Image, Lens::Picture);
        assert_eq!(Workspace::from_json(&damaged), want);
        for kind in SelectionKind::ALL {
            assert_eq!(SelectionKind::from_key(kind.key()), Some(*kind));
        }
        for card in CardId::ALL {
            assert_eq!(CardId::from_key(card.key()), Some(*card));
        }
    }

    #[test]
    fn lens_and_card_memory_follow_the_selection_kind() {
        let mut ws = Workspace::default();
        let all = Holds {
            text: true,
            visual: true,
            color: true,
        };
        let pictures = Holds {
            visual: true,
            color: true,
            ..Holds::default()
        };
        let text = lenses(SelectionKind::Text, all);
        assert_eq!(text, TEXT_LENSES);
        assert_eq!(ws.lens(SelectionKind::Text, &text), Some(Lens::Content));
        ws.set_lens(SelectionKind::Text, Lens::Style);
        assert_eq!(ws.lens(SelectionKind::Text, &text), Some(Lens::Style));
        // A kind without that lens falls back to its first.
        ws.set_lens(SelectionKind::Video, Lens::Sound);
        let image = lenses(SelectionKind::Image, pictures);
        assert_eq!(ws.lens(SelectionKind::Image, &image), Some(Lens::Picture));
        assert_eq!(
            ws.lens(
                SelectionKind::Video,
                &lenses(SelectionKind::Video, pictures)
            ),
            Some(Lens::Sound)
        );
        assert_eq!(
            lenses(SelectionKind::Several, all),
            [
                Lens::Content,
                Lens::Style,
                Lens::Picture,
                Lens::Look,
                Lens::Sound
            ]
        );
        assert_eq!(
            lenses(SelectionKind::Several, Holds::default()),
            [Lens::Sound]
        );
        // Adjustment clips show the Look lens alone (ui-concept §6.3).
        assert_eq!(lenses(SelectionKind::Adjustment, pictures), [Lens::Look]);
        assert_eq!(ws.look_page(SelectionKind::Video), ColorPage::Basic);
        ws.set_look_page(SelectionKind::Video, ColorPage::Curves);
        let back = Workspace::from_json(&ws.to_json());
        assert_eq!(back.look_page(SelectionKind::Video), ColorPage::Curves);
        assert_eq!(back.look_page(SelectionKind::Image), ColorPage::Basic);

        // Collapse state is per (kind, card).
        assert!(ws.card(SelectionKind::Text, CardId::Type).open);
        assert!(!ws.card(SelectionKind::Text, CardId::ClipInfo).open);
        ws.card(SelectionKind::Text, CardId::Type).open = false;
        assert!(!ws.card(SelectionKind::Text, CardId::Type).open);
        assert!(ws.card(SelectionKind::Several, CardId::Type).open);
    }
}
