//! The update check: asks once whether to look for a newer version, then fetches the release
//! feed in the background and tells the user about one. There is no self-updater; the user
//! downloads the package themselves.
//!
//! The feed is a `latest.json` asset on each GitHub Release:
//!
//! ```json
//! { "version": "1.2.3", "url": "https://…/download-page", "notes_url": "https://…/notes" }
//! ```
//!
//! `version` is `X.Y.Z` (a leading `v` is tolerated). `url` is where to download it.
//! `notes_url` is optional release notes. Unknown fields are ignored.

use std::io::Read;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use eframe::egui;
use serde::{Deserialize, Serialize};

/// The `latest.json` of the newest release.
pub(crate) const FEED_URL: &str =
    "https://github.com/hjrdave/StarScriptMapMaker/releases/latest/download/latest.json";
const STORAGE_KEY: &str = "check_for_updates";
/// The feed is a few lines; anything bigger is not ours.
const MAX_FEED_BYTES: u64 = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);

/// One release as published in `latest.json`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) struct Feed {
    pub version: String,
    pub url: String,
    #[serde(default)]
    pub notes_url: Option<String>,
}

/// `X.Y.Z` as numbers, so `1.10.0` sorts after `1.9.0`. A leading `v` is dropped; anything
/// else (pre-release suffixes, a missing part) is not a version we offer.
pub(crate) fn parse_version(text: &str) -> Option<[u64; 3]> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let mut version = [0; 3];
    for slot in &mut version {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(version)
}

/// Whether `feed` names a version after `current`.
pub(crate) fn is_newer(feed: &Feed, current: &str) -> bool {
    match (parse_version(&feed.version), parse_version(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

/// Whether `url` is safe to show as a link to click.
fn is_web_link(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// Fetches and parses the feed at `url`.
pub(crate) fn fetch(url: &str) -> Result<Feed, String> {
    let roots: Vec<_> = rustls_native_certs::load_native_certs()
        .certs
        .iter()
        .map(|der| ureq::tls::Certificate::from_der(der).to_owned())
        .collect();
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::new_with_certs(&roots))
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|err| err.to_string())?;
    let mut body = String::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_FEED_BYTES)
        .read_to_string(&mut body)
        .map_err(|err| err.to_string())?;
    serde_json::from_str(&body).map_err(|err| format!("unreadable update feed: {err}"))
}

/// What the notice window says.
enum Notice {
    Available(Feed),
    UpToDate,
    Failed(String),
}

type Fetched = Result<Feed, String>;

pub(crate) struct UpdateCheck {
    /// `None` until the user has been asked.
    enabled: Option<bool>,
    feed_url: String,
    current: String,
    pending: Option<Receiver<Fetched>>,
    /// Whether the user asked for the running check, so that "up to date" and failures are
    /// reported; an automatic check only speaks up for a newer version.
    manual: bool,
    notice: Option<Notice>,
    /// Whether this session has begun its automatic check; it runs once per launch.
    started: bool,
}

impl UpdateCheck {
    /// Reads the setting from `storage`. Without storage the choice could not be kept, so
    /// nothing is asked and nothing is checked (UI tests run this way).
    pub(crate) fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let enabled = match storage {
            Some(storage) => storage
                .get_string(STORAGE_KEY)
                .and_then(|json| serde_json::from_str(&json).ok()),
            None => Some(false),
        };
        Self::new(enabled, FEED_URL, env!("CARGO_PKG_VERSION"))
    }

    pub(crate) fn new(enabled: Option<bool>, feed_url: &str, current: &str) -> Self {
        Self {
            enabled,
            feed_url: feed_url.into(),
            current: current.into(),
            pending: None,
            manual: false,
            notice: None,
            started: false,
        }
    }

    pub(crate) fn store(&self, storage: &mut dyn eframe::Storage) {
        if let Some(enabled) = self.enabled {
            storage.set_string(STORAGE_KEY, enabled.to_string());
        }
    }

    pub(crate) fn enabled(&self) -> Option<bool> {
        self.enabled
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = Some(enabled);
    }

    /// The version a finished check found, if newer than this one.
    #[cfg(test)]
    pub(crate) fn available(&self) -> Option<&Feed> {
        match &self.notice {
            Some(Notice::Available(feed)) => Some(feed),
            _ => None,
        }
    }

    pub(crate) fn is_checking(&self) -> bool {
        self.pending.is_some()
    }

    /// Starts a check in the background, unless one is running.
    pub(crate) fn start(&mut self, manual: bool) {
        if self.pending.is_some() {
            return;
        }
        self.manual = manual;
        let (sender, receiver) = channel();
        let url = self.feed_url.clone();
        let spawned = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let _ = sender.send(fetch(&url));
            });
        match spawned {
            Ok(_) => self.pending = Some(receiver),
            Err(err) => self.finish(Err(err.to_string())),
        }
    }

    /// Takes in a finished check. Returns whether one finished.
    pub(crate) fn poll(&mut self) -> bool {
        let Some(receiver) = &self.pending else {
            return false;
        };
        let fetched = match receiver.try_recv() {
            Ok(fetched) => fetched,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("the check stopped unexpectedly".into()),
        };
        self.pending = None;
        self.finish(fetched);
        true
    }

    fn finish(&mut self, fetched: Fetched) {
        self.notice = match fetched {
            Ok(feed) if is_newer(&feed, &self.current) => Some(Notice::Available(feed)),
            Ok(_) if self.manual => Some(Notice::UpToDate),
            Err(err) if self.manual => Some(Notice::Failed(err)),
            _ => None,
        };
    }

    /// The first-launch question, the running check and the result window.
    pub(crate) fn show(&mut self, ctx: &egui::Context) {
        // While undecided nothing is checked; the Home tab asks (see [`Self::banner`]).
        if self.enabled == Some(true) && !self.started {
            self.started = true;
            self.start(false);
        }
        if self.pending.is_some() {
            if self.poll() {
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
        self.notice_window(ctx);
    }

    /// The first-launch question, as a banner at the bottom of the Home tab. Answering sets
    /// the choice, which the banner then no longer shows for.
    pub(crate) fn banner(&mut self, ui: &mut egui::Ui) {
        let p = crate::theme::palette(ui.ctx());
        let mut choice = None;
        egui::Frame::NONE
            .fill(p.bg_panel)
            .stroke(egui::Stroke::new(1.0, p.border_strong))
            .corner_radius(crate::theme::RADIUS_DOCK)
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Check for updates automatically?")
                                .size(crate::theme::BODY + 1.0)
                                .color(p.text_primary)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new(
                                "Once each time it starts. Only the app version and platform are sent — \
                                 never your maps. Change this any time in the Help menu.",
                            )
                            .size(crate::theme::BODY)
                            .color(p.text_secondary),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Check automatically").clicked() {
                            choice = Some(true);
                        }
                        if ui.button("Not now").clicked() {
                            choice = Some(false);
                        }
                    });
                });
            });
        if let Some(choice) = choice {
            self.set_enabled(choice);
        }
    }

    fn notice_window(&mut self, ctx: &egui::Context) {
        let Some(notice) = &self.notice else {
            return;
        };
        let mut open = true;
        egui::Window::new("Updates")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| match notice {
                Notice::Available(feed) => {
                    ui.label(format!("Version {} is available.", feed.version));
                    ui.label(format!("You have version {}.", self.current));
                    if is_web_link(&feed.url) {
                        ui.hyperlink_to("Download", &feed.url);
                    }
                    if let Some(notes) = feed.notes_url.as_deref().filter(|u| is_web_link(u)) {
                        ui.hyperlink_to("Release notes", notes);
                    }
                }
                Notice::UpToDate => {
                    ui.label(format!("Version {} is the latest.", self.current));
                }
                Notice::Failed(err) => {
                    ui.label(format!("Couldn’t check for updates: {err}"));
                }
            });
        if !open {
            self.notice = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    fn feed(version: &str) -> Feed {
        Feed {
            version: version.into(),
            url: "https://example.com/download".into(),
            notes_url: None,
        }
    }

    /// Serves `body` once over plain HTTP and returns the URL.
    fn serve(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/latest.json", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = std::io::Read::read(&mut stream, &mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
        });
        url
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer(&feed("1.10.0"), "1.9.0"));
        assert!(is_newer(&feed("v0.2.0"), "0.1.0"));
        assert!(!is_newer(&feed("0.1.0"), "0.1.0"));
        assert!(!is_newer(&feed("0.1.0"), "0.2.0"));
    }

    #[test]
    fn unreadable_versions_are_never_newer() {
        assert!(!is_newer(&feed("2.0"), "0.1.0"));
        assert!(!is_newer(&feed("2.0.0-beta"), "0.1.0"));
        assert!(!is_newer(&feed("1.2.3.4"), "0.1.0"));
        assert!(!is_newer(&feed("latest"), "0.1.0"));
    }

    #[test]
    fn feed_parses_with_and_without_notes() {
        let full: Feed = serde_json::from_str(
            r#"{"version":"1.2.3","url":"https://a","notes_url":"https://b","extra":1}"#,
        )
        .unwrap();
        assert_eq!(full.notes_url.as_deref(), Some("https://b"));
        let bare: Feed = serde_json::from_str(r#"{"version":"1.2.3","url":"https://a"}"#).unwrap();
        assert_eq!(bare.notes_url, None);
    }

    #[test]
    fn fetch_reads_the_feed() {
        let url = serve(r#"{"version":"9.9.9","url":"https://example.com/d"}"#);
        assert_eq!(fetch(&url).unwrap().version, "9.9.9");
    }

    #[test]
    fn fetch_rejects_a_bad_feed() {
        assert!(fetch(&serve("<html>not json</html>")).is_err());
    }

    fn wait(check: &mut UpdateCheck) {
        while !check.poll() {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn detects_a_newer_version() {
        let url = serve(r#"{"version":"9.9.9","url":"https://example.com/d"}"#);
        let mut check = UpdateCheck::new(Some(true), &url, "0.1.0");
        check.start(false);
        wait(&mut check);
        assert_eq!(check.available().map(|f| f.version.as_str()), Some("9.9.9"));
    }

    #[test]
    fn automatic_check_is_silent_when_current_or_failing() {
        let url = serve(r#"{"version":"0.1.0","url":"https://example.com/d"}"#);
        let mut check = UpdateCheck::new(Some(true), &url, "0.1.0");
        check.start(false);
        wait(&mut check);
        assert!(check.notice.is_none());

        let mut check = UpdateCheck::new(Some(true), "http://127.0.0.1:1/x", "0.1.0");
        check.start(false);
        wait(&mut check);
        assert!(check.notice.is_none());
    }

    #[test]
    fn manual_check_reports_current_and_failures() {
        let url = serve(r#"{"version":"0.1.0","url":"https://example.com/d"}"#);
        let mut check = UpdateCheck::new(Some(true), &url, "0.1.0");
        check.start(true);
        wait(&mut check);
        assert!(matches!(check.notice, Some(Notice::UpToDate)));

        let mut check = UpdateCheck::new(Some(true), "http://127.0.0.1:1/x", "0.1.0");
        check.start(true);
        wait(&mut check);
        assert!(matches!(check.notice, Some(Notice::Failed(_))));
    }

    #[test]
    fn links_must_be_web_links() {
        assert!(is_web_link("https://example.com"));
        assert!(!is_web_link("file:///etc/passwd"));
        assert!(!is_web_link("javascript:alert(1)"));
    }
}
