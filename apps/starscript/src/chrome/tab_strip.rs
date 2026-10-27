//! The strip of tabs drawn in the title-bar area: a Home tab, one tab per open Map and a +.
//!
//! Everything that is a decision (tab widths, which tab a shortcut reaches, whether Home can
//! close, a dot or a ×) is a plain function so it is tested without a window; [`show`] only
//! draws and reports what the user did as [`Action`]s.

use eframe::egui::{self, Color32, Rect, Sense, vec2};

use crate::icons::{self, Icon};
use crate::theme::{self, Palette, metrics as layout};

/// Width of one scroll chevron.
const CHEVRON: f32 = 24.0;
/// The hit area of a tab's × and the room its dot takes.
const CLOSE_HIT: f32 = 16.0;
const DOT: f32 = 7.0;
const TAB_PADDING: f32 = 12.0;
const ACTIVE_LINE: f32 = 2.0;

/// A place the window can show: the Home tab or the Map in tab `n`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Home,
    Map(usize),
}

impl Slot {
    /// Home is always first and cannot be closed; a Map tab can.
    pub(crate) fn can_close(self) -> bool {
        matches!(self, Slot::Map(_))
    }
}

/// The tab ⌘`n` (Ctrl+`n`) reaches, `n` being 1 to 9 and `maps` the number of open Maps.
/// 1 is Home, 2 to 8 the Maps in order, and 9 the last tab whatever the count, as in browsers.
pub(crate) fn slot_for_number(n: u8, maps: usize) -> Option<Slot> {
    match n {
        1 => Some(Slot::Home),
        2..=8 => {
            let index = usize::from(n) - 2;
            (index < maps).then_some(Slot::Map(index))
        }
        9 => Some(maps.checked_sub(1).map_or(Slot::Home, Slot::Map)),
        _ => None,
    }
}

/// The tab after (or before) `current`, wrapping round Home and the Maps.
pub(crate) fn cycle(current: Slot, maps: usize, forward: bool) -> Slot {
    let total = maps + 1;
    let position = match current {
        Slot::Home => 0,
        Slot::Map(index) => index + 1,
    };
    let next = if forward {
        (position + 1) % total
    } else {
        (position + total - 1) % total
    };
    match next {
        0 => Slot::Home,
        n => Slot::Map(n - 1),
    }
}

/// What sits where a Map tab's × is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Indicator {
    Nothing,
    /// The gold dot of a Map with unsaved changes.
    Dot,
    Close,
}

/// A hovered tab always offers ×, which replaces the unsaved dot. Otherwise the dot shows for
/// unsaved changes, and the active tab keeps its ×.
pub(crate) fn indicator(unsaved: bool, hovered: bool, active: bool) -> Indicator {
    if hovered {
        Indicator::Close
    } else if unsaved {
        Indicator::Dot
    } else if active {
        Indicator::Close
    } else {
        Indicator::Nothing
    }
}

/// How the Map tabs fit the room the strip has for them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fit {
    /// Width of each Map tab.
    pub tab_width: f32,
    /// Whether the tabs do not fit even at the least width, so the strip scrolls with chevrons.
    pub scrolls: bool,
    /// Width of the window onto the tabs: the room, less the chevrons when scrolling.
    pub viewport: f32,
}

impl Fit {
    /// The furthest the tabs can scroll.
    pub(crate) fn max_scroll(&self, count: usize) -> f32 {
        (count as f32 * self.tab_width - self.viewport).max(0.0)
    }
}

/// Fits `count` Map tabs in `room` points: they share it, shrinking from 220 to 120, and
/// beyond that stay at 120 and scroll.
pub(crate) fn fit(room: f32, count: usize) -> Fit {
    let room = room.max(0.0);
    if count == 0 {
        return Fit {
            tab_width: layout::MAP_TAB_WIDTH,
            scrolls: false,
            viewport: room,
        };
    }
    if count as f32 * layout::MAP_TAB_MIN > room {
        return Fit {
            tab_width: layout::MAP_TAB_MIN,
            scrolls: true,
            viewport: (room - 2.0 * CHEVRON).max(0.0),
        };
    }
    Fit {
        tab_width: (room / count as f32).clamp(layout::MAP_TAB_MIN, layout::MAP_TAB_WIDTH),
        scrolls: false,
        viewport: room,
    }
}

/// The scroll offset that shows the tab at `[left, left + width]` (relative to the first tab)
/// in a viewport `viewport` wide, moving as little as it can from `offset`.
pub(crate) fn scroll_into_view(offset: f32, viewport: f32, left: f32, width: f32) -> f32 {
    if left < offset {
        left
    } else if left + width > offset + viewport {
        left + width - viewport
    } else {
        offset
    }
}

/// One Map tab as the strip shows it.
pub(crate) struct TabInfo {
    pub name: String,
    /// The file path for the tooltip; `None` for a Map never saved.
    pub path: Option<String>,
    pub unsaved: bool,
}

/// What the user did in the strip.
#[derive(Debug, PartialEq)]
pub(crate) enum Action {
    Show(Slot),
    Close(usize),
    NewMap,
    /// Pop tab `n` out into a window of its own.
    MoveToNewWindow(usize),
    /// Put tab `n` of a popped-out window back in the main window.
    MoveToMainWindow(usize),
    /// Tab `n` was dragged and let go with the pointer at `at`, in this window's points.
    Dropped {
        index: usize,
        at: egui::Pos2,
    },
}

/// Which window a strip belongs to, which decides what its tabs' context menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    /// The main window, which also has the Home tab.
    Main,
    /// A window a tab was popped out into.
    Popped,
}

/// Where the strip and its Map tabs were last drawn, in the window's own points, so a tab
/// dropped on it lands at the right place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    pub strip: Rect,
    /// Left edge of the first Map tab when not scrolled.
    pub tabs_left: f32,
    pub tab_width: f32,
    pub scroll: f32,
}

impl Geometry {
    /// The position among the `count` Map tabs a tab dropped at `x` lands in: before the tab
    /// whose left half `x` is in, or at the end.
    pub(crate) fn drop_index(&self, x: f32, count: usize) -> usize {
        if self.tab_width <= 0.0 {
            return count;
        }
        let tabs = (x - self.tabs_left + self.scroll) / self.tab_width;
        (tabs + 0.5).floor().clamp(0.0, count as f32) as usize
    }
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            strip: Rect::NOTHING,
            tabs_left: 0.0,
            tab_width: 0.0,
            scroll: 0.0,
        }
    }
}

/// The strip's own memory between frames.
#[derive(Default)]
pub(crate) struct State {
    scroll: f32,
    revealed: Option<(Slot, usize)>,
    pub geometry: Geometry,
}

fn tab_text(name: &str, color: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(
        name.to_owned(),
        egui::FontId::new(theme::BODY, theme::ui_family()),
        color,
    );
    job.wrap = egui::text::TextWrapping {
        max_width: width.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job
}

fn fill(ui: &egui::Ui, rect: Rect, color: Color32) {
    ui.painter().rect_filled(rect, 0.0, color);
}

/// Draws the strip in the panel `ui` and returns what the user did. `inset` is the room left
/// clear at the left for the native window buttons; `custom_chrome` makes the empty part drag
/// the window and a double-click zoom it.
pub(crate) fn show(
    ui: &mut egui::Ui,
    tabs: &[TabInfo],
    current: Slot,
    state: &mut State,
    inset: f32,
    custom_chrome: bool,
    placement: Placement,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let p: &Palette = theme::palette(ui.ctx());
    let strip = ui.max_rect();
    let top = strip.min.y;
    let bottom = top + layout::TITLE_BAR_WITH_TABS;
    let column = |left: f32, width: f32| {
        Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + width, bottom))
    };

    // Behind everything else, so a tab or button wins where they overlap.
    if custom_chrome {
        let background = ui.interact(strip, ui.id().with("tab-strip-bg"), Sense::click_and_drag());
        if background.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if background.double_clicked() {
            let zoomed = ui.input(|i| i.viewport().maximized).unwrap_or(false);
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!zoomed));
        }
    }

    let flat_button = |ui: &mut egui::Ui, rect: Rect, id: &str, label: &str| {
        let response = ui.interact(rect, ui.id().with(id), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
        });
        if response.hovered() {
            fill(ui, rect, p.tab_hover);
        }
        response
    };

    // Home, which only the main window has.
    let has_home = placement == Placement::Main;
    let home_width = if has_home { layout::HOME_TAB } else { 0.0 };
    let home = column(strip.min.x + inset, home_width);
    let home_active = current == Slot::Home;
    if has_home {
        let response = ui.interact(home, ui.id().with("tab-home"), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                ui.is_enabled(),
                home_active,
                "Home",
            )
        });
        if home_active {
            fill(ui, home, p.tab_active);
            fill(
                ui,
                Rect::from_min_max(egui::pos2(home.min.x, home.max.y - ACTIVE_LINE), home.max),
                p.gold,
            );
        } else if response.hovered() {
            fill(ui, home, p.tab_hover);
        }
        let tint = if home_active {
            p.text_primary
        } else {
            p.text_tertiary
        };
        icons::paint(ui, home, Icon::Home, icons::GRID, tint);
        if response.clicked() {
            actions.push(Action::Show(Slot::Home));
        }
        response.on_hover_text("Home");
    }

    // Map tabs.
    let first = home.max.x;
    let room = (strip.max.x - first - layout::NEW_TAB_BUTTON).max(0.0);
    let fit = fit(room, tabs.len());
    let width = fit.tab_width;
    let viewport_left = if fit.scrolls { first + CHEVRON } else { first };
    let viewport = column(viewport_left, fit.viewport.min(width * tabs.len() as f32));

    // Bring the active tab into view when it, or the count, changed.
    let reveal = (current, tabs.len());
    if state.revealed != Some(reveal) {
        state.revealed = Some(reveal);
        if let Slot::Map(index) = current {
            state.scroll =
                scroll_into_view(state.scroll, fit.viewport, index as f32 * width, width);
        }
    }
    if fit.scrolls && ui.rect_contains_pointer(strip) {
        let delta = ui.input(|i| i.smooth_scroll_delta);
        state.scroll -= delta.x + delta.y;
    }
    state.scroll = state.scroll.clamp(0.0, fit.max_scroll(tabs.len()));

    state.geometry = Geometry {
        strip: column(strip.min.x, strip.width()),
        tabs_left: viewport_left,
        tab_width: width,
        scroll: state.scroll,
    };
    let pointer = ui.input(|i| i.pointer.hover_pos());
    ui.scope_builder(egui::UiBuilder::new().max_rect(viewport), |ui| {
        ui.set_clip_rect(viewport);
        for (index, info) in tabs.iter().enumerate() {
            let rect = column(viewport.min.x + index as f32 * width - state.scroll, width);
            if !rect.intersects(viewport) {
                continue;
            }
            let active = current == Slot::Map(index);
            let hovered = pointer.is_some_and(|at| rect.contains(at) && viewport.contains(at));
            let body = ui.interact(
                rect.intersect(viewport),
                ui.id().with(("tab", index)),
                Sense::click_and_drag(),
            );
            if body.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if body.drag_stopped()
                && let Some(at) = ui.input(|i| i.pointer.latest_pos())
            {
                actions.push(Action::Dropped { index, at });
            }
            body.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    active,
                    &info.name,
                )
            });
            if active {
                fill(ui, rect, p.tab_active);
            } else if hovered {
                fill(ui, rect, p.tab_hover);
            }
            fill(
                ui,
                Rect::from_min_size(rect.min, vec2(1.0, rect.height())),
                p.tab_divider,
            );
            if active {
                fill(
                    ui,
                    Rect::from_min_max(egui::pos2(rect.min.x, rect.max.y - ACTIVE_LINE), rect.max),
                    p.gold,
                );
            }

            let mark = indicator(info.unsaved, hovered, active);
            let hit = Rect::from_center_size(
                egui::pos2(rect.max.x - TAB_PADDING - CLOSE_HIT / 2.0, rect.center().y),
                vec2(CLOSE_HIT, CLOSE_HIT),
            );
            let color = if active {
                p.text_primary
            } else {
                p.text_tertiary
            };
            let galley = ui.painter().layout_job(tab_text(
                &info.name,
                color,
                width - 2.0 * TAB_PADDING - CLOSE_HIT - 4.0,
            ));
            let text_at = egui::pos2(
                rect.min.x + TAB_PADDING,
                rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(text_at, galley, color);

            match mark {
                Indicator::Nothing => {}
                Indicator::Dot => {
                    ui.painter().circle_filled(hit.center(), DOT / 2.0, p.gold);
                }
                Indicator::Close => {
                    let label = format!("Close {}", info.name);
                    let close = ui.interact(
                        hit.intersect(viewport),
                        ui.id().with(("tab-close", index)),
                        Sense::click(),
                    );
                    close.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
                    });
                    if close.hovered() {
                        ui.painter()
                            .rect_filled(hit, theme::RADIUS_CHIP, p.tab_divider);
                    }
                    icons::paint(ui, hit, Icon::Close, 12.0, color);
                    if close.clicked() {
                        actions.push(Action::Close(index));
                    }
                }
            }

            if body.clicked() && !actions.contains(&Action::Close(index)) {
                actions.push(Action::Show(Slot::Map(index)));
            }
            if body.middle_clicked() {
                actions.push(Action::Close(index));
            }
            let body = body.on_hover_text(info.path.as_deref().unwrap_or("Not saved yet"));
            body.context_menu(|ui| {
                // A popped-out window's only tab can only go back.
                if (placement == Placement::Main || tabs.len() > 1)
                    && ui.button("Move Tab to New Window").clicked()
                {
                    actions.push(Action::MoveToNewWindow(index));
                    ui.close();
                }
                if placement == Placement::Popped && ui.button("Move Tab to Main Window").clicked()
                {
                    actions.push(Action::MoveToMainWindow(index));
                    ui.close();
                }
                if ui.button("Close Tab").clicked() {
                    actions.push(Action::Close(index));
                    ui.close();
                }
            });
        }
    });

    // Chevrons, only when the tabs scroll.
    let mut plus_left = viewport.max.x;
    if fit.scrolls {
        let left = column(first, CHEVRON);
        let right = column(viewport.max.x, CHEVRON);
        let can_left = state.scroll > 0.0;
        let can_right = state.scroll < fit.max_scroll(tabs.len());
        for (rect, label, flip, can) in [
            (left, "Scroll tabs left", true, can_left),
            (right, "Scroll tabs right", false, can_right),
        ] {
            let response = flat_button(ui, rect, label, label);
            let tint = if can {
                p.text_secondary
            } else {
                p.text_tertiary
            };
            icons::paint_flipped(ui, rect, Icon::ChevronRight, icons::GRID, tint, flip);
            if response.clicked() {
                let step = if flip { -width } else { width };
                state.scroll = (state.scroll + step).clamp(0.0, fit.max_scroll(tabs.len()));
            }
        }
        plus_left = right.max.x;
    }

    // New Tab.
    let plus = column(plus_left, layout::NEW_TAB_BUTTON);
    let response = flat_button(ui, plus, "tab-new", "New Tab");
    icons::paint(ui, plus, Icon::Add, icons::GRID, p.text_secondary);
    if response.clicked() {
        actions.push(Action::NewMap);
    }
    response.on_hover_text("New Tab");

    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_share_the_room_between_120_and_220() {
        assert_eq!(fit(1000.0, 2).tab_width, 220.0);
        assert_eq!(fit(1000.0, 5).tab_width, 200.0);
        assert_eq!(fit(1000.0, 8).tab_width, 125.0);
        assert!(!fit(1000.0, 8).scrolls);
        assert_eq!(fit(1000.0, 0).tab_width, 220.0);
    }

    #[test]
    fn tabs_stay_at_120_and_scroll_when_they_do_not_fit() {
        let many = fit(1000.0, 9);
        assert!(many.scrolls);
        assert_eq!(many.tab_width, 120.0);
        assert_eq!(many.viewport, 1000.0 - 2.0 * CHEVRON);
        assert_eq!(many.max_scroll(9), 9.0 * 120.0 - many.viewport);
        assert_eq!(fit(1000.0, 8).max_scroll(8), 0.0);
        assert_eq!(fit(10.0, 3).viewport, 0.0);
    }

    #[test]
    fn scrolling_reveals_a_tab_with_the_least_movement() {
        assert_eq!(scroll_into_view(0.0, 300.0, 120.0, 120.0), 0.0);
        assert_eq!(scroll_into_view(0.0, 300.0, 240.0, 120.0), 60.0);
        assert_eq!(scroll_into_view(500.0, 300.0, 120.0, 120.0), 120.0);
    }

    #[test]
    fn number_shortcuts_reach_home_the_nth_tab_and_the_last() {
        assert_eq!(slot_for_number(1, 3), Some(Slot::Home));
        assert_eq!(slot_for_number(2, 3), Some(Slot::Map(0)));
        assert_eq!(slot_for_number(4, 3), Some(Slot::Map(2)));
        assert_eq!(slot_for_number(5, 3), None);
        assert_eq!(slot_for_number(9, 3), Some(Slot::Map(2)));
        assert_eq!(slot_for_number(9, 12), Some(Slot::Map(11)));
        assert_eq!(slot_for_number(8, 12), Some(Slot::Map(6)));
        assert_eq!(slot_for_number(9, 0), Some(Slot::Home));
        assert_eq!(slot_for_number(0, 3), None);
    }

    #[test]
    fn next_and_previous_wrap_round_home_and_the_maps() {
        assert_eq!(cycle(Slot::Home, 2, true), Slot::Map(0));
        assert_eq!(cycle(Slot::Map(1), 2, true), Slot::Home);
        assert_eq!(cycle(Slot::Home, 2, false), Slot::Map(1));
        assert_eq!(cycle(Slot::Map(0), 2, false), Slot::Home);
        assert_eq!(cycle(Slot::Home, 0, true), Slot::Home);
    }

    #[test]
    fn a_dropped_tab_lands_before_the_tab_whose_left_half_it_is_on() {
        let geometry = Geometry {
            strip: Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(800.0, 40.0)),
            tabs_left: 100.0,
            tab_width: 200.0,
            scroll: 0.0,
        };
        assert_eq!(geometry.drop_index(10.0, 3), 0);
        assert_eq!(geometry.drop_index(150.0, 3), 0);
        assert_eq!(geometry.drop_index(250.0, 3), 1);
        assert_eq!(geometry.drop_index(450.0, 3), 2);
        assert_eq!(geometry.drop_index(790.0, 3), 3);
        let scrolled = Geometry {
            scroll: 200.0,
            ..geometry
        };
        assert_eq!(scrolled.drop_index(150.0, 3), 1);
    }

    #[test]
    fn home_cannot_be_closed() {
        assert!(!Slot::Home.can_close());
        assert!(Slot::Map(0).can_close());
    }

    #[test]
    fn an_unsaved_dot_gives_way_to_the_close_cross_on_hover() {
        assert_eq!(indicator(true, false, false), Indicator::Dot);
        assert_eq!(indicator(true, false, true), Indicator::Dot);
        assert_eq!(indicator(true, true, false), Indicator::Close);
        assert_eq!(indicator(false, true, false), Indicator::Close);
        assert_eq!(indicator(false, false, true), Indicator::Close);
        assert_eq!(indicator(false, false, false), Indicator::Nothing);
    }
}
