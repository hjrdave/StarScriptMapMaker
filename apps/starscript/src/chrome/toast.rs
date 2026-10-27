//! Toasts: the editor's notices, floating above the Canvas (`Components.html` → Toast).
//!
//! The queue knows nothing about windows or the real clock: callers pass `now`, so tests run
//! with a fake one. Notices are per tab, so each Editor owns a queue, and only the active tab's
//! is drawn. Time passes in the background: a toast whose time ran out while its tab was in the
//! background is dropped, and one with time left shows for the rest of it on switching back.

use std::time::{Duration, Instant};

use eframe::egui;

use crate::icons::{self, Icon};
use crate::theme;

/// How long an info toast stays after it was last shown or hovered.
const NOTICE_DURATION: Duration = Duration::from_secs(5);
/// How long an error toast stays: a few seconds longer than an info one.
const ERROR_DURATION: Duration = Duration::from_secs(10);
/// The most toasts shown at once; the oldest give way.
const MAX_TOASTS: usize = 4;
/// The toast width, from the mockup.
const WIDTH: f32 = 420.0;
/// Gap between the Canvas's top edge (just under the menu bar) and the highest toast.
const TOP_OFFSET: f32 = 16.0;
/// How far above its resting place a new toast starts, and how long it takes to slide down.
const SLIDE_DISTANCE: f32 = 24.0;
const SLIDE_SECONDS: f32 = 0.15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

/// The one action a toast can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastAction {
    /// Undo the Command just run. `steps` is the Editor's undo depth right after it, so the
    /// action goes away once the history has moved on.
    Undo { steps: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastId(u64);

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: ToastId,
    pub text: String,
    pub kind: ToastKind,
    pub action: Option<ToastAction>,
    expires: Instant,
}

/// How far above its resting place a toast is drawn `age` seconds after it first showed.
fn slide_offset(age: f32) -> f32 {
    (1.0 - (age / SLIDE_SECONDS).clamp(0.0, 1.0)) * SLIDE_DISTANCE
}

impl Toast {
    fn duration(&self) -> Duration {
        match self.kind {
            ToastKind::Info => NOTICE_DURATION,
            ToastKind::Error => ERROR_DURATION,
        }
    }

    pub fn is_error(&self) -> bool {
        self.kind == ToastKind::Error
    }
}

/// The toasts of one tab, oldest first.
#[derive(Clone, Debug, Default)]
pub(crate) struct Toasts {
    items: Vec<Toast>,
    next_id: u64,
}

impl Toasts {
    pub(crate) fn push(
        &mut self,
        text: impl Into<String>,
        kind: ToastKind,
        action: Option<ToastAction>,
        now: Instant,
    ) -> ToastId {
        let id = ToastId(self.next_id);
        self.next_id += 1;
        let mut toast = Toast {
            id,
            text: text.into(),
            kind,
            action,
            expires: now,
        };
        toast.expires = now + toast.duration();
        self.items.push(toast);
        let excess = self.items.len().saturating_sub(MAX_TOASTS);
        self.items.drain(..excess);
        id
    }

    /// Drops the toasts whose time is up.
    fn expire(&mut self, now: Instant) {
        self.items.retain(|toast| toast.expires > now);
    }

    /// Gives toast `id` its full time again: it is under the pointer.
    pub(crate) fn hold(&mut self, id: ToastId, now: Instant) {
        if let Some(toast) = self.items.iter_mut().find(|toast| toast.id == id) {
            toast.expires = now + toast.duration();
        }
    }

    pub(crate) fn dismiss(&mut self, id: ToastId) {
        self.items.retain(|toast| toast.id != id);
    }

    /// Strips the action from every toast whose action `keep` rejects; the text stays.
    pub(crate) fn retain_actions(&mut self, mut keep: impl FnMut(ToastAction) -> bool) {
        for toast in &mut self.items {
            if toast.action.is_some_and(|action| !keep(action)) {
                toast.action = None;
            }
        }
    }

    /// The toasts to show, oldest first (the newest is drawn lowest).
    pub(crate) fn items(&self) -> &[Toast] {
        &self.items
    }

    /// When the next toast runs out, for scheduling a repaint.
    fn next_expiry(&self) -> Option<Instant> {
        self.items.iter().map(|toast| toast.expires).min()
    }
}

/// Draws `toasts` top-centre over `canvas` and returns the action clicked, if any. Removes
/// expired toasts first, keeps hovered ones, and schedules the repaint that expires the rest.
pub(crate) fn show(
    ctx: &egui::Context,
    canvas: egui::Rect,
    toasts: &mut Toasts,
) -> Option<ToastAction> {
    let now = Instant::now();
    toasts.expire(now);
    let palette = theme::palette(ctx);
    let mut clicked = None;
    let mut dismissed = Vec::new();
    let mut held = Vec::new();
    // Stack downward from the top, oldest first, so the newest is lowest.
    let mut top = canvas.top() + TOP_OFFSET;
    let mut sliding = false;
    for toast in toasts.items() {
        // Egui's own clock, so the slide follows frame time; the first frame fixes the start.
        let time = ctx.input(|i| i.time);
        let first_seen = ctx.data_mut(|d| {
            *d.get_temp_mut_or_insert_with(egui::Id::new(("toast_seen", toast.id)), || time)
        });
        let slide = slide_offset((time - first_seen) as f32);
        sliding |= slide > 0.0;
        let (color, icon, border) = match toast.kind {
            ToastKind::Info => (palette.ok, Icon::Check, palette.border_strong),
            ToastKind::Error => (palette.danger, Icon::Warning, palette.danger),
        };
        let area = egui::Area::new(egui::Id::new(("toast", toast.id)))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(canvas.center().x, top - slide))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(palette.bg_raised)
                    .stroke(egui::Stroke::new(1.0, border))
                    .corner_radius(theme::RADIUS_POPOVER)
                    .shadow(theme::shadow_e1())
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 8,
                        top: 8,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        ui.set_width(WIDTH - 21.0);
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::Vec2::splat(icons::GRID),
                                egui::Sense::hover(),
                            );
                            icons::paint(ui, rect, icon, icons::GRID, color);
                            // Errors carry an icon, not just colour; name them for assistive
                            // tech too.
                            let text = if toast.is_error() {
                                format!("Error: {}", toast.text)
                            } else {
                                toast.text.clone()
                            };
                            ui.label(
                                egui::RichText::new(text)
                                    .size(theme::BODY)
                                    .color(palette.text_primary),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if toast.is_error()
                                        && icons::icon_button(ui, Icon::Close, "Dismiss").clicked()
                                    {
                                        dismissed.push(toast.id);
                                    }
                                    if let Some(action) = toast.action {
                                        let label =
                                            egui::RichText::new("Undo  ⌘Z").size(theme::BODY);
                                        if ui.button(label).clicked() {
                                            clicked = Some(action);
                                            dismissed.push(toast.id);
                                        }
                                    }
                                },
                            );
                        });
                    });
            });
        let rect = area.response.rect;
        if ctx.pointer_hover_pos().is_some_and(|p| rect.contains(p)) {
            held.push(toast.id);
        }
        top = rect.bottom() + slide + theme::SPACING[3];
    }
    for id in held {
        toasts.hold(id, now);
    }
    for id in dismissed {
        toasts.dismiss(id);
    }
    if sliding {
        ctx.request_repaint_after(Duration::from_millis(16));
    }
    if let Some(expiry) = toasts.next_expiry() {
        ctx.request_repaint_after(expiry.saturating_duration_since(now));
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn toasts_stack_with_the_newest_last() {
        let t0 = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push("first", ToastKind::Info, None, t0);
        toasts.push("second", ToastKind::Info, None, t0);
        let texts: Vec<_> = toasts.items().iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["first", "second"]);
    }

    #[test]
    fn a_new_toast_slides_down_from_above_to_its_place() {
        assert_eq!(slide_offset(0.0), SLIDE_DISTANCE);
        assert!(slide_offset(SLIDE_SECONDS / 2.0) < SLIDE_DISTANCE);
        assert_eq!(slide_offset(SLIDE_SECONDS), 0.0);
        assert_eq!(slide_offset(10.0), 0.0);
    }

    #[test]
    fn the_stack_is_capped_dropping_the_oldest() {
        let t0 = Instant::now();
        let mut toasts = Toasts::default();
        for n in 0..MAX_TOASTS + 2 {
            toasts.push(format!("{n}"), ToastKind::Info, None, t0);
        }
        assert_eq!(toasts.items().len(), MAX_TOASTS);
        assert_eq!(toasts.items()[0].text, "2");
    }

    #[test]
    fn info_fades_after_the_notice_duration() {
        let t0 = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push("saved", ToastKind::Info, None, t0);
        toasts.expire(t0 + NOTICE_DURATION - Duration::from_millis(1));
        assert_eq!(toasts.items().len(), 1);
        toasts.expire(t0 + NOTICE_DURATION);
        assert!(toasts.items().is_empty());
    }

    #[test]
    fn errors_stay_longer_than_info() {
        let t0 = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push("bad", ToastKind::Error, None, t0);
        toasts.expire(t0 + NOTICE_DURATION);
        assert_eq!(toasts.items().len(), 1);
        toasts.expire(t0 + ERROR_DURATION);
        assert!(toasts.items().is_empty());
    }

    #[test]
    fn hovering_keeps_a_toast() {
        let t0 = Instant::now();
        let mut toasts = Toasts::default();
        let id = toasts.push("saved", ToastKind::Info, None, t0);
        // Hovered up to and past the deadline: still there.
        let hovered_until = t0 + NOTICE_DURATION + secs(3);
        toasts.hold(id, hovered_until - secs(1));
        toasts.expire(hovered_until - Duration::from_millis(1));
        assert_eq!(toasts.items().len(), 1);
        // Once the pointer leaves, it runs out a full duration after the last hover.
        toasts.expire(hovered_until - secs(1) + NOTICE_DURATION);
        assert!(toasts.items().is_empty());
    }

    #[test]
    fn dismissing_removes_one_toast() {
        let t0 = Instant::now();
        let mut toasts = Toasts::default();
        let first = toasts.push("a", ToastKind::Error, None, t0);
        toasts.push("b", ToastKind::Error, None, t0);
        toasts.dismiss(first);
        assert_eq!(toasts.items().len(), 1);
        assert_eq!(toasts.items()[0].text, "b");
    }

    #[test]
    fn the_next_expiry_is_the_soonest() {
        let t0 = Instant::now();
        let mut toasts = Toasts::default();
        assert_eq!(toasts.next_expiry(), None);
        toasts.push("e", ToastKind::Error, None, t0);
        toasts.push("i", ToastKind::Info, None, t0);
        assert_eq!(toasts.next_expiry(), Some(t0 + NOTICE_DURATION));
    }

    #[test]
    fn stale_actions_are_stripped_but_the_text_stays() {
        let t0 = Instant::now();
        let mut toasts = Toasts::default();
        toasts.push(
            "x",
            ToastKind::Info,
            Some(ToastAction::Undo { steps: 1 }),
            t0,
        );
        toasts.retain_actions(|_| false);
        assert_eq!(toasts.items().len(), 1);
        assert_eq!(toasts.items()[0].action, None);
    }
}
