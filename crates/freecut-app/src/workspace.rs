//! Workspace state (UI-002): `workspace.json` next to `settings.json`.
//!
//! What it holds so far is the inspector's: the Detail switch, the last
//! lens per selection kind, and each card's collapse and *More* state per
//! (kind, card) (UI-008), under the `inspector` key. Other keys, such as
//! those a newer FreeCut writes, are kept as found.
//!
//! The file is read once at startup on a helper thread ([`load`]) and
//! written [`SAVE_DELAY`] after the last change, atomically, on a writer
//! thread. A file that can't be read, isn't a JSON object or comes from a
//! newer format is ignored: the workspace starts from its defaults and the
//! file is left alone until the next change replaces it (UI-002).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::TryRecvError;
use serde_json::{Map, Value};

use crate::settings::{WriteJob, Writer};

/// The workspace file's name.
pub const FILE_NAME: &str = "workspace.json";
/// The format version this build writes.
pub const VERSION: u64 = 1;
/// How long after the last change the file is written (UI-002).
pub const SAVE_DELAY: Duration = Duration::from_secs(1);
/// The key of the inspector's state.
pub const INSPECTOR: &str = "inspector";

/// The workspace file of this machine's user (no file I/O).
pub fn workspace_path() -> Option<PathBuf> {
    crate::settings::settings_path().map(|p| p.with_file_name(FILE_NAME))
}

/// Blocking: the JSON object at `path`; `None` when there is none, it
/// can't be read, or a newer FreeCut wrote it.
pub fn load(path: &Path) -> Option<Map<String, Value>> {
    let bytes = std::fs::read(path).ok()?;
    let Ok(Value::Object(raw)) = serde_json::from_slice::<Value>(&bytes) else {
        return None;
    };
    let version = raw.get("version").and_then(Value::as_u64).unwrap_or(0);
    (version <= VERSION).then_some(raw)
}

/// The workspace file and when to write it.
#[derive(Debug, Default)]
pub struct WorkspaceStore {
    /// The file as read or last written.
    raw: Map<String, Value>,
    /// Where it is saved; `None` keeps the state in memory.
    path: Option<PathBuf>,
    /// The time of the first change not yet written.
    changed_at: Option<Instant>,
    writer: Option<Writer>,
    /// Writes queued but not answered yet.
    in_flight: usize,
}

impl WorkspaceStore {
    /// A store over what [`load`] read from `path`.
    pub fn new(raw: Option<Map<String, Value>>, path: PathBuf) -> WorkspaceStore {
        WorkspaceStore {
            raw: raw.unwrap_or_default(),
            path: Some(path),
            ..WorkspaceStore::default()
        }
    }

    /// The value stored under `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.raw.get(key)
    }

    /// Sets `key`: written [`SAVE_DELAY`] after the last change.
    pub fn set(&mut self, key: &str, value: Value, now: Instant) {
        if self.raw.get(key) != Some(&value) {
            self.raw.insert(key.to_owned(), value);
            self.changed_at = Some(now);
        }
    }

    /// Starts the write once [`SAVE_DELAY`] has passed since the last
    /// change, and collects finished writes. Returns when to call again,
    /// if a write is still waiting.
    pub fn tick(&mut self, now: Instant, wake: impl Fn() + Send + 'static) -> Option<Instant> {
        self.collect();
        let due = self.changed_at? + SAVE_DELAY;
        if now < due {
            return Some(due);
        }
        self.changed_at = None;
        self.write(wake);
        None
    }

    /// Writes whatever changed and waits up to `timeout` for the writer
    /// (the window is closing).
    pub fn flush_and_wait(&mut self, timeout: Duration) {
        if self.changed_at.take().is_some() {
            self.write(|| {});
        }
        let deadline = Instant::now() + timeout;
        while self.in_flight > 0 {
            let Some(w) = &self.writer else { return };
            let left = deadline.saturating_duration_since(Instant::now());
            match w.done.recv_timeout(left) {
                Ok(result) => {
                    self.in_flight -= 1;
                    if let Err(e) = result {
                        eprintln!("FreeCut: can't save the workspace: {e}");
                    }
                }
                Err(_) => return,
            }
        }
    }

    fn write(&mut self, wake: impl Fn() + Send + 'static) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.raw.insert("version".to_owned(), Value::from(VERSION));
        let Ok(bytes) = serde_json::to_vec_pretty(&Value::Object(self.raw.clone())) else {
            return;
        };
        if self.writer.is_none() {
            match Writer::spawn("freecut-workspace", wake) {
                Ok(w) => self.writer = Some(w),
                Err(e) => {
                    eprintln!("FreeCut: can't save the workspace: {e}");
                    return;
                }
            }
        }
        if let Some(w) = &self.writer
            && w.jobs.send(WriteJob { path, bytes }).is_ok()
        {
            self.in_flight += 1;
        }
    }

    /// Picks up finished writes. A failure keeps the workspace in memory
    /// from then on (it is a convenience: no warning beyond the log).
    fn collect(&mut self) {
        let Some(w) = &self.writer else { return };
        loop {
            match w.done.try_recv() {
                Ok(result) => {
                    self.in_flight = self.in_flight.saturating_sub(1);
                    if let Err(e) = result
                        && self.path.take().is_some()
                    {
                        eprintln!("FreeCut: can't save the workspace: {e}");
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.writer = None;
                    self.in_flight = 0;
                    return;
                }
            }
        }
    }

    /// Whether a write is waiting or running.
    #[cfg(test)]
    fn busy(&self) -> bool {
        self.changed_at.is_some() || self.in_flight > 0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use serde_json::json;

    use super::*;
    use crate::ui::inspector::lens::{CardId, Detail, SelectionKind, Workspace};

    fn wait_idle(store: &mut WorkspaceStore) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while store.busy() {
            assert!(Instant::now() < deadline, "the write never finished");
            store.tick(Instant::now() + SAVE_DELAY, || {});
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// UI-002 / UI-008: the inspector's state is written a second after
    /// the last change and comes back at the next start; other keys stay.
    #[test]
    fn the_inspector_state_is_saved_after_a_second_and_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("FreeCut").join(FILE_NAME);
        assert_eq!(load(&path), None, "no file yet");
        let mut store = WorkspaceStore::new(None, path.clone());
        let mut ws = Workspace::default();
        ws.card(SelectionKind::Text, CardId::Type).open = false;
        ws.detail = Detail::Everything;
        let start = Instant::now();
        store.set(INSPECTOR, ws.to_json(), start);
        assert_eq!(store.tick(start, || {}), Some(start + SAVE_DELAY));
        assert!(!path.exists(), "not before the delay");
        assert_eq!(store.tick(start + SAVE_DELAY, || {}), None);
        wait_idle(&mut store);

        let raw = load(&path).unwrap();
        assert_eq!(raw["version"], json!(VERSION));
        assert_eq!(Workspace::from_json(&raw[INSPECTOR]), ws);
        // Keys of a later FreeCut survive a write.
        let mut raw = raw;
        raw.insert("panels".to_owned(), json!({ "inspector_width": 360 }));
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let mut store = WorkspaceStore::new(load(&path), path.clone());
        ws.detail = Detail::Essentials;
        store.set(INSPECTOR, ws.to_json(), start);
        store.flush_and_wait(Duration::from_secs(10));
        let raw = load(&path).unwrap();
        assert_eq!(raw["panels"], json!({ "inspector_width": 360 }));
        assert_eq!(
            Workspace::from_json(&raw[INSPECTOR]).detail,
            Detail::Essentials
        );
    }

    /// UI-002: an unreadable file or one from a newer format is ignored.
    #[test]
    fn a_damaged_or_newer_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(load(&path), None);
        std::fs::write(&path, br#"{ "version": 99, "inspector": {} }"#).unwrap();
        assert_eq!(load(&path), None);
        std::fs::write(&path, br#"[1, 2]"#).unwrap();
        assert_eq!(load(&path), None);
    }
}
