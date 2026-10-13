//! Codepoints of the Phosphor icon font (MIT, `assets/fonts/Phosphor.ttf`
//! and `Phosphor-Fill.ttf`, release 2.1.2), our own constants so FreeCut
//! doesn't depend on a wrapper crate (ui-concept §14). Both weights use the
//! same codepoints. The names after each constant are Phosphor's.
//!
//! Icons render in any label (the icon font is in every text family's
//! fallback chain) or alone with [`super::fonts::icon_font`].

/// `film-strip`: the Media tab.
pub const MEDIA: &str = "\u{e792}";
/// `music-notes`: the Audio tab.
pub const AUDIO: &str = "\u{e340}";
/// `music-note`: an audio item.
pub const MUSIC_NOTE: &str = "\u{e33c}";
/// `text-t`: the Text tab and text clips.
pub const TEXT: &str = "\u{e48a}";
/// `film-slate`: the project.
pub const PROJECT: &str = "\u{e8c2}";
/// `shapes`: stickers, effects and adjustment clips.
pub const SHAPES: &str = "\u{ec5e}";
/// `caret-down`: an open card.
pub const CARET_DOWN: &str = "\u{e136}";
/// `caret-right`: a closed card, a submenu.
pub const CARET_RIGHT: &str = "\u{e13a}";
/// `caret-left`: previous keyframe.
pub const CARET_LEFT: &str = "\u{e138}";
/// `dots-three`: a card's `...` menu.
pub const DOTS_THREE: &str = "\u{e1fe}";
/// `arrow-counter-clockwise`: reset to the default.
pub const RESET: &str = "\u{e038}";
/// `arrows-clockwise`: list again.
pub const REFRESH: &str = "\u{e094}";
/// `magnifying-glass`: search fields.
pub const SEARCH: &str = "\u{e30c}";
/// `x`: close.
pub const CLOSE: &str = "\u{e4f6}";
/// `warning`.
pub const WARNING: &str = "\u{e4e0}";
/// `info`.
pub const INFO: &str = "\u{e2ce}";
/// `x-circle`: a problem.
pub const ERROR: &str = "\u{e4f8}";
/// `link`: linked clips.
pub const LINK: &str = "\u{e2e2}";
/// `text-b`: bold.
pub const BOLD: &str = "\u{e5be}";
/// `text-italic`.
pub const ITALIC: &str = "\u{e5c0}";
/// `text-align-left`.
pub const ALIGN_LEFT: &str = "\u{e484}";
/// `text-align-center`.
pub const ALIGN_CENTER: &str = "\u{e480}";
/// `text-align-right`.
pub const ALIGN_RIGHT: &str = "\u{e486}";
/// `text-underline`.
pub const UNDERLINE: &str = "\u{e5c4}";
/// `text-strikethrough`.
pub const STRIKE: &str = "\u{e5c2}";
/// `text-align-justify`.
pub const ALIGN_JUSTIFY: &str = "\u{e482}";
/// `align-top`: the vertical anchor's Top.
pub const ANCHOR_TOP: &str = "\u{e512}";
/// `align-center-vertical`: the vertical anchor's Middle.
pub const ANCHOR_MIDDLE: &str = "\u{e50c}";
/// `align-bottom`: the vertical anchor's Bottom.
pub const ANCHOR_BOTTOM: &str = "\u{e506}";
/// `plus`: a stepper's increase, add a layer or a stop.
pub const PLUS: &str = "\u{e3d4}";
/// `minus`: a stepper's decrease, remove a stop.
pub const MINUS: &str = "\u{e32a}";
/// `diamond`: a keyframe; a card with keys (ui-concept §13.6).
pub const DIAMOND: &str = "\u{e1ec}";
/// `dice-five`: a new random seed.
pub const DICE: &str = "\u{e1ee}";
/// `arrows-left-right`: reverse a gradient.
pub const REVERSE: &str = "\u{e0a0}";
/// `flip-horizontal`.
pub const FLIP_H: &str = "\u{ed6a}";
/// `flip-vertical`.
pub const FLIP_V: &str = "\u{ed6c}";
/// `scissors`: split.
pub const SPLIT: &str = "\u{eae0}";
/// `trash`: delete.
pub const DELETE: &str = "\u{e4a6}";
/// `list-dashes`: the Detail switch's Essentials.
pub const DETAIL_ESSENTIALS: &str = "\u{e2f4}";
/// `list`: the Detail switch's Everything.
pub const DETAIL_EVERYTHING: &str = "\u{e2f0}";
/// `article`: the Content lens.
pub const LENS_CONTENT: &str = "\u{e0a8}";
/// `text-aa`: the Style lens.
pub const LENS_STYLE: &str = "\u{e6ee}";
/// `chat-centered-text`: the Box & bubble lens.
pub const LENS_BOX: &str = "\u{e166}";
/// `path`: the Motion lens.
pub const LENS_MOTION: &str = "\u{e39c}";
/// `squares-four`: the Presets lens.
pub const LENS_PRESETS: &str = "\u{e464}";
/// `image`: the Picture lens.
pub const LENS_PICTURE: &str = "\u{e2ca}";
/// `speaker-high`: the Sound lens.
pub const LENS_SOUND: &str = "\u{e44a}";
/// `export`: the Export button.
pub const EXPORT: &str = "\u{eaf0}";
/// `gear`: Preferences.
pub const GEAR: &str = "\u{e270}";
/// `skip-back`: go to start.
pub const GO_START: &str = "\u{e5a4}";
/// `skip-forward`: go to end.
pub const GO_END: &str = "\u{e5a6}";
/// `play`.
pub const PLAY: &str = "\u{e3d0}";
/// `pause`.
pub const PAUSE: &str = "\u{e39e}";
/// `speaker-high`: preview sound on.
pub const SPEAKER: &str = "\u{e44a}";
/// `speaker-slash`: preview sound muted.
pub const SPEAKER_MUTED: &str = "\u{e45a}";
/// `corners-in`: fit the canvas to the Stage.
pub const FIT: &str = "\u{e1ce}";

/// `download-simple`: the Import button (§13.27).
pub const IMPORT: &str = "\u{e20c}";
/// `clock`: sort by date added.
pub const SORT_ADDED: &str = "\u{e19a}";
/// `text-a-underline`: sort by name.
pub const SORT_NAME: &str = "\u{ed34}";
/// `tag`: sort by kind.
pub const SORT_KIND: &str = "\u{e478}";
/// `check`: the current option of a select.
pub const CHECK: &str = "\u{e182}";
/// `video`: a video item without a poster.
pub const VIDEO: &str = "\u{e740}";
/// `image`: a picture item without a poster.
pub const IMAGE: &str = "\u{e2ca}";

/// `eye`: a shown track (R5 track headers).
pub const EYE: &str = "\u{e220}";
/// `eye-slash`: a hidden track.
pub const EYE_SLASH: &str = "\u{e224}";
/// `lock-simple`: a locked track.
pub const LOCK: &str = "\u{e308}";
/// `lock-simple-open`: an unlocked track.
pub const LOCK_OPEN: &str = "\u{e30a}";
/// `headphones`: solo (AU-012).
pub const SOLO: &str = "\u{e2a6}";
/// `magnet`: the magnetic main track (TL-005).
pub const MAGNET: &str = "\u{e680}";
/// `arrows-in-line-horizontal`: snapping (TL-028).
pub const SNAP: &str = "\u{e530}";
/// `arrow-u-up-left`: undo.
pub const UNDO: &str = "\u{e08a}";
/// `arrow-u-up-right`: redo.
pub const REDO: &str = "\u{e08c}";
/// `magnifying-glass-plus`: zoom in.
pub const ZOOM_IN: &str = "\u{e310}";
/// `magnifying-glass-minus`: zoom out.
pub const ZOOM_OUT: &str = "\u{e30e}";
/// `arrows-out-line-horizontal`: zoom to fit the sequence.
pub const ZOOM_FIT: &str = "\u{e534}";
/// `faders`: the Mixer.
pub const MIXER: &str = "\u{e228}";
/// `waveform`: audio tracks.
pub const WAVEFORM: &str = "\u{e802}";
/// `sticker`: sticker tracks and clips.
pub const STICKER: &str = "\u{e5ac}";
/// `sparkle`: effect tracks and clips.
pub const EFFECT: &str = "\u{e6a2}";
/// `sliders-horizontal`: adjustment tracks and clips.
pub const ADJUST: &str = "\u{e434}";
/// `sliders-horizontal`: the Look lens (colour, CL-043).
pub const LENS_LOOK: &str = ADJUST;

/// `folder-open`: Open project…, Show in folder.
pub const FOLDER_OPEN: &str = "\u{e256}";
/// `file-plus`: New project.
pub const FILE_PLUS: &str = "\u{e236}";
/// `squares-four`: the Projects page of Home.
pub const GRID: &str = "\u{e464}";
/// `copy`: Copy details, Copy path.
pub const COPY: &str = "\u{e1ca}";
/// `check-circle`: success (a finished export, a success toast).
pub const CHECK_CIRCLE: &str = "\u{e184}";
/// `palette`: Preferences › Appearance.
pub const PALETTE: &str = "\u{e6c8}";
/// `speaker-high`: Preferences › Playback & audio.
pub const SPEAKER_HIGH: &str = "\u{e44a}";
/// `file-video`: a project card without a cover.
pub const FILE_VIDEO: &str = "\u{ea22}";
/// `arrow-square-out`: open in another app.
pub const OPEN_OUT: &str = "\u{e5de}";

/// Every icon, for the font coverage test.
#[cfg(test)]
pub const ALL: &[&str] = &[
    MEDIA,
    AUDIO,
    MUSIC_NOTE,
    TEXT,
    PROJECT,
    SHAPES,
    CARET_DOWN,
    CARET_RIGHT,
    CARET_LEFT,
    DOTS_THREE,
    RESET,
    REFRESH,
    SEARCH,
    CLOSE,
    WARNING,
    INFO,
    ERROR,
    LINK,
    BOLD,
    ITALIC,
    ALIGN_LEFT,
    ALIGN_CENTER,
    ALIGN_RIGHT,
    UNDERLINE,
    STRIKE,
    ALIGN_JUSTIFY,
    ANCHOR_TOP,
    ANCHOR_MIDDLE,
    ANCHOR_BOTTOM,
    PLUS,
    MINUS,
    DIAMOND,
    DICE,
    REVERSE,
    FLIP_H,
    FLIP_V,
    SPLIT,
    DELETE,
    DETAIL_ESSENTIALS,
    DETAIL_EVERYTHING,
    LENS_CONTENT,
    LENS_STYLE,
    LENS_BOX,
    LENS_MOTION,
    LENS_PRESETS,
    LENS_PICTURE,
    LENS_SOUND,
    EXPORT,
    GEAR,
    GO_START,
    GO_END,
    PLAY,
    PAUSE,
    SPEAKER,
    SPEAKER_MUTED,
    FIT,
    IMPORT,
    SORT_ADDED,
    SORT_NAME,
    SORT_KIND,
    CHECK,
    VIDEO,
    IMAGE,
    EYE,
    EYE_SLASH,
    LOCK,
    LOCK_OPEN,
    SOLO,
    MAGNET,
    SNAP,
    UNDO,
    REDO,
    ZOOM_IN,
    ZOOM_OUT,
    ZOOM_FIT,
    MIXER,
    WAVEFORM,
    STICKER,
    EFFECT,
    ADJUST,
    FOLDER_OPEN,
    FILE_PLUS,
    GRID,
    COPY,
    CHECK_CIRCLE,
    PALETTE,
    SPEAKER_HIGH,
    FILE_VIDEO,
    OPEN_OUT,
    CURSOR,
    BLADE,
    HAND,
    FLAG,
    MICROPHONE,
    FOLLOW,
    SKIM,
];
/// `cursor`: the Select tool (TL-024).
pub const CURSOR: &str = "\u{e1dc}";
/// `knife`: the Blade tool (TL-024).
pub const BLADE: &str = "\u{e636}";
/// `hand`: the Hand tool (TL-024).
pub const HAND: &str = "\u{e298}";
/// `flag`: markers (TL-037).
pub const FLAG: &str = "\u{e244}";
/// `microphone`: Record voiceover (AU-027).
pub const MICROPHONE: &str = "\u{e326}";
/// `link-simple-horizontal`: *Follow main track* (TL-029).
pub const FOLLOW: &str = "\u{e2ea}";
/// `cursor-click`: hover skim (TL-014).
pub const SKIM: &str = "\u{e7c8}";
