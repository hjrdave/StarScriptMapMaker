//! The Home tab: a browser of the Maps the app knows of, from where the user opens and
//! creates them.
//!
//! What is a decision (which Maps a list shows, in what order, the counts, the "Edited …"
//! text) is plain data and functions in [`entries`], tested without a window. [`Home::show`]
//! draws it, with the rail and cards painted by [`cards`].
//!
//! Each Map's preview, Canvas size and modified time are read on a worker thread and cached,
//! so a long list does not stall the first frame. The Canvas size comes from
//! [`ssmap_core::file::read_canvas`], which does not open the Map.

mod cards;
mod entries;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::SystemTime;

use eframe::egui::{self, vec2};

use crate::pack_page::{self, PackPage};
use crate::theme;
use crate::update::UpdateCheck;
use cards::{map_card, new_map_card, rail_row, section_label};

pub(crate) use entries::{
    Counts, Entry, Info, OpenTab, Sort, View, counts, edited_ago, known_entries, open_entries,
    read_info, select, size_label,
};

/// What the user did on Home this frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Action {
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
    /// The folder dialog for a pack to add.
    AddPack,
    /// Install this pack folder.
    AddPackFolder(PathBuf),
    /// Ask to move the Asset pack with this id to the Trash.
    DeletePack(String),
    /// Close the message about the last Add pack.
    DismissPackStatus,
    /// Close the messages about problems.
    DismissNotices,
}

/// What the screen draws besides its own state.
pub(crate) struct Data<'a> {
    pub known: &'a [Entry],
    pub open: &'a [Entry],
    /// The installed Asset pack count.
    pub packs: usize,
    /// The Asset packs' art, for the Asset packs page.
    pub sprites: Option<&'a ssmap_render::SpriteStore>,
    /// The last Add pack: its progress or result.
    pub pack_status: Option<&'a crate::pack_install::Status>,
    /// Problems to tell the user about, such as an Asset pack that did not load.
    pub notices: &'a [String],
}

struct Loaded {
    path: PathBuf,
    info: Info,
    image: Option<egui::ColorImage>,
}

/// The Home screen's state: the chosen list, search and sort, and the cache of what was read
/// from each Map file.
#[derive(Default)]
pub(crate) struct Home {
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
    pub(crate) fn info(&self, path: &Path) -> Info {
        self.infos.get(path).copied().unwrap_or_default()
    }

    /// Drops what was read from `path`, so it is read again (after a save, say).
    pub(crate) fn forget(&mut self, path: &Path) {
        self.infos.remove(path);
        self.thumbnails.remove(path);
        self.requested.remove(path);
    }

    /// Makes the Asset packs page the list Home shows.
    pub(crate) fn show_packs(&mut self) {
        self.view = View::Packs;
    }

    /// Called each frame Home is drawn. The first frame after another tab was shown reads
    /// every file again, so times and previews are not stale. What was read stays on screen
    /// until the new reading arrives.
    pub(crate) fn entered(&mut self) {
        if !self.visible {
            self.visible = true;
            self.requested.clear();
        }
    }

    /// Called each frame another tab is drawn.
    pub(crate) fn left(&mut self) {
        self.visible = false;
    }

    /// Asks the worker to read each of `paths` not read yet.
    pub(crate) fn request<'a>(&mut self, paths: impl IntoIterator<Item = &'a Path>) {
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
    pub(crate) fn poll(&mut self, ctx: &egui::Context) -> bool {
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
    pub(crate) fn loading(&self) -> bool {
        self.outstanding > 0
    }

    /// Draws the screen and returns what the user did.
    pub(crate) fn show(
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

        if !data.notices.is_empty() {
            egui::Panel::top("home_notices")
                .frame(
                    egui::Frame::NONE
                        .fill(p.bg_window)
                        .inner_margin(egui::Margin::symmetric(32, 12)),
                )
                .show(ui, |ui| {
                    for notice in data.notices {
                        ui.label(egui::RichText::new(notice).color(p.text_primary));
                    }
                    if ui.button("Dismiss").clicked() {
                        actions.push(Action::DismissNotices);
                    }
                });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(p.bg_window))
            .show(ui, |ui| self.main(ui, data, &mut actions));
        actions
    }

    /// Draws a new tab's start page: the New Map card, the latest of the known Maps and an
    /// Open… button. A lighter Home, without the rail, search or sorting.
    pub(crate) fn start_page(&mut self, ui: &mut egui::Ui, data: &Data<'_>) -> Vec<Action> {
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
        section_label(ui, "Asset packs");
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
            for action in self.pack_page.show(ui, data.sprites, data.pack_status) {
                match action {
                    pack_page::Action::Credits => out.push(Action::Credits),
                    pack_page::Action::AddPack => out.push(Action::AddPack),
                    pack_page::Action::AddFolder(path) => out.push(Action::AddPackFolder(path)),
                    pack_page::Action::Delete(id) => out.push(Action::DeletePack(id)),
                    pack_page::Action::DismissStatus => out.push(Action::DismissPackStatus),
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
