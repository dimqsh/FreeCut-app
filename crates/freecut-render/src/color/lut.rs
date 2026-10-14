//! The user LUT stage (CL-017, stage 13) and where its tables come from
//! (render-compositor.md §17): a [`LutSource`] turns a [`LutRef`]'s hash
//! into a parsed [`Lut`]. The app's source looks in FreeCut's bundled set
//! first (CL-045), then the user's library (CL-016), and parses on first
//! use with `freecut_media::lut`; tests use a [`LutSet`].
//!
//! A LUT that its source doesn't have renders as identity (CL-017's edge
//! case) and is reported, so the app can badge the clip and the export
//! preflight can list it.

use std::collections::HashMap;
use std::sync::Arc;

use freecut_media::lut::Lut;
use freecut_timeline::model::{ColorStack, LutInterp, LutRef, LutSlot, Section, SkinScope};

/// Where LUT tables come from, by the hash of their file
/// ([`LutRef::hash`]). Called on render threads; an implementation may read
/// and parse a file the first time a hash is asked for, and should keep the
/// result.
pub trait LutSource: Send + Sync + std::fmt::Debug {
    /// The parsed table of the file with this hash; `None` when it can't be
    /// found or read.
    fn lut(&self, hash: &[u8; 16]) -> Option<Arc<Lut>>;
}

/// A source with no tables: every LUT is missing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLuts;

impl LutSource for NoLuts {
    fn lut(&self, _hash: &[u8; 16]) -> Option<Arc<Lut>> {
        None
    }
}

/// Tables held in memory, by hash.
#[derive(Debug, Clone, Default)]
pub struct LutSet {
    tables: HashMap<[u8; 16], Arc<Lut>>,
}

impl LutSet {
    pub fn new() -> LutSet {
        LutSet::default()
    }

    /// Adds (or replaces) the table of `hash`.
    pub fn insert(&mut self, hash: [u8; 16], lut: Arc<Lut>) {
        self.tables.insert(hash, lut);
    }

    /// `self` with the table of `hash` added.
    #[must_use]
    pub fn with(mut self, hash: [u8; 16], lut: Lut) -> LutSet {
        self.insert(hash, Arc::new(lut));
        self
    }

    pub fn len(&self) -> usize {
        self.tables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

impl LutSource for LutSet {
    fn lut(&self, hash: &[u8; 16]) -> Option<Arc<Lut>> {
        self.tables.get(hash).cloned()
    }
}

/// The LUT slot that grades anything: present, switched on and above
/// Intensity 0.
pub(crate) fn active_slot(stack: &ColorStack) -> Option<&LutSlot> {
    stack
        .lut
        .as_ref()
        .filter(|l| l.intensity > 0 && stack.sections_enabled.is_on(Section::Lut))
}

/// A stack's LUT looked up in `luts`.
#[derive(Debug, Clone)]
pub struct ResolvedLut {
    /// The table, when the stack's active slot has one and the source found
    /// it.
    pub table: Option<Arc<Lut>>,
    /// The slot's file when its table is missing.
    pub missing: Option<LutRef>,
}

/// Looks up the table of `stack`'s active LUT slot.
pub fn resolve_lut(stack: &ColorStack, luts: &dyn LutSource) -> ResolvedLut {
    match active_slot(stack) {
        None => ResolvedLut {
            table: None,
            missing: None,
        },
        Some(slot) => match luts.lut(&slot.lut.hash) {
            Some(table) => ResolvedLut {
                table: Some(table),
                missing: None,
            },
            None => ResolvedLut {
                table: None,
                missing: Some(slot.lut.clone()),
            },
        },
    }
}

/// Stage 13 with its constants worked out: the table, its sampling, the
/// Intensity and the LUT scope's skin protection.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LutStage {
    table: Arc<Lut>,
    trilinear: bool,
    intensity: f32,
    /// The LUT scope's skin Amount, 0–1, while it is active.
    pub(crate) skin: Option<f32>,
}

impl LutStage {
    /// The stage of `stack`'s active slot with `table` as its file's table.
    pub(crate) fn new(stack: &ColorStack, table: Arc<Lut>) -> Option<LutStage> {
        let slot = active_slot(stack)?;
        let skin = stack.skin(SkinScope::Lut);
        Some(LutStage {
            table,
            trilinear: slot.interp == LutInterp::Trilinear,
            intensity: f32::from(slot.intensity.min(100)) / 100.0,
            skin: skin
                .is_active()
                .then(|| f32::from(skin.amount.min(100)) / 100.0),
        })
    }

    /// The table alone on one pixel.
    #[inline]
    pub(crate) fn sample(&self, c: [f32; 3]) -> [f32; 3] {
        if self.trilinear {
            self.table.apply_trilinear(c)
        } else {
            self.table.apply_tetrahedral(c)
        }
    }

    /// `lerp(c, LUT(c), intensity × (1 − mask·amount))` (CL-017, CL-030);
    /// `mask` is the pixel's skin mask (0 without protection).
    #[inline]
    pub(crate) fn apply(&self, c: [f32; 3], mask: f32) -> [f32; 3] {
        let t = match self.skin {
            Some(a) => self.intensity * (1.0 - mask * a),
            None => self.intensity,
        };
        let l = self.sample(c);
        if t >= 1.0 {
            return l;
        }
        [
            c[0] + t * (l[0] - c[0]),
            c[1] + t * (l[1] - c[1]),
            c[2] + t * (l[2] - c[2]),
        ]
    }

    /// The 3D table's lattice size (0 for a 1D-only LUT).
    pub(crate) fn cube_size(&self) -> u32 {
        self.table.cube.as_ref().map_or(0, |c| c.size())
    }
}
