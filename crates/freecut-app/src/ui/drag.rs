//! Slider and field drags whose edits merge into one undo entry (TL-043).

use eframe::egui::Response;
use freecut_timeline::history::{Coalesce, GestureId};

use crate::session::Project;

/// Where the pointer is for [`DragGesture::coalesce_held`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Held {
    /// Not pressed.
    #[default]
    Up,
    Down,
    /// Released this frame.
    Released,
}

impl Held {
    pub fn of(ui: &eframe::egui::Ui) -> Held {
        ui.input(|i| {
            if i.pointer.any_down() {
                Held::Down
            } else if i.pointer.any_released() {
                Held::Released
            } else {
                Held::Up
            }
        })
    }
}

/// The drag in progress on one panel's sliders and fields.
#[derive(Debug, Default)]
pub struct DragGesture {
    gesture: Option<GestureId>,
}

impl DragGesture {
    /// How to commit an edit that `response` made this frame: as part of
    /// its drag, or as an entry of its own. The frame that releases a drag
    /// still belongs to it: egui sets the value from the release position,
    /// so a pointer that moved during that frame changes the value once more.
    pub fn coalesce(&mut self, project: &mut Project, response: &Response) -> Coalesce {
        if response.dragged() || response.drag_started() || response.drag_stopped() {
            let g = match self.gesture {
                Some(g) => g,
                None => *self.gesture.insert(project.begin_gesture()),
            };
            Coalesce::Gesture(g)
        } else {
            self.end(project);
            Coalesce::Never
        }
    }

    /// How to commit an edit made by a widget without a drag of its own
    /// (a colour picker's square): merged into one entry while the pointer
    /// is held, its own entry otherwise. The release frame still belongs to
    /// an open gesture: egui's colour sliders set the value from the release
    /// position, so a pointer that moved during that frame changes it once
    /// more. [`DragGesture::settle`] ends the gesture after the release.
    pub fn coalesce_held(&mut self, project: &mut Project, held: Held) -> Coalesce {
        let joins = match held {
            Held::Down => true,
            Held::Released => self.gesture.is_some(),
            Held::Up => false,
        };
        if joins {
            let g = match self.gesture {
                Some(g) => g,
                None => *self.gesture.insert(project.begin_gesture()),
            };
            Coalesce::Gesture(g)
        } else {
            self.end(project);
            Coalesce::Never
        }
    }

    /// Ends the drag once `response` reports its release. Call after the
    /// release frame's edit, if any, was committed.
    pub fn finish(&mut self, project: &mut Project, response: &Response) {
        if response.drag_stopped() {
            self.end(project);
        }
    }

    pub fn end(&mut self, project: &mut Project) {
        if let Some(g) = self.gesture.take() {
            project.end_gesture(g);
        }
    }

    /// `Esc` during a drag (UI-009): everything the drag committed is
    /// reverted and no undo entry remains. Nothing happens without a drag
    /// (one that changed nothing yet has nothing to revert).
    pub fn cancel(&mut self, project: &mut Project) -> Result<(), String> {
        match self.gesture.take() {
            Some(g) => project.cancel_gesture(g),
            None => Ok(()),
        }
    }

    /// Ends a drag whose widget went away without reporting its end (the
    /// selection changed mid-drag, the panel closed). Call once per frame.
    pub fn settle(&mut self, project: Option<&mut Project>, pointer_down: bool) {
        if pointer_down {
            return;
        }
        if let Some(p) = project {
            self.end(p);
        } else {
            self.gesture = None;
        }
    }

    /// Forgets a drag of a project that is no longer open.
    pub fn reset(&mut self) {
        self.gesture = None;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used)] // test code

    use eframe::egui::{
        self, Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Response,
    };
    use freecut_timeline::edit::{EditRequest, InsertTarget};
    use freecut_timeline::ids::ClipId;
    use freecut_timeline::model::{
        AudioStreamInfo, GainDb, MediaInfo, ProjectSettings, StoredPath,
    };
    use freecut_timeline::time::Tick;

    use super::*;
    use crate::clip_sound::gain_from_slider;
    use crate::session::ProbedFile;
    use freecut_timeline::edit::{AudioEdit, NewMedia};

    /// A project holding one 4 s audio clip, selected.
    pub(crate) fn project_with_a_sound() -> (Project, ClipId) {
        let mut project = Project::new(ProjectSettings::new_default().unwrap()).unwrap();
        let four = Tick::from_secs(4).unwrap();
        let info = MediaInfo {
            container: "wav".to_owned(),
            default_video: None,
            duration: four,
            video: Vec::new(),
            audio: vec![AudioStreamInfo {
                index: 0,
                codec: "pcm_s16le".to_owned(),
                sample_rate: 48_000,
                channels: 2,
                start: Tick::ZERO,
                duration: four,
                duration_exact: true,
                default: true,
                language: None,
                title: None,
                layout: None,
            }],
            still: None,
        };
        let file = ProbedFile {
            path: "tone.wav".into(),
            media: NewMedia {
                path: StoredPath {
                    absolute: "/tone.wav".to_owned(),
                    relative: None,
                },
                info,
                fingerprint: None,
                display_name: None,
            },
        };
        let ids = project.import(vec![file]).unwrap();
        crate::ui::timeline::insert_media(&mut project, ids[0], InsertTarget::NewTrack, Tick::ZERO)
            .unwrap();
        let clip = project
            .doc()
            .sequence(project.doc().root())
            .unwrap()
            .audio_tracks()[0]
            .clips()[0]
            .id;
        project.selection = vec![clip];
        (project, clip)
    }

    pub(crate) fn gain(project: &Project, clip: ClipId) -> GainDb {
        project
            .doc()
            .clip(clip)
            .unwrap()
            .audio
            .active()
            .unwrap()
            .gain
            .static_value()
            .to_owned()
    }

    /// Drives a headless egui context one frame at a time.
    pub(crate) struct Frames {
        pub ctx: Context,
        time: f64,
    }

    impl Frames {
        pub fn new() -> Frames {
            Frames {
                ctx: Context::default(),
                time: 0.0,
            }
        }

        /// Runs one frame; returns when the frame asked to be repainted.
        pub fn run(
            &mut self,
            events: Vec<Event>,
            draw: impl FnMut(&mut egui::Ui),
        ) -> std::time::Duration {
            self.run_full(events, draw)
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .map_or(std::time::Duration::MAX, |v| v.repaint_delay)
        }

        /// Runs one frame; returns all it put out (its shapes, …).
        pub fn run_full(
            &mut self,
            events: Vec<Event>,
            mut draw: impl FnMut(&mut egui::Ui),
        ) -> egui::FullOutput {
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                time: Some(self.time),
                events,
                focused: true,
                ..RawInput::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| draw(ui));
            // Nothing renders here: drop the font atlas uploads.
            output.textures_delta.clear();
            output
        }
    }

    pub(crate) fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    /// Holds `modifiers` from this frame on (Alt-click, Shift-click).
    pub(crate) fn modifiers(modifiers: Modifiers) -> Event {
        Event::ModifiersChanged(modifiers)
    }

    pub(crate) fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// One frame of a volume slider wired like the inspector's.
    fn slider_frame(
        frames: &mut Frames,
        project: &mut Project,
        drag: &mut DragGesture,
        clip: ClipId,
        events: Vec<Event>,
    ) -> Rect {
        let mut rect = Rect::NOTHING;
        frames.run(events, |ui| {
            let mut u = crate::clip_sound::slider_from_gain(gain(project, clip));
            let slider: Response = ui.add(egui::Slider::new(&mut u, 0.0..=1.0).show_value(false));
            rect = slider.rect;
            if slider.changed() {
                let coalesce = drag.coalesce(project, &slider);
                let req = EditRequest::SetClipAudio {
                    seq: project.doc().root(),
                    clips: vec![clip],
                    edit: AudioEdit::Gain(gain_from_slider(u)),
                };
                project.apply(&req, coalesce).unwrap();
            }
            drag.finish(project, &slider);
        });
        let down = frames.ctx.input(|i| i.pointer.any_down());
        drag.settle(Some(project), down);
        rect
    }

    /// S1: a colour picker drag whose release frame moves the colour once
    /// more is still one undo entry.
    #[test]
    fn a_held_drag_released_while_moving_is_one_undo_entry() {
        let (mut project, clip) = project_with_a_sound();
        let mut drag = DragGesture::default();
        let entries = project.editor.history().len();
        let set = |project: &mut Project, drag: &mut DragGesture, db: i32, held: Held| {
            let coalesce = drag.coalesce_held(project, held);
            let req = EditRequest::SetClipAudio {
                seq: project.doc().root(),
                clips: vec![clip],
                edit: AudioEdit::Gain(GainDb::from_hundredths(db)),
            };
            project.apply(&req, coalesce).unwrap();
        };
        set(&mut project, &mut drag, -100, Held::Down);
        set(&mut project, &mut drag, -200, Held::Down);
        // The release frame: the picker sets the value at the release point.
        set(&mut project, &mut drag, -300, Held::Released);
        drag.settle(Some(&mut project), false);
        assert_eq!(project.editor.history().len(), entries + 1, "one entry");
        project.undo().unwrap();
        assert_eq!(gain(&project, clip), GainDb::ZERO, "one undo: back to 0 dB");

        // A click without a gesture (a released pointer) is its own entry.
        set(&mut project, &mut drag, -400, Held::Released);
        assert!(!project.editor.gesture_open());
        set(&mut project, &mut drag, -500, Held::Up);
        assert_eq!(project.editor.history().len(), entries + 2);
    }

    #[test]
    fn a_slider_drag_released_while_moving_is_one_undo_entry() {
        let (mut project, clip) = project_with_a_sound();
        let mut drag = DragGesture::default();
        let mut frames = Frames::new();
        let rect = slider_frame(&mut frames, &mut project, &mut drag, clip, Vec::new());
        let entries = project.editor.history().len();
        let y = rect.center().y;
        let at = |u: f32| Pos2::new(rect.left() + rect.width() * u, y);
        let start = crate::clip_sound::slider_from_gain(GainDb::ZERO) as f32;
        let mut events = vec![Event::PointerMoved(at(start)), button(at(start), true)];
        for step in 1..=5 {
            events.push(Event::PointerMoved(at(start - 0.05 * step as f32)));
            slider_frame(&mut frames, &mut project, &mut drag, clip, events);
            events = Vec::new();
        }
        // The release frame moves the pointer once more.
        let end = at(start - 0.4);
        slider_frame(
            &mut frames,
            &mut project,
            &mut drag,
            clip,
            vec![Event::PointerMoved(end), button(end, false)],
        );
        slider_frame(&mut frames, &mut project, &mut drag, clip, Vec::new());

        let dragged_to = gain(&project, clip);
        assert!(
            dragged_to < GainDb::from_hundredths(-1_000),
            "{dragged_to:?}"
        );
        assert!(!project.editor.gesture_open(), "the drag ended");
        assert_eq!(project.editor.history().len(), entries + 1);
        project.undo().unwrap();
        assert_eq!(gain(&project, clip), GainDb::ZERO, "one undo: back to 0 dB");
    }
}
