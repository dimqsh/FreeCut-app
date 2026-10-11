//! The animation engine's pure parts (FX-018, FX-019, FX-030, TX-022,
//! TX-023, TX-056): preset manifests, their evaluation into offsets
//! (seeded channels included), TX-022's seam envelope, the
//! photosensitivity checker and Pace. Per-unit application to glyphs needs
//! the layout and lives in `freecut-render::text::animate`; it poses each
//! unit with [`pose_with`] (or [`crate::plan::TextAnimPlan::unit_pose`]).

mod eval;
mod flash;
mod manifest;
mod pace;
mod seeded;

pub use eval::{
    BlockOffsets, MotionScale, RestBounds, UnitContext, animation_offsets, animation_offsets_with,
    effective_direction, effective_unit, pose_at, pose_with, seam_envelope, slot_progress,
    travel_px,
};
pub use flash::{
    Photosensitivity, is_photosafe, max_flashes_per_second, min_safe_cycle, photosensitivity,
};
pub use manifest::{
    Availability, Channel, ChannelProp, ChannelUnit, Curve, CurvePoint, ManifestError, NoPresets,
    ParamDefaults, Pivot, PresetCatalog, PresetKind, PresetManifest, PresetTable, StyleChannel,
    StyleUse, samples,
};
pub use pace::{count_units, pace_duration, pace_unit, with_paced_duration};
