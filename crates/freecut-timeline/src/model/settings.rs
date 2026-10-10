//! Project settings (PM-004, PM-005).

use crate::time::{FrameRate, SampleRate, TimeError};

/// Canvas and timing settings shared by every sequence of a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectSettings {
    /// Canvas size in pixels; both even and positive.
    pub width: u32,
    pub height: u32,
    /// The timeline frame grid (TL-001).
    pub frame_rate: FrameRate,
    pub sample_rate: SampleRate,
    /// Adopt the first clip's size and rate when it is placed (PM-006).
    pub match_first_clip: bool,
}

/// Largest canvas side (8K plus headroom).
pub const MAX_CANVAS_SIDE: u32 = 8_192;

impl ProjectSettings {
    /// 1920×1080, 30 fps, 48 kHz (PM-004, PM-005 defaults).
    pub fn new_default() -> Result<ProjectSettings, TimeError> {
        Ok(ProjectSettings {
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(30, 1)?,
            sample_rate: SampleRate::new(48_000)?,
            match_first_clip: false,
        })
    }

    /// Whether the canvas size is usable: even, positive, within limits.
    pub fn canvas_is_valid(&self) -> bool {
        let ok = |side: u32| side > 0 && side.is_multiple_of(2) && side <= MAX_CANVAS_SIDE;
        ok(self.width) && ok(self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let s = ProjectSettings::new_default().unwrap();
        assert!(s.canvas_is_valid());
        assert_eq!((s.frame_rate.num(), s.frame_rate.den()), (30, 1));
    }

    #[test]
    fn odd_or_oversized_canvases_are_invalid() {
        let mut s = ProjectSettings::new_default().unwrap();
        s.width = 1921;
        assert!(!s.canvas_is_valid());
        s.width = 0;
        assert!(!s.canvas_is_valid());
        s.width = 10_000;
        assert!(!s.canvas_is_valid());
    }
}
