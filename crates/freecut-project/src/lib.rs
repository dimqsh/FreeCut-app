//! FreeCut project persistence: the `.freecut` file format, migrations, the
//! open and save pipelines, the single-writer lock and portable media paths.
//!
//! # Overview
//!
//! - [`open`] / [`decode`]: file → [`Opened`] (a validated
//!   `freecut_timeline::model::Document`, the id allocator to resume, unknown
//!   keys to keep, the project's [`UiState`], warnings about repairs). Pipeline: parse → envelope check
//!   → [`migrate`] → typed decode with JSON-path errors → map through the
//!   timeline constructors → repair → validate.
//! - [`save`] / [`encode`]: document → canonical bytes → atomic replace with
//!   verify-after-write. [`save_with`] takes a [`fs::FileSystem`] so tests
//!   can inject failures.
//! - [`lock::ProjectLock`]: the `<name>.freecut.lock` sidecar that keeps two
//!   FreeCut processes from editing one project (PM-013).
//! - [`paths`]: building a portable `StoredPath` for a media file and
//!   resolving it again after the project moved (PM-100).
//! - [`export`]: export settings (saved with the project), output sizes,
//!   bitrates, file names and the preflight checks (EX-021…EX-037).
//! - [`presets`]: the animation preset manifest format (FX-030, TX-056),
//!   its parser and the presets compiled in from `resources/`.
//! - [`recordings`]: where voiceover takes are written and the startup scan
//!   that recovers takes interrupted by a crash (AU-027, PM-041).
//! - [`luts`]: the user's LUT library in app data, an index and hashed
//!   copies (CL-016); it never parses a LUT.
//!
//! # Design rules
//!
//! - The timeline model is serde-free. Private DTOs define the file format
//!   and are mapped by hand (README C4), so a model refactor can't change the
//!   format by accident.
//! - Time is integer ticks in the project file, which has no floats at
//!   all. Preset manifests hold floats only for amplitudes and curve
//!   points (FX-030's non-time scalars); their times are whole
//!   milliseconds.
//! - Unknown keys survive open → save ([`Extras`]).
//! - Everything here is blocking and meant for worker threads; nothing keeps
//!   global state or changes the current directory.
#![forbid(unsafe_code)]

pub mod export;
mod extras;
mod format;
pub mod fs;
pub mod lock;
pub mod luts;
pub mod migrate;
mod names;
mod open;
pub mod paths;
pub mod presets;
pub mod recordings;
mod repair;
mod save;
mod ui;

pub use extras::{Extras, Fields, Owner};
pub use format::{APP_ID, CURRENT_FORMAT_VERSION, FORMAT_MAGIC};
pub use open::{MAX_FILE_BYTES, OpenError, OpenWarning, Opened, decode, open};
pub use save::{
    SaveError, SaveRequest, SaveStep, encode, encode_with_export, encode_with_ui, save, save_with,
    write_atomic,
};
pub use ui::UiState;
