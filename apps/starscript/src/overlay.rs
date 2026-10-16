//! The frame-time overlay (issue #309): frame time, frames per second and, in a build with the
//! `profiling` feature, the time of each stage of the last frame. Off by default; F3 toggles it
//! in debug and profiling builds. Release builds without the feature have no way to show it.

use std::time::{Duration, Instant};

use eframe::egui;
use ssmap_render::profile::{Stage, StageTimes};

use crate::theme;

/// Whether this build can show the overlay.
pub const AVAILABLE: bool = cfg!(any(debug_assertions, feature = "profiling"));

/// The frame time the overlay marks in red: the 60 fps budget (docs/design/performance.md).
const BUDGET: Duration = Duration::from_micros(16_600);

/// Where the stage times of the whole process come from, set once at startup by [`record`].
#[cfg(feature = "profiling")]
static RECORDER: std::sync::OnceLock<std::sync::Arc<ssmap_render::profile::Recorder>> =
    std::sync::OnceLock::new();

/// Starts timing the stages of every frame on the calling (UI) thread. Without the `profiling`
/// feature there is nothing to time and this does nothing.
pub fn record() {
    #[cfg(feature = "profiling")]
    if let Some(recorder) = ssmap_render::profile::Recorder::install() {
        let _ = RECORDER.set(recorder);
    }
}

#[derive(Default)]
pub struct Overlay {
    visible: bool,
    last_frame: Option<Instant>,
    /// How long the last frame took, from its start to the next one's.
    frame: Duration,
    stages: Option<StageTimes>,
}

impl Overlay {
    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        self.last_frame = None;
    }

    /// Called at the start of every frame: reads the toggle key, then the last frame's timings.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        if !AVAILABLE {
            return;
        }
        if !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F3))
        {
            self.toggle();
        }
        if !self.visible {
            return;
        }
        let now = Instant::now();
        if let Some(last) = self.last_frame.replace(now) {
            self.frame = now - last;
        }
        #[cfg(feature = "profiling")]
        {
            self.stages = RECORDER.get().map(|recorder| recorder.take());
        }
        // Keep frames coming so the numbers stay current while it is shown.
        ctx.request_repaint();
    }

    /// The overlay's lines of text, empty until a frame has been measured.
    pub fn lines(&self) -> Vec<String> {
        if self.frame.is_zero() {
            return Vec::new();
        }
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let mut lines = vec![format!(
            "frame {:.1} ms ({:.0} fps)",
            ms(self.frame),
            1.0 / self.frame.as_secs_f64()
        )];
        if let Some(stages) = &self.stages {
            lines.extend(
                Stage::ALL
                    .into_iter()
                    .map(|stage| format!("{:<13}{:>6.2} ms", stage.name(), ms(stages[stage]))),
            );
        }
        lines
    }

    pub fn show(&self, ctx: &egui::Context) {
        let lines = self.lines();
        if !self.visible || lines.is_empty() {
            return;
        }
        let palette = theme::palette(ctx);
        egui::Area::new(egui::Id::new("frame_time_overlay"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::LEFT_BOTTOM, [8.0, -8.0])
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(palette.bg_panel.gamma_multiply(0.9))
                    .stroke(egui::Stroke::new(1.0, palette.border_subtle))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        for (i, line) in lines.iter().enumerate() {
                            let mut text = egui::RichText::new(line).monospace();
                            if i == 0 && self.frame > BUDGET {
                                text = text.color(egui::Color32::LIGHT_RED);
                            }
                            ui.label(text);
                        }
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_off_until_toggled() {
        let mut overlay = Overlay::default();
        assert!(!overlay.visible());
        overlay.toggle();
        assert!(overlay.visible());
        overlay.toggle();
        assert!(!overlay.visible());
    }

    #[test]
    fn it_shows_the_frame_time_and_each_stage() {
        let mut overlay = Overlay::default();
        assert!(overlay.lines().is_empty());
        overlay.frame = Duration::from_millis(10);
        assert_eq!(overlay.lines(), ["frame 10.0 ms (100 fps)"]);
        let mut stages = StageTimes::default();
        stages[Stage::Draw] = Duration::from_micros(2500);
        overlay.stages = Some(stages);
        let lines = overlay.lines();
        assert_eq!(lines.len(), 1 + Stage::ALL.len());
        assert_eq!(lines.last().unwrap(), "draw           2.50 ms");
    }
}
