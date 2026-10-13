//! Hover skim (TL-014): the frame under the pointer in the player, without
//! moving the playhead.
//!
//! Three pure parts, so the rules are tested without a window:
//! - [`Skim`]: the mode and the last mode that was on (a per-user
//!   preference, UI-020). `S` and the toolbar toggle switch between Off and
//!   the last on-mode; the menu picks a mode.
//! - [`skim_time`]: the time the player should show instead of the
//!   playhead, if any: never while playing or dragging, nothing with the
//!   pointer outside the ruler and lanes, and always the blade's cut line
//!   (TL-024), whatever the mode.
//! - [`Throttle`]: frame requests at most once per display refresh; a
//!   request that a newer one overtakes before it is due is dropped.
//!
//! The player reads `preview_time = skim.unwrap_or(playhead)`.

use std::time::{Duration, Instant};

use freecut_timeline::time::Tick;

/// AU-016's scrub-sound grains aren't built yet: until they are, *Picture
/// and sound* is not offered and `S` toggles Off ↔ *Picture only*.
pub const SCRUB_SOUND_BUILT: bool = false;

/// What skimming shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkimMode {
    Off,
    /// *Picture only*.
    Picture,
    /// *Picture and sound* (needs AU-016).
    PictureAndSound,
}

impl SkimMode {
    /// The settings-file key.
    pub fn key(self) -> &'static str {
        match self {
            SkimMode::Off => "off",
            SkimMode::Picture => "picture",
            SkimMode::PictureAndSound => "picture_and_sound",
        }
    }

    pub fn from_key(key: &str) -> Option<SkimMode> {
        Some(match key {
            "off" => SkimMode::Off,
            "picture" => SkimMode::Picture,
            "picture_and_sound" => SkimMode::PictureAndSound,
            _ => return None,
        })
    }

    /// The menu's label (FreeCut wording).
    pub fn label(self) -> &'static str {
        match self {
            SkimMode::Off => "Off",
            SkimMode::Picture => "Picture only",
            SkimMode::PictureAndSound => "Picture and sound",
        }
    }

    /// Whether this build can show the mode.
    pub fn available(self) -> bool {
        self != SkimMode::PictureAndSound || SCRUB_SOUND_BUILT
    }

    /// The modes the menu offers, in order.
    pub fn offered() -> &'static [SkimMode] {
        if SCRUB_SOUND_BUILT {
            &[SkimMode::Off, SkimMode::Picture, SkimMode::PictureAndSound]
        } else {
            &[SkimMode::Off, SkimMode::Picture]
        }
    }
}

/// The hover-skim preference: the mode, and the on-mode `S` comes back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Skim {
    mode: SkimMode,
    last_on: SkimMode,
}

impl Default for Skim {
    /// Off; the first `S` turns on *Picture only*.
    fn default() -> Skim {
        Skim {
            mode: SkimMode::Off,
            last_on: SkimMode::Picture,
        }
    }
}

impl Skim {
    /// A preference as read from the settings file. Modes this build can't
    /// show become *Picture only*; the last on-mode is never Off.
    pub fn new(mode: SkimMode, last_on: SkimMode) -> Skim {
        let usable = |m: SkimMode| if m.available() { m } else { SkimMode::Picture };
        let last_on = match usable(last_on) {
            SkimMode::Off => SkimMode::Picture,
            m => m,
        };
        Skim {
            mode: usable(mode),
            last_on,
        }
    }

    pub fn mode(self) -> SkimMode {
        self.mode
    }

    pub fn last_on(self) -> SkimMode {
        self.last_on
    }

    pub fn is_on(self) -> bool {
        self.mode != SkimMode::Off
    }

    /// `S`, and a click on the toolbar toggle: Off ↔ the last on-mode.
    pub fn toggle(&mut self) {
        self.mode = if self.is_on() {
            SkimMode::Off
        } else {
            self.last_on
        };
    }

    /// The menu's radio items. Picking an on-mode makes it the one `S`
    /// comes back to; a mode this build can't show is ignored.
    pub fn set_mode(&mut self, mode: SkimMode) {
        if !mode.available() {
            return;
        }
        self.mode = mode;
        if mode != SkimMode::Off {
            self.last_on = mode;
        }
    }

    /// The toggle's tooltip: the mode and the key ("Skim: picture only (S)").
    pub fn tooltip(self) -> String {
        match self.mode {
            SkimMode::Off => "Skim: off. Preview the frame under the pointer (S)".to_owned(),
            SkimMode::Picture => "Skim: picture only (S)".to_owned(),
            SkimMode::PictureAndSound => "Skim: picture and sound (S)".to_owned(),
        }
    }
}

/// What the timeline knows this frame, for [`skim_time`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SkimInput {
    /// The sequence is playing.
    pub playing: bool,
    /// A clip, edge, marker or the playhead is being dragged.
    pub dragging: bool,
    /// The frame under the pointer over the ruler or the lanes; `None`
    /// outside them.
    pub pointer: Option<Tick>,
    /// The blade's cut line, while the blade hovers a clip (TL-024).
    pub blade: Option<Tick>,
}

/// The time the player shows instead of the playhead, if any.
pub fn skim_time(skim: Skim, input: SkimInput) -> Option<Tick> {
    if input.playing || input.dragging {
        return None;
    }
    // TL-014: the blade shows the frame under its cut line on its own, so
    // it works the same with skim Off.
    if input.blade.is_some() {
        return input.blade;
    }
    if !skim.is_on() {
        return None;
    }
    input.pointer
}

/// Paces skim frame requests to the display refresh (TL-014). Each frame
/// the app passes the time it wants shown; [`Throttle::update`] says which
/// time to request now. A wanted time that arrives too soon waits; a newer
/// one replaces it, so stale requests are dropped rather than queued.
#[derive(Debug, Clone)]
pub struct Throttle {
    interval: Duration,
    sent_at: Option<Instant>,
    shown: Option<Tick>,
    pending: Option<Tick>,
    dropped: u64,
}

impl Throttle {
    pub fn new(interval: Duration) -> Throttle {
        Throttle {
            interval,
            sent_at: None,
            shown: None,
            pending: None,
            dropped: 0,
        }
    }

    /// The display's refresh interval (it can change with the monitor).
    pub fn set_interval(&mut self, interval: Duration) {
        self.interval = interval;
    }

    /// The skim time to show at `now` when `want` is wanted. `None` (the
    /// pointer left, playback started) goes back to the playhead at once.
    pub fn update(&mut self, now: Instant, want: Option<Tick>) -> Option<Tick> {
        let Some(t) = want else {
            self.pending = None;
            self.shown = None;
            return None;
        };
        if self.shown == Some(t) {
            self.pending = None;
            return self.shown;
        }
        let due = self
            .sent_at
            .is_none_or(|at| now.saturating_duration_since(at) >= self.interval);
        if due {
            self.shown = Some(t);
            self.sent_at = Some(now);
            self.pending = None;
        } else if self.pending.replace(t).is_some_and(|old| old != t) {
            self.dropped += 1;
        }
        self.shown
    }

    /// When a waiting time becomes due: the app repaints then.
    pub fn due(&self) -> Option<Instant> {
        self.pending?;
        self.sent_at.map(|at| at + self.interval)
    }

    /// Requests that were overtaken before they were sent.
    #[cfg(test)]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

impl Default for Throttle {
    /// 60 Hz until the display says otherwise.
    fn default() -> Throttle {
        Throttle::new(Duration::from_micros(16_667))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use super::*;

    fn s(n: i64) -> Tick {
        Tick::from_secs(n).unwrap()
    }

    #[test]
    fn s_and_the_toggle_switch_between_off_and_the_last_on_mode() {
        let mut skim = Skim::default();
        assert_eq!(skim.mode(), SkimMode::Off);
        skim.toggle();
        assert_eq!(skim.mode(), SkimMode::Picture, "starts as Picture only");
        skim.toggle();
        assert_eq!(skim.mode(), SkimMode::Off);
        assert_eq!(skim.last_on(), SkimMode::Picture);
    }

    #[test]
    fn the_menu_sets_the_mode_and_remembers_the_last_on_mode() {
        let mut skim = Skim::default();
        skim.set_mode(SkimMode::Picture);
        assert!(skim.is_on());
        skim.set_mode(SkimMode::Off);
        assert!(!skim.is_on());
        assert_eq!(
            skim.last_on(),
            SkimMode::Picture,
            "Off is never the last on-mode"
        );
        skim.toggle();
        assert_eq!(skim.mode(), SkimMode::Picture);
    }

    #[test]
    fn picture_and_sound_waits_for_scrub_sound() {
        assert!(!SkimMode::offered().contains(&SkimMode::PictureAndSound));
        let mut skim = Skim::default();
        skim.set_mode(SkimMode::PictureAndSound);
        assert_eq!(skim.mode(), SkimMode::Off, "not offered, so ignored");
        // A settings file written by a build that had it reads as Picture.
        let read = Skim::new(SkimMode::PictureAndSound, SkimMode::PictureAndSound);
        assert_eq!(read.mode(), SkimMode::Picture);
        assert_eq!(read.last_on(), SkimMode::Picture);
        let read = Skim::new(SkimMode::Off, SkimMode::Off);
        assert_eq!(read.last_on(), SkimMode::Picture);
        for m in [SkimMode::Off, SkimMode::Picture, SkimMode::PictureAndSound] {
            assert_eq!(SkimMode::from_key(m.key()), Some(m));
        }
        assert_eq!(SkimMode::from_key("loud"), None);
    }

    #[test]
    fn the_tooltip_names_the_mode_and_the_key() {
        let mut skim = Skim::default();
        assert!(skim.tooltip().ends_with("(S)"));
        skim.toggle();
        assert_eq!(skim.tooltip(), "Skim: picture only (S)");
    }

    #[test]
    fn no_skim_while_playing_dragging_or_outside_the_lanes() {
        let on = Skim::new(SkimMode::Picture, SkimMode::Picture);
        let over = SkimInput {
            pointer: Some(s(2)),
            ..SkimInput::default()
        };
        assert_eq!(skim_time(on, over), Some(s(2)));
        let playing = SkimInput {
            playing: true,
            ..over
        };
        assert_eq!(skim_time(on, playing), None);
        let dragging = SkimInput {
            dragging: true,
            ..over
        };
        assert_eq!(skim_time(on, dragging), None);
        assert_eq!(skim_time(on, SkimInput::default()), None, "outside");
        assert_eq!(skim_time(Skim::default(), over), None, "mode Off");
    }

    #[test]
    fn the_blade_shows_its_cut_line_whatever_the_mode() {
        let input = SkimInput {
            pointer: Some(s(2)),
            blade: Some(s(3)),
            ..SkimInput::default()
        };
        assert_eq!(skim_time(Skim::default(), input), Some(s(3)));
        let playing = SkimInput {
            playing: true,
            ..input
        };
        assert_eq!(skim_time(Skim::default(), playing), None);
    }

    #[test]
    fn the_throttle_sends_once_per_refresh_and_drops_stale_requests() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut th = Throttle::new(ms(16));
        assert_eq!(
            th.update(t0, Some(s(1))),
            Some(s(1)),
            "the first goes at once"
        );
        assert_eq!(th.due(), None);
        // Two more within the same refresh: the first is overtaken.
        assert_eq!(th.update(t0 + ms(4), Some(s(2))), Some(s(1)));
        assert_eq!(th.update(t0 + ms(8), Some(s(3))), Some(s(1)));
        assert_eq!(th.dropped(), 1, "2 s was never requested");
        assert_eq!(th.due(), Some(t0 + ms(16)));
        // Due: the newest goes.
        assert_eq!(th.update(t0 + ms(16), Some(s(3))), Some(s(3)));
        assert_eq!(th.due(), None);
        // The same time again costs nothing.
        assert_eq!(th.update(t0 + ms(17), Some(s(3))), Some(s(3)));
        // Leaving the lanes goes back to the playhead at once.
        assert_eq!(th.update(t0 + ms(18), Some(s(4))), Some(s(3)));
        assert_eq!(th.update(t0 + ms(19), None), None);
        assert_eq!(th.due(), None, "nothing waits after leaving");
        // Back in: still paced from the last request.
        assert_eq!(th.update(t0 + ms(20), Some(s(5))), None);
        assert_eq!(th.update(t0 + ms(32), Some(s(5))), Some(s(5)));
    }
}
