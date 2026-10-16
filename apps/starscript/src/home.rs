//! The Home tab: a browser of the Maps the app knows of, from where the user opens and
//! creates them.
//!
//! What is a decision (which Maps a list shows, in what order, the counts, the "Edited …"
//! text) is plain data and functions here, tested without a window. [`Home::show`] draws it.
//!
//! Each Map's preview, Canvas size and modified time are read on a worker thread and cached,
//! so a long list does not stall the first frame. The Canvas size comes from
//! [`ssmap_core::file::read_canvas`], which does not open the Map.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::SystemTime;

use eframe::egui::{self, Color32, Rect, vec2};

use crate::file_actions;
use crate::icons::{self, Icon};
use crate::pack_page::{self, PackPage};
use crate::theme;
use crate::update::UpdateCheck;

/// Which Maps the main area lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum View {
    /// Every Map file the app knows of.
    #[default]
    All,
    /// The same list, newest first.
    Recent,
    /// The Maps open in tabs, including those never saved.
    OpenTabs,
    /// The installed Asset packs, not a list of Maps.
    Packs,
}

impl View {
    pub fn title(self) -> &'static str {
        match self {
            View::All => "All maps",
            View::Recent => "Recent",
            View::OpenTabs => "Open in tabs",
            View::Packs => "Asset packs",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sort {
    #[default]
    LastEdited,
    Name,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Sort::LastEdited => "Last edited",
            Sort::Name => "Name",
        }
    }
}

/// One Map in a list.
#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    /// `None` for a Map never saved.
    pub path: Option<PathBuf>,
    pub name: String,
    /// The file's modified time.
    pub modified: Option<SystemTime>,
    /// The Canvas size in pixels, once known.
    pub canvas: Option<(u32, u32)>,
    /// The index of the tab showing this Map, if open.
    pub tab: Option<usize>,
    /// The file is gone.
    pub missing: bool,
}

impl Entry {
    pub fn is_open(&self) -> bool {
        self.tab.is_some()
    }
}

/// What reading a Map file's header gives, kept with its thumbnail.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Info {
    pub modified: Option<SystemTime>,
    pub canvas: Option<(u32, u32)>,
    pub missing: bool,
}

/// Reads what the list shows of the Map file at `path`, and its preview image. Reads only the
/// file's header, never the Map.
pub fn read_info(path: &Path) -> (Info, Option<egui::ColorImage>) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (
            Info {
                missing: true,
                ..Info::default()
            },
            None,
        );
    };
    let canvas = ssmap_core::file::read_canvas(path)
        .ok()
        .map(|canvas| (canvas.width, canvas.height));
    let info = Info {
        modified: meta.modified().ok(),
        canvas,
        missing: false,
    };
    (info, read_thumbnail(path))
}

fn read_thumbnail(path: &Path) -> Option<egui::ColorImage> {
    let png = ssmap_core::file::read_preview(path).ok()??;
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        image.as_raw(),
    ))
}

/// A Map open in a tab, as the list needs to know it.
#[derive(Clone, Debug)]
pub struct OpenTab {
    pub path: Option<PathBuf>,
    pub name: String,
    pub canvas: (u32, u32),
    /// A blank new tab: it is no Map yet, so no list shows it.
    pub blank: bool,
}

/// The known Maps, in the order the app remembers them, with the tab each is open in.
pub fn known_entries(
    recent: &[PathBuf],
    tabs: &[OpenTab],
    info: impl Fn(&Path) -> Info,
) -> Vec<Entry> {
    recent
        .iter()
        .map(|path| {
            let info = info(path);
            Entry {
                path: Some(path.clone()),
                name: file_name(path),
                modified: info.modified,
                canvas: info.canvas,
                tab: tab_of(path, tabs),
                missing: info.missing,
            }
        })
        .collect()
}

/// One entry per tab, in tab order.
pub fn open_entries(tabs: &[OpenTab], info: impl Fn(&Path) -> Info) -> Vec<Entry> {
    tabs.iter()
        .enumerate()
        .filter(|(_, tab)| !tab.blank)
        .map(|(index, tab)| {
            let info = tab.path.as_deref().map(&info).unwrap_or_default();
            Entry {
                path: tab.path.clone(),
                name: tab
                    .path
                    .as_deref()
                    .map_or_else(|| tab.name.clone(), file_name),
                modified: info.modified,
                canvas: Some(tab.canvas),
                tab: Some(index),
                missing: false,
            }
        })
        .collect()
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn tab_of(path: &Path, tabs: &[OpenTab]) -> Option<usize> {
    let wanted = canonical(path);
    tabs.iter().position(|tab| {
        tab.path
            .as_deref()
            .is_some_and(|open| open == path || canonical(open) == wanted)
    })
}

/// The Map's name: its file name without the extension.
fn file_name(path: &Path) -> String {
    path.file_stem().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The numbers beside the list names in the left rail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Counts {
    pub all: usize,
    pub recent: usize,
    pub open: usize,
}

pub fn counts(known: &[Entry], open: &[Entry]) -> Counts {
    Counts {
        all: known.len(),
        recent: known.len(),
        open: open.len(),
    }
}

/// The entries `view` lists, whose name contains `query` (ignoring case), in `sort` order.
/// Recent keeps the app's own order, newest first.
pub fn select<'a>(
    view: View,
    known: &'a [Entry],
    open: &'a [Entry],
    query: &str,
    sort: Sort,
) -> Vec<&'a Entry> {
    let source = match view {
        View::OpenTabs => open,
        View::All | View::Recent => known,
        View::Packs => &[],
    };
    let query = query.trim().to_lowercase();
    let mut entries: Vec<&Entry> = source
        .iter()
        .filter(|entry| query.is_empty() || entry.name.to_lowercase().contains(&query))
        .collect();
    if view != View::Recent {
        match sort {
            // Newest first; Maps with no file time (never saved) last.
            Sort::LastEdited => entries.sort_by_key(|entry| std::cmp::Reverse(entry.modified)),
            Sort::Name => entries.sort_by_key(|entry| entry.name.to_lowercase()),
        }
    }
    entries
}

/// "Edited 2 hours ago".
pub fn edited_ago(now: SystemTime, then: SystemTime) -> String {
    let seconds = now.duration_since(then).unwrap_or_default().as_secs();
    let plural = |n: u64, unit: &str| {
        if n == 1 {
            format!("Edited 1 {unit} ago")
        } else {
            format!("Edited {n} {unit}s ago")
        }
    };
    match seconds {
        0..60 => "Edited just now".into(),
        60..3600 => plural(seconds / 60, "minute"),
        3600..86_400 => plural(seconds / 3600, "hour"),
        86_400..2_592_000 => plural(seconds / 86_400, "day"),
        2_592_000..31_536_000 => plural(seconds / 2_592_000, "month"),
        _ => plural(seconds / 31_536_000, "year"),
    }
}

/// "4096 × 3072 px".
pub fn size_label((width, height): (u32, u32)) -> String {
    format!("{width} × {height} px")
}

/// What the user did on Home this frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    NewMap,
    /// The Open… dialog.
    OpenDialog,
    /// Open the Map file in a tab, or show its tab.
    Open(PathBuf),
    /// Show the tab.
    Focus(usize),
    /// Forget a missing file.
    Remove(PathBuf),
    /// Show the file in the system file manager.
    Reveal(PathBuf),
    /// Ask to move the file to the Trash.
    Delete(PathBuf),
    Credits,
}

/// What the screen draws besides its own state.
pub struct Data<'a> {
    pub known: &'a [Entry],
    pub open: &'a [Entry],
    /// The installed Asset pack count.
    pub packs: usize,
    /// The Asset packs' art, for the Asset packs page.
    pub sprites: Option<&'a ssmap_render::SpriteStore>,
}

struct Loaded {
    path: PathBuf,
    info: Info,
    image: Option<egui::ColorImage>,
}

/// The Home screen's state: the chosen list, search and sort, and the cache of what was read
/// from each Map file.
#[derive(Default)]
pub struct Home {
    pub view: View,
    pub query: String,
    pub sort: Sort,
    pack_page: PackPage,
    infos: HashMap<PathBuf, Info>,
    thumbnails: HashMap<PathBuf, egui::TextureHandle>,
    requested: HashSet<PathBuf>,
    /// How many requested reads have not come back yet.
    outstanding: usize,
    /// Whether Home was drawn last frame; coming back to it reads every file again.
    visible: bool,
    requests: Option<Sender<PathBuf>>,
    results: Option<Receiver<Loaded>>,
}

impl Home {
    /// What is known of the file at `path` so far; nothing until the worker has read it.
    pub fn info(&self, path: &Path) -> Info {
        self.infos.get(path).copied().unwrap_or_default()
    }

    /// Drops what was read from `path`, so it is read again (after a save, say).
    pub fn forget(&mut self, path: &Path) {
        self.infos.remove(path);
        self.thumbnails.remove(path);
        self.requested.remove(path);
    }

    /// Called each frame Home is drawn. The first frame after another tab was shown reads
    /// every file again, so times and previews are not stale. What was read stays on screen
    /// until the new reading arrives.
    pub fn entered(&mut self) {
        if !self.visible {
            self.visible = true;
            self.requested.clear();
        }
    }

    /// Called each frame another tab is drawn.
    pub fn left(&mut self) {
        self.visible = false;
    }

    /// Asks the worker to read each of `paths` not read yet.
    pub fn request<'a>(&mut self, paths: impl IntoIterator<Item = &'a Path>) {
        for path in paths {
            if self.requested.contains(path) {
                continue;
            }
            self.requested.insert(path.to_path_buf());
            if self.requests.is_none() {
                self.spawn_worker();
            }
            if let Some(requests) = &self.requests
                && requests.send(path.to_path_buf()).is_ok()
            {
                self.outstanding += 1;
            }
        }
    }

    fn spawn_worker(&mut self) {
        let (request_tx, request_rx) = channel::<PathBuf>();
        let (result_tx, result_rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("home-thumbnails".into())
            .spawn(move || {
                // Ends when the screen is dropped and the request channel closes.
                for path in request_rx {
                    let (info, image) = read_info(&path);
                    if result_tx.send(Loaded { path, info, image }).is_err() {
                        break;
                    }
                }
            });
        if spawned.is_ok() {
            self.requests = Some(request_tx);
            self.results = Some(result_rx);
        }
    }

    /// Takes in what the worker has read. Returns whether anything arrived.
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        let Some(results) = &self.results else {
            return false;
        };
        let mut arrived = false;
        while let Ok(loaded) = results.try_recv() {
            self.outstanding = self.outstanding.saturating_sub(1);
            // A file forgotten while its read was running is read again.
            if !self.requested.contains(&loaded.path) {
                continue;
            }
            match loaded.image {
                Some(image) => {
                    let texture = ctx.load_texture(
                        format!("home:{}", loaded.path.display()),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.thumbnails.insert(loaded.path.clone(), texture);
                }
                None => {
                    self.thumbnails.remove(&loaded.path);
                }
            }
            self.infos.insert(loaded.path, loaded.info);
            arrived = true;
        }
        arrived
    }

    /// Whether reads are still outstanding.
    pub fn loading(&self) -> bool {
        self.outstanding > 0
    }

    /// Draws the screen and returns what the user did.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        data: &Data<'_>,
        updates: &mut UpdateCheck,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let p = theme::palette(ui.ctx());
        let counts = counts(data.known, data.open);

        egui::Panel::left("home_rail")
            .exact_size(RAIL_WIDTH)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(p.bg_panel)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| self.rail(ui, data, counts, &mut actions));

        if updates.enabled().is_none() {
            egui::Panel::bottom("home_update_banner")
                .frame(
                    egui::Frame::NONE
                        .fill(p.bg_window)
                        .inner_margin(egui::Margin::symmetric(32, 20)),
                )
                .show(ui, |ui| updates.banner(ui));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(p.bg_window))
            .show(ui, |ui| self.main(ui, data, &mut actions));
        actions
    }

    /// Draws a new tab's start page: the New Map card, the latest of the known Maps and an
    /// Open… button. A lighter Home, without the rail, search or sorting.
    pub fn start_page(&mut self, ui: &mut egui::Ui, data: &Data<'_>) -> Vec<Action> {
        let mut out = Vec::new();
        let p = theme::palette(ui.ctx());
        let now = SystemTime::now();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(p.bg_window))
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin {
                        left: 32,
                        right: 32,
                        top: 24,
                        bottom: 0,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let title = if data.known.is_empty() {
                                "New Tab"
                            } else {
                                "Recent maps"
                            };
                            ui.label(
                                egui::RichText::new(title)
                                    .size(20.0)
                                    .color(p.text_primary)
                                    .strong(),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("Open…").clicked() {
                                        out.push(Action::OpenDialog);
                                    }
                                },
                            );
                        });
                    });
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::symmetric(24, 16))
                            .show(ui, |ui| {
                                let gap = 8.0;
                                let width = ((ui.available_width() - gap * (COLUMNS as f32 - 1.0))
                                    / COLUMNS as f32)
                                    .floor()
                                    .max(120.0);
                                ui.spacing_mut().item_spacing = vec2(gap, gap);
                                let mut cards: Vec<Option<&Entry>> = vec![None];
                                cards.extend(data.known.iter().take(START_RECENT).map(Some));
                                for row in cards.chunks(COLUMNS) {
                                    ui.horizontal_top(|ui| {
                                        for card in row {
                                            match card {
                                                None => new_map_card(ui, width, &mut out),
                                                Some(entry) => {
                                                    let texture = entry
                                                        .path
                                                        .as_deref()
                                                        .and_then(|path| self.thumbnails.get(path));
                                                    map_card(
                                                        ui, width, entry, texture, now, &mut out,
                                                    );
                                                }
                                            }
                                        }
                                    });
                                }
                            });
                    });
            });
        out
    }

    fn rail(&mut self, ui: &mut egui::Ui, data: &Data<'_>, counts: Counts, out: &mut Vec<Action>) {
        let lists = [
            (View::All, counts.all),
            (View::Recent, counts.recent),
            (View::OpenTabs, counts.open),
        ];
        for (view, count) in lists {
            if rail_row(ui, self.view == view, view.title(), Some(count)).clicked() {
                self.view = view;
            }
        }
        ui.add_space(12.0);
        section_label(ui, "Library");
        if rail_row(
            ui,
            self.view == View::Packs,
            View::Packs.title(),
            Some(data.packs),
        )
        .clicked()
        {
            self.view = View::Packs;
        }
        if rail_row(ui, false, "Credits & licences", None).clicked() {
            out.push(Action::Credits);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                    .size(theme::CAPTION)
                    .color(theme::palette(ui.ctx()).text_tertiary),
            );
        });
    }

    fn main(&mut self, ui: &mut egui::Ui, data: &Data<'_>, out: &mut Vec<Action>) {
        let p = theme::palette(ui.ctx());
        if self.view == View::Packs {
            for action in self.pack_page.show(ui, data.sprites) {
                match action {
                    pack_page::Action::Credits => out.push(Action::Credits),
                }
            }
            return;
        }
        egui::Frame::NONE
            .inner_margin(egui::Margin {
                left: 32,
                right: 32,
                top: 24,
                bottom: 0,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(self.view.title())
                            .size(20.0)
                            .color(p.text_primary)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new("Drop a .ssmap file anywhere to open it in a new tab")
                            .size(theme::CAPTION)
                            .color(p.text_tertiary),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Open…").clicked() {
                            out.push(Action::OpenDialog);
                        }
                        egui::ComboBox::from_id_salt("home_sort")
                            .selected_text(self.sort.label())
                            .show_ui(ui, |ui| {
                                for sort in [Sort::LastEdited, Sort::Name] {
                                    ui.selectable_value(&mut self.sort, sort, sort.label());
                                }
                            });
                        ui.add(
                            egui::TextEdit::singleline(&mut self.query)
                                .hint_text("Search maps")
                                .desired_width(200.0),
                        );
                    });
                });
            });

        let entries = select(self.view, data.known, data.open, &self.query, self.sort);
        let now = SystemTime::now();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(24, 16))
                    .show(ui, |ui| {
                        let gap = 8.0;
                        let width = ((ui.available_width() - gap * (COLUMNS as f32 - 1.0))
                            / COLUMNS as f32)
                            .floor()
                            .max(120.0);
                        ui.spacing_mut().item_spacing = vec2(gap, gap);
                        // The New Map card comes first, then the Maps.
                        let mut cards: Vec<Option<&Entry>> = vec![None];
                        cards.extend(entries.iter().copied().map(Some));
                        for row in cards.chunks(COLUMNS) {
                            ui.horizontal_top(|ui| {
                                for card in row {
                                    match card {
                                        None => new_map_card(ui, width, out),
                                        Some(entry) => {
                                            let texture = entry
                                                .path
                                                .as_deref()
                                                .and_then(|path| self.thumbnails.get(path));
                                            map_card(ui, width, entry, texture, now, out);
                                        }
                                    }
                                }
                            });
                        }
                        if data.known.is_empty() && data.open.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(
                                    "No Maps yet. Create one, open a .ssmap file, or drop one \
                                     on the window.",
                                )
                                .color(p.text_secondary),
                            );
                        } else if entries.is_empty() {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("No Maps match.").color(p.text_secondary));
                        }
                    });
            });
    }
}

const RAIL_WIDTH: f32 = 232.0;
const COLUMNS: usize = 4;
/// How many recent Maps a new tab's start page lists after the New Map card: two rows.
const START_RECENT: usize = 7;
const ROW_HEIGHT: f32 = 28.0;

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(theme::CAPTION)
            .color(theme::palette(ui.ctx()).text_tertiary),
    );
    ui.add_space(2.0);
}

fn rail_row(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    count: Option<usize>,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    paint_rail_row(ui, rect, selected, response.hovered(), label, count);
    response
}

fn paint_rail_row(
    ui: &egui::Ui,
    rect: Rect,
    selected: bool,
    hovered: bool,
    label: &str,
    count: Option<usize>,
) {
    let p = theme::palette(ui.ctx());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, theme::RADIUS_BUTTON, p.gold_subtle);
    } else if hovered {
        painter.rect_filled(rect, theme::RADIUS_BUTTON, p.hover_fill);
    }
    let color = if selected {
        p.gold_text
    } else {
        p.text_secondary
    };
    painter.text(
        egui::pos2(rect.min.x + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::new(theme::BODY, theme::ui_family()),
        color,
    );
    if let Some(count) = count {
        painter.text(
            egui::pos2(rect.max.x - 10.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::new(theme::CAPTION, theme::ui_family()),
            p.text_tertiary,
        );
    }
}

/// One line of text, cut with an ellipsis at `width`.
fn line(ui: &egui::Ui, at: egui::Pos2, width: f32, text: &str, size: f32, color: Color32) {
    let mut job = egui::text::LayoutJob::simple_singleline(
        text.to_owned(),
        egui::FontId::new(size, theme::ui_family()),
        color,
    );
    job.wrap = egui::text::TextWrapping {
        max_width: width.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(at, galley, color);
}

const FOOTER_HEIGHT: f32 = 52.0;

fn thumb_height(width: f32) -> f32 {
    // The preview is 16:10, as in the mockup.
    (width * 0.625).round()
}

fn card_size(width: f32) -> egui::Vec2 {
    vec2(width, thumb_height(width) + 1.0 + FOOTER_HEIGHT)
}

fn new_map_card(ui: &mut egui::Ui, width: f32, out: &mut Vec<Action>) {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(card_size(width), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            "New Map: blank or from template",
        )
    });
    let preview = Rect::from_min_size(rect.min, vec2(width, thumb_height(width)));
    let stroke = egui::Stroke::new(
        1.0,
        if response.hovered() {
            p.gold
        } else {
            p.border_strong
        },
    );
    let corners = [
        preview.left_top(),
        preview.right_top(),
        preview.right_bottom(),
        preview.left_bottom(),
        preview.left_top(),
    ];
    ui.painter()
        .extend(egui::Shape::dashed_line(&corners, stroke, 4.0, 3.0));
    icons::paint(
        ui,
        Rect::from_center_size(preview.center() - vec2(0.0, 12.0), vec2(24.0, 24.0)),
        Icon::Add,
        24.0,
        p.gold,
    );
    ui.painter().text(
        preview.center() + vec2(0.0, 14.0),
        egui::Align2::CENTER_CENTER,
        "New Map",
        egui::FontId::new(theme::BODY, theme::ui_family()),
        p.text_secondary,
    );
    let below = preview.max.y + 10.0;
    line(
        ui,
        egui::pos2(rect.min.x + 2.0, below),
        width - 4.0,
        "Blank or from template",
        theme::BODY,
        p.text_primary,
    );
    line(
        ui,
        egui::pos2(rect.min.x + 2.0, below + 18.0),
        width - 4.0,
        NEW_MAP_SHORTCUT,
        theme::CAPTION,
        p.text_tertiary,
    );
    if response.clicked() {
        out.push(Action::NewMap);
    }
}

/// How the New Map shortcut reads.
const NEW_MAP_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⌘N"
} else {
    "Ctrl+N"
};

fn map_card(
    ui: &mut egui::Ui,
    width: f32,
    entry: &Entry,
    texture: Option<&egui::TextureHandle>,
    now: SystemTime,
    out: &mut Vec<Action>,
) {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(card_size(width), egui::Sense::click());
    let label = format!("Open {}", entry.name);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let preview = Rect::from_min_size(rect.min, vec2(width, thumb_height(width)));
    let radius = theme::RADIUS_FIELD;
    let more_id = response.id.with("more");
    let menu_open = egui::Popup::is_id_open(ui.ctx(), more_id.with("popup"))
        || egui::Popup::is_id_open(ui.ctx(), response.id.with("context"));
    let hovered = ui.rect_contains_pointer(rect) || menu_open;
    let painter = ui.painter();
    painter.rect_filled(rect, radius, p.bg_raised);
    painter.rect_filled(
        preview,
        egui::CornerRadius {
            nw: radius,
            ne: radius,
            sw: 0,
            se: 0,
        },
        p.bg_canvas,
    );
    match texture {
        Some(texture) => {
            // Shown at about 2×: the stored preview is up to 512 px on its longer side.
            let size = texture.size_vec2();
            let scale = ((preview.width() - 16.0) / size.x).min((preview.height() - 16.0) / size.y);
            let image = Rect::from_center_size(preview.center(), size * scale);
            painter.image(
                texture.id(),
                image,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            let text = if entry.missing {
                "File missing"
            } else {
                "No preview"
            };
            painter.text(
                preview.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::new(theme::CAPTION, theme::ui_family()),
                if entry.missing {
                    p.danger
                } else {
                    p.text_tertiary
                },
            );
        }
    }
    painter.hline(
        rect.x_range(),
        preview.max.y + 0.5,
        egui::Stroke::new(1.0, p.border_subtle),
    );
    painter.rect_stroke(
        rect,
        radius,
        egui::Stroke::new(1.0, if hovered { p.gold } else { p.border_subtle }),
        egui::StrokeKind::Inside,
    );

    // The footer: file-type icon, name, and when it was edited.
    let footer_top = preview.max.y + 1.0;
    let text_x = rect.min.x + 14.0;
    let text_width = (rect.max.x - text_x - 36.0).max(0.0);
    line(
        ui,
        egui::pos2(text_x, footer_top + 8.0),
        text_width,
        &entry.name,
        theme::BODY,
        p.text_primary,
    );
    let meta = if entry.missing {
        "Missing".to_owned()
    } else {
        entry
            .modified
            .map(|then| edited_ago(now, then))
            .unwrap_or_default()
    };
    line(
        ui,
        egui::pos2(text_x, footer_top + 28.0),
        text_width,
        &meta,
        theme::CAPTION,
        if entry.missing {
            p.danger
        } else {
            p.text_tertiary
        },
    );

    let mut removed = false;
    if let (true, Some(path)) = (entry.missing, &entry.path) {
        let button = Rect::from_center_size(preview.center() + vec2(0.0, 26.0), vec2(130.0, 24.0));
        if ui
            .put(button, egui::Button::new("Remove from list"))
            .clicked()
        {
            out.push(Action::Remove(path.clone()));
            removed = true;
        }
    }
    // The "…" button, shown while the card is hovered or its menu is open.
    if entry.path.is_some() {
        let button = Rect::from_center_size(
            egui::pos2(rect.max.x - 22.0, footer_top + FOOTER_HEIGHT / 2.0),
            vec2(24.0, 24.0),
        );
        let more = ui.interact(button, more_id, egui::Sense::click());
        more.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "More actions")
        });
        if hovered || more.has_focus() {
            if more.hovered() {
                ui.painter()
                    .rect_filled(button, theme::RADIUS_BUTTON, p.hover_fill);
            }
            icons::paint(ui, button, Icon::More, 16.0, p.text_secondary);
        }
        egui::Popup::menu(&more)
            .id(more_id.with("popup"))
            .show(|ui| card_menu(ui, entry, out));
        removed |= more.clicked();
    }
    response.context_menu(|ui| card_menu(ui, entry, out));

    // No tooltip over an open menu, where it would cover the items.
    let menu_open =
        egui::Popup::is_id_open(ui.ctx(), more_id.with("popup")) || response.context_menu_opened();
    if !menu_open {
        let mut parts = Vec::new();
        if let Some(canvas) = entry.canvas {
            parts.push(size_label(canvas));
        }
        if entry.is_open() {
            parts.push("Open in a tab".to_owned());
        }
        if !parts.is_empty() {
            response.clone().on_hover_text(parts.join(" · "));
        }
    }
    if response.clicked() && !removed && !entry.missing {
        match (entry.tab, &entry.path) {
            (Some(tab), _) => out.push(Action::Focus(tab)),
            (None, Some(path)) => out.push(Action::Open(path.clone())),
            (None, None) => {}
        }
    }
}

/// The items of a card's "…" and right-click menus.
fn card_menu(ui: &mut egui::Ui, entry: &Entry, out: &mut Vec<Action>) {
    let Some(path) = &entry.path else {
        return;
    };
    if !entry.missing {
        let open = if entry.is_open() { "Show tab" } else { "Open" };
        if ui.button(open).clicked() {
            match entry.tab {
                Some(tab) => out.push(Action::Focus(tab)),
                None => out.push(Action::Open(path.clone())),
            }
            ui.close();
        }
    }
    if ui
        .add_enabled(
            !entry.missing,
            egui::Button::new(file_actions::REVEAL_LABEL),
        )
        .clicked()
    {
        out.push(Action::Reveal(path.clone()));
        ui.close();
    }
    ui.separator();
    if entry.missing {
        if ui.button("Remove from list").clicked() {
            out.push(Action::Remove(path.clone()));
            ui.close();
        }
    } else if ui
        .button(egui::RichText::new("Delete…").color(theme::palette(ui.ctx()).danger))
        .clicked()
    {
        out.push(Action::Delete(path.clone()));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::file::MapFile;
    use ssmap_core::{Canvas, DistanceUnit, Map, Scale};
    use std::time::Duration;

    fn entry(name: &str, age: Option<u64>, tab: Option<usize>) -> Entry {
        Entry {
            path: Some(PathBuf::from(format!("/maps/{name}.ssmap"))),
            name: name.into(),
            modified: age.map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - s)),
            canvas: Some((100, 100)),
            tab,
            missing: false,
        }
    }

    fn names(entries: &[&Entry]) -> Vec<String> {
        entries.iter().map(|e| e.name.clone()).collect()
    }

    /// Known in the app's order: Delta (newest opened), then Alpha, then Charlie.
    fn known() -> Vec<Entry> {
        vec![
            entry("Delta", Some(500), None),
            entry("alpha", Some(10), Some(0)),
            entry("Charlie", Some(900), None),
        ]
    }

    #[test]
    fn last_edited_lists_the_newest_file_first_and_name_ignores_case() {
        let known = known();
        let by_time = select(View::All, &known, &[], "", Sort::LastEdited);
        assert_eq!(names(&by_time), ["alpha", "Delta", "Charlie"]);
        let by_name = select(View::All, &known, &[], "", Sort::Name);
        assert_eq!(names(&by_name), ["alpha", "Charlie", "Delta"]);
    }

    #[test]
    fn recent_keeps_the_apps_order_whatever_the_sort() {
        let known = known();
        let recent = select(View::Recent, &known, &[], "", Sort::Name);
        assert_eq!(names(&recent), ["Delta", "alpha", "Charlie"]);
    }

    #[test]
    fn search_filters_by_name_ignoring_case_and_spaces_around() {
        let known = known();
        let found = select(View::All, &known, &[], "  AL ", Sort::Name);
        assert_eq!(names(&found), ["alpha"]);
        assert!(select(View::All, &known, &[], "zzz", Sort::Name).is_empty());
    }

    #[test]
    fn never_saved_maps_sort_after_saved_ones() {
        let mut open = vec![
            entry("Untitled", None, Some(1)),
            entry("Isle", Some(5), Some(0)),
        ];
        open[0].path = None;
        let listed = select(View::OpenTabs, &[], &open, "", Sort::LastEdited);
        assert_eq!(names(&listed), ["Isle", "Untitled"]);
    }

    #[test]
    fn counts_include_never_saved_tabs() {
        let known = known();
        let tabs = [
            OpenTab {
                path: None,
                name: "Untitled".into(),
                canvas: (256, 256),
                blank: false,
            },
            OpenTab {
                path: Some("/maps/alpha.ssmap".into()),
                name: "alpha.ssmap".into(),
                canvas: (100, 100),
                blank: false,
            },
        ];
        let open = open_entries(&tabs, |_| Info::default());
        let counts = counts(&known, &open);
        assert_eq!((counts.all, counts.recent, counts.open), (3, 3, 2));
        assert_eq!(open[0].name, "Untitled");
        assert_eq!(open[0].canvas, Some((256, 256)));
        assert_eq!(open[1].name, "alpha");
    }

    #[test]
    fn a_blank_tab_is_in_no_list() {
        let tabs = [
            OpenTab {
                path: None,
                name: "New Tab".into(),
                canvas: (1, 1),
                blank: true,
            },
            OpenTab {
                path: Some("/maps/alpha.ssmap".into()),
                name: "alpha.ssmap".into(),
                canvas: (100, 100),
                blank: false,
            },
        ];
        let open = open_entries(&tabs, |_| Info::default());
        assert_eq!(open.len(), 1);
        // The index is the tab's own, so Focus reaches the right tab.
        assert_eq!(open[0].tab, Some(1));
    }

    #[test]
    fn a_known_map_open_in_a_tab_is_marked_with_its_tab() {
        let recent = [
            PathBuf::from("/maps/a.ssmap"),
            PathBuf::from("/maps/b.ssmap"),
        ];
        let tabs = [
            OpenTab {
                path: None,
                name: "Untitled".into(),
                canvas: (1, 1),
                blank: false,
            },
            OpenTab {
                path: Some("/maps/b.ssmap".into()),
                name: "b.ssmap".into(),
                canvas: (1, 1),
                blank: false,
            },
        ];
        let known = known_entries(&recent, &tabs, |_| Info::default());
        assert_eq!(known[0].tab, None);
        assert!(!known[0].is_open());
        assert_eq!(known[1].tab, Some(1));
        assert!(known[1].is_open());
    }

    #[test]
    fn edited_times_read_naturally() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100_000_000);
        let ago = |s| edited_ago(now, now - Duration::from_secs(s));
        assert_eq!(ago(5), "Edited just now");
        assert_eq!(ago(60), "Edited 1 minute ago");
        assert_eq!(ago(2 * 3600 + 5), "Edited 2 hours ago");
        assert_eq!(ago(86_400), "Edited 1 day ago");
        assert_eq!(ago(3 * 2_592_000), "Edited 3 months ago");
        // A file time in the future is not "negative".
        assert_eq!(
            edited_ago(now, now + Duration::from_secs(60)),
            "Edited just now"
        );
    }

    #[test]
    fn the_size_label_gives_the_canvas_in_pixels() {
        assert_eq!(size_label((4096, 3072)), "4096 × 3072 px");
    }

    #[test]
    fn info_reads_the_canvas_and_time_of_a_map_file_written_here() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Isle.ssmap");
        let canvas = Canvas::new(640, 480, Scale::new(1.0, DistanceUnit::Miles));
        MapFile::create(&path, &Map::new(canvas), None).unwrap();
        let (info, image) = read_info(&path);
        assert_eq!(info.canvas, Some((640, 480)));
        assert!(info.modified.is_some());
        assert!(!info.missing);
        assert!(image.is_none(), "no preview was stored");

        let (gone, _) = read_info(&dir.path().join("gone.ssmap"));
        assert!(gone.missing);
        assert_eq!(gone.canvas, None);
    }
}
