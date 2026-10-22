//! The Maps Home lists: which ones, in what order, with what text beside them. Plain data and
//! functions, tested without a window.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use eframe::egui;

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
    pub(super) fn label(self) -> &'static str {
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
