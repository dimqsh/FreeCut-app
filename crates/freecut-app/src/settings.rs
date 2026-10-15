//! App preferences (UI-018): `settings.json` in the platform config folder.
//!
//! The file is a JSON object. FreeCut reads the keys it knows and keeps
//! every other key, at any depth, exactly as it found it, so settings
//! written by a newer FreeCut survive an older one (UI-018). Known values
//! of the wrong type or out of range fall back to their default; the file
//! is only treated as damaged when it isn't a JSON object at all.
//!
//! Reading happens once at startup on a helper thread ([`load`]). Writing
//! happens [`SAVE_DELAY`] after the last change, atomically (temporary
//! file, then rename), on a writer thread owned by the [`SettingsStore`].
//! A folder that can't be written turns the store into an in-memory one,
//! with a single warning.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};
use freecut_audio::EngineConfig;
use serde_json::{Map, Value};

use crate::jobs::platform::{Os, home_dir};
use crate::skim::{Skim, SkimMode};
use crate::theme::{DensityChoice, ThemeChoice};

/// The settings file's name.
pub const FILE_NAME: &str = "settings.json";
/// The format version this build writes. A file from a newer build keeps
/// its higher number (UI-018: never migrated downward).
pub const SETTINGS_VERSION: u64 = 1;
/// How long after the last change the file is written (UI-018).
pub const SAVE_DELAY: Duration = Duration::from_millis(500);

/// Shown once when the file was damaged and set aside (UI-018).
pub const RESET_NOTICE: &str = "Preferences were reset because the file was damaged";
/// Shown once when the settings folder can't be written (UI-018).
pub const READ_ONLY_NOTICE: &str =
    "Preferences can't be saved (the settings folder is read-only); changes last until you quit";

/// The settings FreeCut knows. Keys are `section.name` in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub playback: PlaybackSettings,
    pub appearance: AppearanceSettings,
    pub timeline: TimelineSettings,
    pub voiceover: VoiceoverSettings,
}

/// The timeline's per-user preferences (UI-020).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimelineSettings {
    /// `timeline.skim` and `timeline.skim_last_on` (TL-014): `"off"`
    /// (default), `"picture"` or `"picture_and_sound"`.
    pub skim: Skim,
}

/// The voiceover recorder's choices (AU-027), remembered per user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceoverSettings {
    /// `voiceover.input_device`: the input's id as `freecut_audio` lists
    /// it; `None` (`null`) is the system default.
    pub input_device: Option<String>,
    /// `voiceover.countdown_secs`: 0 (off), 3 (default) or 5.
    pub countdown_secs: u8,
    /// `voiceover.preroll_secs`: 0 (default) to 3.
    pub preroll_secs: u8,
    /// `voiceover.channels`: `"mono"` (default) or `"stereo"`.
    pub stereo: bool,
    /// `voiceover.place`: `"first_free"` (default) or `"new_track"`.
    pub new_track: bool,
    /// `voiceover.mute_timeline`: default `false`.
    pub mute_timeline: bool,
    /// `voiceover.monitor` ("Hear yourself"): default `false`.
    pub monitor: bool,
    /// `voiceover.input_level_db`: −20 to +20 dB, in hundredths. Default 0.
    pub input_level: i32,
    /// `voiceover.latency_offset_ms`: −500 to +500. Default 0.
    pub latency_offset_ms: i32,
}

impl Default for VoiceoverSettings {
    fn default() -> VoiceoverSettings {
        VoiceoverSettings {
            input_device: None,
            countdown_secs: 3,
            preroll_secs: 0,
            stereo: false,
            new_track: false,
            mute_timeline: false,
            monitor: false,
            input_level: 0,
            latency_offset_ms: 0,
        }
    }
}

/// The countdowns the recorder offers (AU-027).
pub const COUNTDOWNS: [u8; 3] = [0, 3, 5];
/// The longest pre-roll, in seconds (AU-027).
pub const MAX_PREROLL_SECS: u8 = 3;

/// Preferences ▸ Appearance (UI-014, ui-concept §12.4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppearanceSettings {
    /// `appearance.theme`: `"dark"` (default), `"light"`, `"system"` or
    /// `"high_contrast"`.
    pub theme: ThemeChoice,
    /// `appearance.density`: `"automatic"` (default), `"comfortable"` or
    /// `"compact"`.
    pub density: DensityChoice,
    /// `appearance.reduce_motion` (UI-043): preset tiles show a still
    /// instead of playing on hover; the player still plays the content.
    /// Default `false`.
    pub reduce_motion: bool,
}

/// Preferences ▸ Playback & audio (UI-029, AU-010's settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackSettings {
    /// `playback.output_device`: the device name as `freecut_audio` lists
    /// it; `None` (stored as `null`) is the system default.
    pub output_device: Option<String>,
    /// `playback.buffer_frames`: 256 to 4096 frames. Default 1024.
    pub buffer_frames: u32,
}

impl Default for PlaybackSettings {
    fn default() -> PlaybackSettings {
        PlaybackSettings {
            output_device: None,
            buffer_frames: EngineConfig::DEFAULT_BUFFER_FRAMES,
        }
    }
}

impl PlaybackSettings {
    /// What the audio engine is given.
    pub fn engine_config(&self) -> EngineConfig {
        EngineConfig {
            device: self.output_device.clone(),
            buffer_frames: self.buffer_frames,
        }
    }
}

impl Settings {
    /// The known settings in `raw`; anything missing, mistyped or out of
    /// range takes its default.
    pub fn from_raw(raw: &Map<String, Value>) -> Settings {
        let mut settings = Settings::default();
        if let Some(Value::Object(playback)) = raw.get("playback") {
            let p = &mut settings.playback;
            if let Some(Value::String(name)) = playback.get("output_device") {
                p.output_device = Some(name.clone());
            }
            if let Some(frames) = playback.get("buffer_frames").and_then(Value::as_u64) {
                let clamped = frames.clamp(
                    u64::from(EngineConfig::MIN_BUFFER_FRAMES),
                    u64::from(EngineConfig::MAX_BUFFER_FRAMES),
                );
                // Within u32 after the clamp.
                p.buffer_frames = u32::try_from(clamped).unwrap_or(p.buffer_frames);
            }
        }
        if let Some(Value::Object(appearance)) = raw.get("appearance") {
            let a = &mut settings.appearance;
            if let Some(theme) = appearance
                .get("theme")
                .and_then(Value::as_str)
                .and_then(ThemeChoice::from_key)
            {
                a.theme = theme;
            }
            if let Some(density) = appearance
                .get("density")
                .and_then(Value::as_str)
                .and_then(DensityChoice::from_key)
            {
                a.density = density;
            }
            if let Some(reduce) = appearance.get("reduce_motion").and_then(Value::as_bool) {
                a.reduce_motion = reduce;
            }
        }
        if let Some(Value::Object(timeline)) = raw.get("timeline") {
            let mode = |key: &str| {
                timeline
                    .get(key)
                    .and_then(Value::as_str)
                    .and_then(SkimMode::from_key)
            };
            let default = Skim::default();
            settings.timeline.skim = Skim::new(
                mode("skim").unwrap_or(default.mode()),
                mode("skim_last_on").unwrap_or(default.last_on()),
            );
        }
        if let Some(Value::Object(v)) = raw.get("voiceover") {
            voiceover_from_raw(v, &mut settings.voiceover);
        }
        settings
    }

    /// `onto` with every setting in which `self` differs from `base` taken
    /// from `self`: the changes made to `base`, replayed on other settings.
    /// The app uses it for preferences changed before the file was read:
    /// they win over the file, and only they do.
    pub fn changes_onto(&self, base: &Settings, onto: &Settings) -> Settings {
        fn pick<T: Clone + PartialEq>(mine: &T, base: &T, onto: &T) -> T {
            if mine == base { onto } else { mine }.clone()
        }
        let (mine, base_p, onto_p) = (&self.playback, &base.playback, &onto.playback);
        let (mine_a, base_a, onto_a) = (&self.appearance, &base.appearance, &onto.appearance);
        let (mine_v, base_v, onto_v) = (&self.voiceover, &base.voiceover, &onto.voiceover);
        Settings {
            timeline: TimelineSettings {
                skim: pick(
                    &self.timeline.skim,
                    &base.timeline.skim,
                    &onto.timeline.skim,
                ),
            },
            voiceover: VoiceoverSettings {
                input_device: pick(
                    &mine_v.input_device,
                    &base_v.input_device,
                    &onto_v.input_device,
                ),
                countdown_secs: pick(
                    &mine_v.countdown_secs,
                    &base_v.countdown_secs,
                    &onto_v.countdown_secs,
                ),
                preroll_secs: pick(
                    &mine_v.preroll_secs,
                    &base_v.preroll_secs,
                    &onto_v.preroll_secs,
                ),
                stereo: pick(&mine_v.stereo, &base_v.stereo, &onto_v.stereo),
                new_track: pick(&mine_v.new_track, &base_v.new_track, &onto_v.new_track),
                mute_timeline: pick(
                    &mine_v.mute_timeline,
                    &base_v.mute_timeline,
                    &onto_v.mute_timeline,
                ),
                monitor: pick(&mine_v.monitor, &base_v.monitor, &onto_v.monitor),
                input_level: pick(
                    &mine_v.input_level,
                    &base_v.input_level,
                    &onto_v.input_level,
                ),
                latency_offset_ms: pick(
                    &mine_v.latency_offset_ms,
                    &base_v.latency_offset_ms,
                    &onto_v.latency_offset_ms,
                ),
            },
            playback: PlaybackSettings {
                output_device: pick(
                    &mine.output_device,
                    &base_p.output_device,
                    &onto_p.output_device,
                ),
                buffer_frames: pick(
                    &mine.buffer_frames,
                    &base_p.buffer_frames,
                    &onto_p.buffer_frames,
                ),
            },
            appearance: AppearanceSettings {
                theme: pick(&mine_a.theme, &base_a.theme, &onto_a.theme),
                density: pick(&mine_a.density, &base_a.density, &onto_a.density),
                reduce_motion: pick(
                    &mine_a.reduce_motion,
                    &base_a.reduce_motion,
                    &onto_a.reduce_motion,
                ),
            },
        }
    }

    /// `raw` with the known settings written into it: every other key is
    /// kept as it was, and a higher version number stays.
    pub fn to_raw(&self, raw: &Map<String, Value>) -> Map<String, Value> {
        let mut out = raw.clone();
        let version = raw
            .get("version")
            .and_then(Value::as_u64)
            .map_or(SETTINGS_VERSION, |v| v.max(SETTINGS_VERSION));
        out.insert("version".to_owned(), Value::from(version));
        let playback = out
            .entry("playback")
            .or_insert_with(|| Value::Object(Map::new()));
        if !playback.is_object() {
            *playback = Value::Object(Map::new());
        }
        if let Value::Object(p) = playback {
            p.insert(
                "output_device".to_owned(),
                self.playback
                    .output_device
                    .clone()
                    .map_or(Value::Null, Value::String),
            );
            p.insert(
                "buffer_frames".to_owned(),
                Value::from(self.playback.buffer_frames),
            );
        }
        let appearance = out
            .entry("appearance")
            .or_insert_with(|| Value::Object(Map::new()));
        if !appearance.is_object() {
            *appearance = Value::Object(Map::new());
        }
        if let Value::Object(a) = appearance {
            a.insert("theme".to_owned(), Value::from(self.appearance.theme.key()));
            a.insert(
                "density".to_owned(),
                Value::from(self.appearance.density.key()),
            );
            a.insert(
                "reduce_motion".to_owned(),
                Value::from(self.appearance.reduce_motion),
            );
        }
        if let Value::Object(t) = section(&mut out, "timeline") {
            let skim = self.timeline.skim;
            t.insert("skim".to_owned(), Value::from(skim.mode().key()));
            t.insert("skim_last_on".to_owned(), Value::from(skim.last_on().key()));
        }
        if let Value::Object(v) = section(&mut out, "voiceover") {
            let s = &self.voiceover;
            v.insert(
                "input_device".to_owned(),
                s.input_device.clone().map_or(Value::Null, Value::String),
            );
            v.insert("countdown_secs".to_owned(), Value::from(s.countdown_secs));
            v.insert("preroll_secs".to_owned(), Value::from(s.preroll_secs));
            let channels = if s.stereo { "stereo" } else { "mono" };
            v.insert("channels".to_owned(), Value::from(channels));
            let place = if s.new_track {
                "new_track"
            } else {
                "first_free"
            };
            v.insert("place".to_owned(), Value::from(place));
            v.insert("mute_timeline".to_owned(), Value::from(s.mute_timeline));
            v.insert("monitor".to_owned(), Value::from(s.monitor));
            v.insert(
                "input_level_db".to_owned(),
                Value::from(f64::from(s.input_level) / 100.0),
            );
            v.insert(
                "latency_offset_ms".to_owned(),
                Value::from(s.latency_offset_ms),
            );
        }
        out
    }
}

/// `out[name]`, made an object when it is missing or isn't one.
fn section<'a>(out: &'a mut Map<String, Value>, name: &str) -> &'a mut Value {
    let value = out.entry(name).or_insert_with(|| Value::Object(Map::new()));
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
}

/// The `voiceover` section's known keys; anything mistyped or out of range
/// keeps its default.
fn voiceover_from_raw(raw: &Map<String, Value>, v: &mut VoiceoverSettings) {
    if let Some(Value::String(id)) = raw.get("input_device") {
        v.input_device = Some(id.clone());
    }
    if let Some(n) = raw.get("countdown_secs").and_then(Value::as_u64)
        && let Some(secs) = COUNTDOWNS.into_iter().find(|c| u64::from(*c) == n)
    {
        v.countdown_secs = secs;
    }
    if let Some(n) = raw.get("preroll_secs").and_then(Value::as_u64)
        && n <= u64::from(MAX_PREROLL_SECS)
    {
        v.preroll_secs = u8::try_from(n).unwrap_or(0);
    }
    match raw.get("channels").and_then(Value::as_str) {
        Some("mono") => v.stereo = false,
        Some("stereo") => v.stereo = true,
        _ => {}
    }
    match raw.get("place").and_then(Value::as_str) {
        Some("first_free") => v.new_track = false,
        Some("new_track") => v.new_track = true,
        _ => {}
    }
    if let Some(b) = raw.get("mute_timeline").and_then(Value::as_bool) {
        v.mute_timeline = b;
    }
    if let Some(b) = raw.get("monitor").and_then(Value::as_bool) {
        v.monitor = b;
    }
    if let Some(db) = raw.get("input_level_db").and_then(Value::as_f64)
        && (-20.0..=20.0).contains(&db)
    {
        // Within ±2000 hundredths by the check.
        v.input_level = (db * 100.0).round() as i32;
    }
    if let Some(ms) = raw.get("latency_offset_ms").and_then(Value::as_i64)
        && (-500..=500).contains(&ms)
    {
        v.latency_offset_ms = i32::try_from(ms).unwrap_or(0);
    }
}

/// The settings folder of `os` (UI-018), from the environment values given:
/// `~/Library/Application Support/FreeCut` on macOS, `%APPDATA%\FreeCut`
/// on Windows, and `$XDG_CONFIG_HOME/freecut` (else `~/.config/freecut`)
/// elsewhere. `None` when the environment doesn't say.
pub fn config_dir(
    os: Os,
    home: Option<PathBuf>,
    appdata: Option<PathBuf>,
    xdg_config: Option<PathBuf>,
) -> Option<PathBuf> {
    match os {
        Os::Mac => Some(home?.join("Library/Application Support/FreeCut")),
        Os::Windows => appdata
            .or_else(|| home.map(|h| h.join("AppData/Roaming")))
            .map(|d| d.join("FreeCut")),
        Os::Other => xdg_config
            .filter(|d| d.is_absolute())
            .or_else(|| home.map(|h| h.join(".config")))
            .map(|d| d.join("freecut")),
    }
}

/// The settings file of this machine's user, from the environment (no file
/// I/O).
pub fn settings_path() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    config_dir(
        Os::current(),
        home_dir(),
        var("APPDATA"),
        var("XDG_CONFIG_HOME"),
    )
    .map(|d| d.join(FILE_NAME))
}

/// What reading the settings file gave.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub settings: Settings,
    /// The whole file as read, unknown keys included.
    pub raw: Map<String, Value>,
    /// Something to tell the user once (a damaged file was set aside).
    pub notice: Option<String>,
}

/// Blocking: reads the settings at `path`. A missing file gives the
/// defaults. A file that can't be read or isn't a JSON object is renamed
/// to `settings.json.corrupt-<date>` and the defaults are used, with
/// [`RESET_NOTICE`] (UI-018).
pub fn load(path: &Path) -> Loaded {
    let defaults = || Loaded {
        settings: Settings::default(),
        raw: Map::new(),
        notice: None,
    };
    let parsed = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return defaults(),
        Err(_) => None,
    };
    match parsed {
        Some(Value::Object(raw)) => Loaded {
            settings: Settings::from_raw(&raw),
            raw,
            notice: None,
        },
        _ => {
            set_aside(path, now_unix_secs());
            Loaded {
                notice: Some(RESET_NOTICE.to_owned()),
                ..defaults()
            }
        }
    }
}

/// Renames a damaged settings file out of the way (best effort: when that
/// fails too, the next save simply replaces it).
fn set_aside(path: &Path, unix_secs: u64) {
    let base = corrupt_name(unix_secs);
    for attempt in 0..100 {
        let name = if attempt == 0 {
            base.clone()
        } else {
            format!("{base}-{attempt}")
        };
        let target = path.with_file_name(name);
        if target.exists() {
            continue;
        }
        let _ = std::fs::rename(path, target);
        return;
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `settings.json.corrupt-<date>` for a time given in seconds since the
/// Unix epoch, as UTC `YYYY-MM-DD-HHMMSS` (no characters any OS refuses in
/// file names).
pub fn corrupt_name(unix_secs: u64) -> String {
    let days = i64::try_from(unix_secs / 86_400).unwrap_or(0);
    let rest = unix_secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{FILE_NAME}.corrupt-{y:04}-{m:02}-{d:02}-{:02}{:02}{:02}",
        rest / 3_600,
        rest / 60 % 60,
        rest % 60
    )
}

/// The proleptic Gregorian date of `days` after 1970-01-01 (Howard
/// Hinnant's `civil_from_days`).
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    // d is 1..=31 and m 1..=12 by construction.
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

/// Blocking: writes `bytes` to `path` atomically: a temporary file in the
/// same folder, flushed, then renamed over the old file. The folder is
/// created when missing.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    // Named after the file, so two files of one folder never share it.
    let name = path
        .file_name()
        .map_or_else(|| FILE_NAME.into(), |n| n.to_string_lossy());
    let tmp = path.with_file_name(format!("{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// A write for the writer thread.
pub(crate) struct WriteJob {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

/// The app's settings, and when to write them.
#[derive(Debug)]
pub struct SettingsStore {
    settings: Settings,
    raw: Map<String, Value>,
    /// Where they are saved; `None` keeps them in memory.
    path: Option<PathBuf>,
    /// The time of the first change not yet written.
    changed_at: Option<Instant>,
    writer: Option<Writer>,
    /// Messages for the user, shown once each.
    notices: Vec<String>,
    /// Writes queued but not answered yet.
    in_flight: usize,
}

/// The writer thread: writes run one after another, in order.
pub(crate) struct Writer {
    pub jobs: Sender<WriteJob>,
    pub done: Receiver<std::io::Result<()>>,
}

impl std::fmt::Debug for Writer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Writer").finish_non_exhaustive()
    }
}

impl Writer {
    /// Starts a writer thread called `name`; `wake` runs after each write.
    pub fn spawn(name: &str, wake: impl Fn() + Send + 'static) -> std::io::Result<Writer> {
        let (jobs, rx) = unbounded::<WriteJob>();
        let (tx, done) = unbounded();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                for job in rx {
                    let result = write_atomic(&job.path, &job.bytes);
                    if tx.send(result).is_err() {
                        return;
                    }
                    wake();
                }
            })?;
        Ok(Writer { jobs, done })
    }
}

impl SettingsStore {
    /// A store over what [`load`] read from `path` (`None`: memory only).
    pub fn new(loaded: Loaded, path: Option<PathBuf>) -> SettingsStore {
        SettingsStore {
            settings: loaded.settings,
            raw: loaded.raw,
            path,
            changed_at: None,
            writer: None,
            notices: loaded.notice.into_iter().collect(),
            in_flight: 0,
        }
    }

    /// Default settings that are never written (tests, and before the file
    /// was read).
    pub fn in_memory() -> SettingsStore {
        SettingsStore::new(
            Loaded {
                settings: Settings::default(),
                raw: Map::new(),
                notice: None,
            },
            None,
        )
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Whether changes are only kept in memory.
    pub fn memory_only(&self) -> bool {
        self.path.is_none()
    }

    /// Changes the settings: they apply now and are written
    /// [`SAVE_DELAY`] after the last change ([`SettingsStore::tick`]).
    pub fn set(&mut self, settings: Settings, now: Instant) {
        if settings != self.settings {
            self.settings = settings;
            self.changed_at = Some(now);
        }
    }

    /// Starts the write once [`SAVE_DELAY`] has passed since the last
    /// change, and collects finished writes. Returns when to call again,
    /// if a write is still waiting. `wake` repaints the window when the
    /// writer finishes.
    pub fn tick(&mut self, now: Instant, wake: impl Fn() + Send + 'static) -> Option<Instant> {
        self.collect();
        let changed = self.changed_at?;
        let due = changed + SAVE_DELAY;
        if now < due {
            return Some(due);
        }
        self.changed_at = None;
        self.write(wake);
        None
    }

    /// Writes now whatever changed (quitting).
    pub fn flush(&mut self, wake: impl Fn() + Send + 'static) {
        if self.changed_at.take().is_some() {
            self.write(wake);
        }
    }

    /// Writes whatever changed and waits up to `timeout` for the writer
    /// (the window is closing: a change made just before must not be lost).
    pub fn flush_and_wait(&mut self, timeout: Duration) {
        self.flush(|| {});
        let deadline = Instant::now() + timeout;
        while self.in_flight > 0 {
            let Some(w) = &self.writer else { return };
            let left = deadline.saturating_duration_since(Instant::now());
            match w.done.recv_timeout(left) {
                Ok(result) => {
                    self.in_flight -= 1;
                    if let Err(e) = result {
                        eprintln!("FreeCut: can't save preferences: {e}");
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
        self.raw = self.settings.to_raw(&self.raw);
        let bytes = match serde_json::to_vec_pretty(&Value::Object(self.raw.clone())) {
            Ok(b) => b,
            Err(e) => {
                self.notices
                    .push(format!("Preferences couldn't be saved: {e}"));
                return;
            }
        };
        if self.writer.is_none() {
            match Writer::spawn("freecut-settings", wake) {
                Ok(w) => self.writer = Some(w),
                Err(e) => {
                    self.notices
                        .push(format!("Preferences couldn't be saved: {e}"));
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

    /// Picks up finished writes. The first failure makes the store keep
    /// settings in memory from then on, with one warning (UI-018).
    fn collect(&mut self) {
        let Some(w) = &self.writer else { return };
        loop {
            match w.done.try_recv() {
                Ok(result) => {
                    self.in_flight = self.in_flight.saturating_sub(1);
                    if let Err(e) = result
                        && self.path.take().is_some()
                    {
                        eprintln!("FreeCut: can't save preferences: {e}");
                        self.notices.push(READ_ONLY_NOTICE.to_owned());
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
    pub fn busy(&self) -> bool {
        self.changed_at.is_some() || self.in_flight > 0
    }

    /// Messages for the user, each returned once.
    pub fn take_notices(&mut self) -> Vec<String> {
        self.collect();
        std::mem::take(&mut self.notices)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use super::*;

    fn wait_idle(store: &mut SettingsStore) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while store.busy() {
            assert!(Instant::now() < deadline, "the write never finished");
            store.tick(Instant::now() + SAVE_DELAY, || {});
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn raw_file(path: &Path) -> Map<String, Value> {
        match serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap() {
            Value::Object(m) => m,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_file_gives_the_defaults_without_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(&dir.path().join(FILE_NAME));
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(loaded.settings.playback.buffer_frames, 1024);
        assert_eq!(loaded.settings.playback.output_device, None);
        assert!(loaded.raw.is_empty());
        assert_eq!(loaded.notice, None);
    }

    #[test]
    fn settings_save_after_the_delay_and_load_back() {
        let dir = tempfile::tempdir().unwrap();
        // The folder doesn't exist yet: the first save creates it.
        let path = dir.path().join("FreeCut").join(FILE_NAME);
        let mut store = SettingsStore::new(load(&path), Some(path.clone()));
        let start = Instant::now();
        let mut s = store.settings().clone();
        s.playback.output_device = Some("USB Headphones".to_owned());
        s.playback.buffer_frames = 512;
        store.set(s.clone(), start);
        // Nothing is written before the delay; a later change restarts it.
        assert_eq!(store.tick(start, || {}), Some(start + SAVE_DELAY));
        assert!(!path.exists());
        let later = start + Duration::from_millis(300);
        s.playback.buffer_frames = 2048;
        store.set(s.clone(), later);
        assert_eq!(
            store.tick(start + SAVE_DELAY, || {}),
            Some(later + SAVE_DELAY)
        );
        assert_eq!(store.tick(later + SAVE_DELAY, || {}), None);
        wait_idle(&mut store);
        assert!(store.take_notices().is_empty());

        let loaded = load(&path);
        assert_eq!(loaded.settings, s);
        assert_eq!(loaded.notice, None);
        assert_eq!(loaded.raw["version"], Value::from(SETTINGS_VERSION));
        // No temporary file is left behind.
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [FILE_NAME]);

        // Back to the system default: stored as null. Quitting writes a
        // change that is still waiting for its delay.
        s.playback.output_device = None;
        store.set(s.clone(), later);
        store.flush_and_wait(Duration::from_secs(10));
        assert!(!store.busy());
        assert_eq!(raw_file(&path)["playback"]["output_device"], Value::Null);
        assert_eq!(load(&path).settings, s);
    }

    #[test]
    fn unknown_keys_and_a_newer_version_survive_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let newer = serde_json::json!({
            "version": 7,
            "appearance": { "theme": "light", "ui_scale": 125 },
            "playback": {
                "output_device": "Speakers",
                "buffer_frames": 256,
                "scrub_audio": false
            },
            "future_list": [1, 2, 3]
        });
        std::fs::write(&path, serde_json::to_vec(&newer).unwrap()).unwrap();
        let loaded = load(&path);
        assert_eq!(
            loaded.settings.playback.output_device.as_deref(),
            Some("Speakers")
        );
        assert_eq!(loaded.settings.playback.buffer_frames, 256);
        assert_eq!(loaded.settings.appearance.theme, ThemeChoice::Light);
        let mut store = SettingsStore::new(loaded, Some(path.clone()));
        let mut s = store.settings().clone();
        s.playback.buffer_frames = 4096;
        store.set(s, Instant::now());
        store.flush(|| {});
        wait_idle(&mut store);

        let mut expected = newer;
        expected["playback"]["buffer_frames"] = Value::from(4096);
        // The known appearance keys are written; `ui_scale` stays.
        expected["appearance"]["density"] = Value::from("automatic");
        expected["appearance"]["reduce_motion"] = Value::from(false);
        // So are the timeline's and the recorder's, at their defaults.
        expected["timeline"] = serde_json::json!({"skim": "off", "skim_last_on": "picture"});
        expected["voiceover"] = serde_json::json!({
            "input_device": null,
            "countdown_secs": 3,
            "preroll_secs": 0,
            "channels": "mono",
            "place": "first_free",
            "mute_timeline": false,
            "monitor": false,
            "input_level_db": 0.0,
            "latency_offset_ms": 0
        });
        assert_eq!(Value::Object(raw_file(&path)), expected);
    }

    #[test]
    fn mistyped_or_out_of_range_values_fall_back_without_losing_the_file() {
        let raw = |v: Value| match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        };
        let s = Settings::from_raw(&raw(serde_json::json!({
            "playback": { "output_device": 3, "buffer_frames": "big" }
        })));
        assert_eq!(s, Settings::default());
        let s = Settings::from_raw(&raw(serde_json::json!({
            "playback": { "buffer_frames": 100_000 }
        })));
        assert_eq!(s.playback.buffer_frames, 4096);
        let s = Settings::from_raw(&raw(serde_json::json!({
            "playback": { "buffer_frames": 16 }
        })));
        assert_eq!(s.playback.buffer_frames, 256);
        // A section of the wrong type is replaced on write.
        let out = Settings::default().to_raw(&raw(serde_json::json!({ "playback": 5 })));
        assert_eq!(out["playback"]["buffer_frames"], Value::from(1024));
    }

    #[test]
    fn a_damaged_file_is_set_aside_and_the_defaults_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let damaged: [&[u8]; 3] = [b"{ \"playback\": ", b"[1, 2]", b"\xff\xfe"];
        for damaged in damaged {
            std::fs::write(&path, damaged).unwrap();
            let loaded = load(&path);
            assert_eq!(loaded.settings, Settings::default());
            assert_eq!(loaded.notice.as_deref(), Some(RESET_NOTICE));
            assert!(!path.exists(), "renamed away");
        }
        let mut aside: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        aside.sort();
        assert_eq!(aside.len(), 3, "{aside:?}");
        for name in &aside {
            assert!(
                name.starts_with("settings.json.corrupt-"),
                "{name} in {aside:?}"
            );
        }
        // The damaged content is kept for the user to look at.
        let first = dir.path().join(&aside[0]);
        assert!(std::fs::metadata(first).unwrap().len() > 0);

        // The store shows the notice once, and saves over the old name.
        let mut store = SettingsStore::new(load(&path), Some(path.clone()));
        assert!(store.take_notices().is_empty(), "missing file: no notice");
        std::fs::write(&path, b"nonsense").unwrap();
        let mut store2 = SettingsStore::new(load(&path), Some(path.clone()));
        assert_eq!(store2.take_notices(), [RESET_NOTICE]);
        assert!(store2.take_notices().is_empty());
        store.set(
            Settings {
                playback: PlaybackSettings {
                    output_device: None,
                    buffer_frames: 512,
                },
                ..Settings::default()
            },
            Instant::now(),
        );
        store.flush(|| {});
        wait_idle(&mut store);
        assert_eq!(load(&path).settings.playback.buffer_frames, 512);
    }

    #[test]
    fn the_theme_and_density_are_saved_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut store = SettingsStore::new(load(&path), Some(path.clone()));
        assert_eq!(store.settings().appearance, AppearanceSettings::default());
        assert_eq!(store.settings().appearance.theme, ThemeChoice::Dark);
        let mut s = store.settings().clone();
        s.appearance.theme = ThemeChoice::HighContrast;
        s.appearance.density = DensityChoice::Comfortable;
        s.appearance.reduce_motion = true;
        store.set(s.clone(), Instant::now());
        store.flush_and_wait(Duration::from_secs(10));
        let raw = raw_file(&path);
        assert_eq!(raw["appearance"]["theme"], Value::from("high_contrast"));
        assert_eq!(raw["appearance"]["density"], Value::from("comfortable"));
        assert_eq!(raw["appearance"]["reduce_motion"], Value::from(true));
        assert_eq!(load(&path).settings, s);
        // Unknown or mistyped values fall back to the defaults.
        let raw = |v: Value| match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        };
        let s = Settings::from_raw(&raw(serde_json::json!({
            "appearance": { "theme": "sepia", "density": 3, "reduce_motion": "yes" }
        })));
        assert_eq!(s.appearance, AppearanceSettings::default());
    }

    #[test]
    fn corrupt_names_carry_the_utc_date() {
        assert_eq!(corrupt_name(0), "settings.json.corrupt-1970-01-01-000000");
        // 2026-10-12 15:30:05 UTC.
        assert_eq!(
            corrupt_name(1_791_819_005),
            "settings.json.corrupt-2026-10-12-153005"
        );
        // A leap day.
        assert_eq!(
            corrupt_name(951_782_400),
            "settings.json.corrupt-2000-02-29-000000"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_keeps_settings_in_memory_with_one_warning() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("locked");
        std::fs::create_dir(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let path = folder.join(FILE_NAME);
        if std::fs::write(folder.join("probe"), b"x").is_ok() {
            eprintln!("skipped: permissions aren't enforced here (running as root?)");
            return;
        }
        let mut store = SettingsStore::new(load(&path), Some(path.clone()));
        let mut s = store.settings().clone();
        for frames in [512, 2048] {
            s.playback.buffer_frames = frames;
            store.set(s.clone(), Instant::now());
            store.flush(|| {});
            wait_idle(&mut store);
        }
        assert_eq!(store.take_notices(), [READ_ONLY_NOTICE]);
        assert!(store.memory_only());
        // The change still applies.
        assert_eq!(store.settings().playback.buffer_frames, 2048);
        assert!(!path.exists());
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_settings_folder_follows_each_platform() {
        let home = Some(PathBuf::from("/home/ana"));
        assert_eq!(
            config_dir(Os::Mac, Some(PathBuf::from("/Users/ana")), None, None),
            Some(PathBuf::from(
                "/Users/ana/Library/Application Support/FreeCut"
            ))
        );
        assert_eq!(
            config_dir(
                Os::Windows,
                home.clone(),
                Some(PathBuf::from("C:/Users/ana/AppData/Roaming")),
                None
            ),
            Some(PathBuf::from("C:/Users/ana/AppData/Roaming/FreeCut"))
        );
        // PathBuf uses the test host's path rules even when we exercise
        // another Os branch: a rooted /cfg has no drive on Windows.
        let xdg = std::env::temp_dir().join("freecut-config-test");
        assert_eq!(
            config_dir(Os::Other, home.clone(), None, Some(xdg.clone())),
            Some(xdg.join("freecut"))
        );
        // A relative XDG folder is ignored, as the spec says.
        assert_eq!(
            config_dir(Os::Other, home, None, Some(PathBuf::from("cfg"))),
            Some(PathBuf::from("/home/ana/.config/freecut"))
        );
        assert_eq!(config_dir(Os::Mac, None, None, None), None);
    }

    #[test]
    fn changes_replay_only_the_settings_that_changed() {
        let base = Settings::default();
        let mut early = base.clone();
        early.playback.buffer_frames = 2048;
        let file = Settings {
            playback: PlaybackSettings {
                output_device: Some("USB Headphones".into()),
                buffer_frames: 512,
            },
            appearance: AppearanceSettings {
                theme: ThemeChoice::Light,
                density: DensityChoice::Compact,
                reduce_motion: true,
            },
            ..Settings::default()
        };
        let merged = early.changes_onto(&base, &file);
        assert_eq!(
            merged.playback.output_device.as_deref(),
            Some("USB Headphones")
        );
        assert_eq!(merged.playback.buffer_frames, 2048);
        assert_eq!(merged.appearance.theme, ThemeChoice::Light, "from the file");
        assert_eq!(base.changes_onto(&base, &file), file, "nothing changed");
        let mut dark = base.clone();
        dark.appearance.theme = ThemeChoice::HighContrast;
        let merged = dark.changes_onto(&base, &file);
        assert_eq!(merged.appearance.theme, ThemeChoice::HighContrast);
        assert_eq!(merged.appearance.density, DensityChoice::Compact);
    }

    #[test]
    fn playback_settings_become_the_engine_config() {
        let p = PlaybackSettings {
            output_device: Some("Desk".to_owned()),
            buffer_frames: 512,
        };
        let c = p.engine_config();
        assert_eq!(c.device.as_deref(), Some("Desk"));
        assert_eq!(c.buffer(), 512);
        assert_eq!(
            PlaybackSettings::default().engine_config(),
            EngineConfig::default()
        );
    }

    /// TL-014 and AU-027: the skim mode and the recorder's choices are
    /// written and read back; bad values keep their defaults.
    #[test]
    fn skim_and_voiceover_choices_round_trip() {
        let mut s = Settings::default();
        assert_eq!(s.voiceover.countdown_secs, 3, "3 s countdown by default");
        assert!(!s.timeline.skim.is_on(), "skim is off by default");
        s.timeline.skim.toggle();
        s.voiceover = VoiceoverSettings {
            input_device: Some("usb-mic".to_owned()),
            countdown_secs: 5,
            preroll_secs: 2,
            stereo: true,
            new_track: true,
            mute_timeline: true,
            monitor: true,
            input_level: -650,
            latency_offset_ms: 40,
        };
        let raw = s.to_raw(&Map::new());
        assert_eq!(raw["timeline"]["skim"], "picture");
        assert_eq!(raw["voiceover"]["channels"], "stereo");
        assert_eq!(Settings::from_raw(&raw), s);

        let bad: Map<String, Value> = serde_json::from_str(
            r#"{"timeline": {"skim": "loud"},
                "voiceover": {"countdown_secs": 4, "preroll_secs": 9,
                              "channels": 2, "input_level_db": 99,
                              "latency_offset_ms": -9000}}"#,
        )
        .unwrap();
        assert_eq!(Settings::from_raw(&bad), Settings::default());
    }
}
