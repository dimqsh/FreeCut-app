//! The font registry: bundled fonts, always available, plus the system
//! fonts found by a background scan (TX-003).
//!
//! The bundled Noto Sans faces are compiled into the binary, so text renders
//! with zero system fonts (CI, minimal installs, golden tests). System
//! discovery (fontique: CoreText, DirectWrite or fontconfig) runs on its own
//! thread; until it finishes, the registry serves the bundled fonts only.
//!
//! **Precedence** (TX-003): a system family hides a bundled family of the
//! same name, so an installed (often newer, or variable) Noto Sans is used
//! with all its faces. The bundled faces of a family are registered only
//! when the scanned system set doesn't have that family. (fontique looks up
//! registered fonts before system ones, so registering them anyway would
//! shadow the system family.)
//! Each published font set bumps a generation number, which
//! [`TextRenderer`](super::TextRenderer)s use to rebuild their font context
//! and drop rasters made with the old set.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use parley::fontique::{Blob, Collection, CollectionOptions};

/// The family every missing font falls back to.
pub const DEFAULT_FAMILY: &str = "Noto Sans";

/// One bundled font file.
#[derive(Debug, Clone, Copy)]
pub struct BundledFont {
    /// File name under `assets/fonts/`.
    pub file: &'static str,
    /// The font's family name.
    pub family: &'static str,
    pub data: &'static [u8],
}

/// The fonts compiled into FreeCut (see `assets/ASSETS.md`; SIL OFL 1.1).
pub const BUNDLED_FONTS: &[BundledFont] = &[
    BundledFont {
        family: DEFAULT_FAMILY,
        file: "NotoSans-Regular.ttf",
        data: include_bytes!("../../../../assets/fonts/NotoSans-Regular.ttf"),
    },
    BundledFont {
        family: DEFAULT_FAMILY,
        file: "NotoSans-Bold.ttf",
        data: include_bytes!("../../../../assets/fonts/NotoSans-Bold.ttf"),
    },
    BundledFont {
        family: DEFAULT_FAMILY,
        file: "NotoSans-Italic.ttf",
        data: include_bytes!("../../../../assets/fonts/NotoSans-Italic.ttf"),
    },
    BundledFont {
        family: DEFAULT_FAMILY,
        file: "NotoSans-BoldItalic.ttf",
        data: include_bytes!("../../../../assets/fonts/NotoSans-BoldItalic.ttf"),
    },
];

/// How far system font discovery has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScanState {
    /// Only bundled fonts; no scan was requested.
    BundledOnly,
    /// Bundled fonts are available; the system scan is running.
    Scanning,
    /// System fonts are available too.
    Complete,
}

/// Shared handle to the font set. Cheap to clone; every clone sees the
/// same fonts.
#[derive(Clone)]
pub struct FontRegistry {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for FontRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontRegistry")
            .field("generation", &self.generation())
            .field("scan", &self.scan_state())
            .finish_non_exhaustive()
    }
}

struct Inner {
    /// Bumped (after `state` is updated) whenever a new font set is
    /// published.
    generation: AtomicU64,
    state: Mutex<State>,
    published: Condvar,
}

struct State {
    collection: Collection,
    families: Arc<[String]>,
    scan: ScanState,
}

impl FontRegistry {
    /// Bundled fonts only: deterministic on every machine (tests, goldens).
    pub fn bundled() -> FontRegistry {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        register_bundled(&mut collection);
        FontRegistry::from_state(collection, ScanState::BundledOnly)
    }

    /// Bundled fonts now, system fonts as soon as a background scan
    /// finishes. Never blocks the caller on the scan.
    pub fn with_system_fonts() -> FontRegistry {
        let registry = FontRegistry::bundled();
        registry.lock().scan = ScanState::Scanning;
        registry.start_scan();
        registry
    }

    fn from_state(mut collection: Collection, scan: ScanState) -> FontRegistry {
        let families = family_list(&mut collection);
        FontRegistry {
            inner: Arc::new(Inner {
                generation: AtomicU64::new(1),
                state: Mutex::new(State {
                    collection,
                    families,
                    scan,
                }),
                published: Condvar::new(),
            }),
        }
    }

    /// Scans system fonts again (TX-007 Rescan), off the caller's thread.
    pub fn rescan(&self) {
        self.lock().scan = ScanState::Scanning;
        self.start_scan();
    }

    fn start_scan(&self) {
        let inner = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("freecut-fonts".into())
            .spawn(move || {
                let mut collection = Collection::new(CollectionOptions {
                    shared: false,
                    system_fonts: true,
                });
                register_bundled_unless_present(&mut collection);
                let families = family_list(&mut collection);
                let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.collection = collection;
                state.families = families;
                state.scan = ScanState::Complete;
                inner.generation.fetch_add(1, Ordering::AcqRel);
                inner.published.notify_all();
            });
        if let Err(e) = spawned {
            tracing::warn!("could not start the font scan thread: {e}");
            self.lock().scan = ScanState::BundledOnly;
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while publishing leaves a consistent state (the fields are
        // replaced, not mutated in place), so poisoning is ignored.
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Changes whenever the available fonts change.
    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::Acquire)
    }

    pub fn scan_state(&self) -> ScanState {
        self.lock().scan
    }

    /// Blocks until the system scan has finished (or `timeout` passes).
    /// Export calls this so its output never depends on scan timing.
    /// Returns the state reached.
    pub fn wait_for_scan(&self, timeout: Duration) -> ScanState {
        let state = self.lock();
        let (state, _) = self
            .inner
            .published
            .wait_timeout_while(state, timeout, |s| s.scan == ScanState::Scanning)
            .unwrap_or_else(PoisonError::into_inner);
        state.scan
    }

    /// Family names for a font picker: sorted case-insensitively,
    /// deduplicated, hidden system families (leading `.`) left out.
    pub fn family_names(&self) -> Arc<[String]> {
        Arc::clone(&self.lock().families)
    }

    /// Whether a family with this name exists (names compare
    /// case-insensitively, as fontique does).
    pub fn has_family(&self, name: &str) -> bool {
        self.lock().collection.family_id(name).is_some()
    }

    /// The current font set and its generation.
    pub(crate) fn snapshot(&self) -> (u64, Collection) {
        let state = self.lock();
        // Read under the lock: the publisher bumps the generation while
        // holding it, so the pair is consistent.
        (self.generation(), state.collection.clone())
    }
}

fn register_bundled(collection: &mut Collection) {
    for font in BUNDLED_FONTS {
        let registered = collection.register_fonts(Blob::new(Arc::new(font.data)), None);
        if registered.is_empty() {
            tracing::error!("bundled font {} could not be parsed", font.file);
        }
    }
}

/// Registers the bundled fonts whose family `collection` (the scanned
/// system fonts) doesn't already have.
fn register_bundled_unless_present(collection: &mut Collection) {
    // Decided before registering anything, so one bundled face doesn't hide
    // the others of its family.
    let present: Vec<bool> = BUNDLED_FONTS
        .iter()
        .map(|font| collection.family_id(font.family).is_some())
        .collect();
    for (font, present) in BUNDLED_FONTS.iter().zip(present) {
        if present {
            tracing::debug!(
                "bundled font {} not registered: the system has {}",
                font.file,
                font.family
            );
            continue;
        }
        if collection
            .register_fonts(Blob::new(Arc::new(font.data)), None)
            .is_empty()
        {
            tracing::error!("bundled font {} could not be parsed", font.file);
        }
    }
}

fn family_list(collection: &mut Collection) -> Arc<[String]> {
    let mut names: Vec<String> = collection
        .family_names()
        .filter(|n| !n.is_empty() && !n.starts_with('.'))
        .map(str::to_owned)
        .collect();
    names.sort_by(|a, b| {
        a.to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b))
    });
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_registry_lists_noto_sans_only() {
        let fonts = FontRegistry::bundled();
        assert_eq!(&*fonts.family_names(), &[DEFAULT_FAMILY.to_owned()]);
        assert!(fonts.has_family("Noto Sans"));
        assert!(fonts.has_family("noto sans"));
        assert!(!fonts.has_family("No Such Family"));
        assert_eq!(fonts.scan_state(), ScanState::BundledOnly);
        assert_eq!(fonts.wait_for_scan(Duration::ZERO), ScanState::BundledOnly);
    }

    #[test]
    fn a_system_family_hides_the_bundled_one_of_the_same_name() {
        let options = CollectionOptions {
            shared: false,
            system_fonts: false,
        };
        let faces = |collection: &mut Collection| {
            let id = collection.family_id(DEFAULT_FAMILY).unwrap();
            collection.family(id).unwrap().fonts().len()
        };
        // Without a system Noto Sans, all bundled faces are registered.
        let mut empty = Collection::new(options);
        register_bundled_unless_present(&mut empty);
        assert_eq!(faces(&mut empty), BUNDLED_FONTS.len());
        // A stand-in for a system "Noto Sans" with a single face: the
        // bundled faces stay out, so only the system family is reachable.
        let mut system = Collection::new(options);
        system.register_fonts(Blob::new(Arc::new(BUNDLED_FONTS[0].data)), None);
        register_bundled_unless_present(&mut system);
        assert_eq!(faces(&mut system), 1);
    }

    #[test]
    fn bundled_families_are_named_correctly() {
        for font in BUNDLED_FONTS {
            let mut c = Collection::new(CollectionOptions {
                shared: false,
                system_fonts: false,
            });
            let got = c.register_fonts(Blob::new(Arc::new(font.data)), None);
            let id = got[0].0;
            assert_eq!(c.family(id).unwrap().name(), font.family, "{}", font.file);
        }
    }

    #[test]
    fn family_list_is_sorted_and_deduplicated() {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        register_bundled(&mut collection);
        // Registering twice must not duplicate the family.
        register_bundled(&mut collection);
        let names = family_list(&mut collection);
        assert_eq!(names.len(), 1);
    }
}
