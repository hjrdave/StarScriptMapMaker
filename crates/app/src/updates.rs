//! The opt-in update check: fetches `latest.json` from the latest GitHub Release and tells the
//! user when a newer version exists. There is no self-updater (ADR 0008).
//!
//! The feed is a small JSON object published as an asset on every Release:
//!
//! ```json
//! {"version": "1.2.3", "url": "https://…/releases/tag/v1.2.3", "notes_url": "https://…"}
//! ```
//!
//! `version` is `X.Y.Z` (a leading `v` is tolerated), `url` is where to download, and
//! `notes_url` is optional.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use eframe::egui;

/// GitHub redirects `latest/download` to the newest Release's asset.
pub const FEED_URL: &str =
    "https://github.com/hjrdave/StarScriptMapMaker/releases/latest/download/latest.json";
const STORAGE_KEY: &str = "check_for_updates";
const TIMEOUT: Duration = Duration::from_secs(10);
/// A feed is a few hundred bytes; refuse anything wildly larger.
const MAX_FEED_BYTES: u64 = 64 * 1024;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Feed {
    pub version: String,
    pub url: String,
    pub notes_url: Option<String>,
}

/// Parses `X.Y.Z`, ignoring a leading `v` and any `-prerelease` or `+build` suffix.
pub fn parse_version(text: &str) -> Option<[u64; 3]> {
    let text = text.trim().trim_start_matches('v');
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let version = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    parts.next().is_none().then_some(version)
}

impl Feed {
    pub fn parse(json: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("bad update feed: {e}"))?;
        let field = |name: &str| value.get(name).and_then(|v| v.as_str()).map(String::from);
        let version = field("version").ok_or("update feed has no version")?;
        let url = field("url").ok_or("update feed has no url")?;
        parse_version(&version).ok_or_else(|| format!("bad version in update feed: {version}"))?;
        Ok(Feed {
            version,
            url,
            notes_url: field("notes_url"),
        })
    }

    /// Whether this feed announces a version newer than `current`.
    pub fn is_newer_than(&self, current: &str) -> bool {
        match (parse_version(&self.version), parse_version(current)) {
            (Some(latest), Some(current)) => latest > current,
            _ => false,
        }
    }
}

/// Downloads and parses the feed at `url`.
pub fn fetch_feed(url: &str) -> Result<Feed, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let json = agent
        .get(url)
        .call()
        .map_err(|e| format!("could not reach the update feed: {e}"))?
        .body_mut()
        .with_config()
        .limit(MAX_FEED_BYTES)
        .read_to_string()
        .map_err(|e| format!("could not read the update feed: {e}"))?;
    Feed::parse(&json)
}

pub struct UpdateCheck {
    /// `None` until the user has been asked.
    setting: Option<bool>,
    /// Without storage the answer can't be remembered, so the user isn't asked.
    can_ask: bool,
    feed_url: String,
    current: String,
    pending: Option<Receiver<Result<Feed, String>>>,
    /// A newer version, shown until dismissed.
    available: Option<Feed>,
    /// Set when the user asked for a check from the Help menu and nothing newer came back.
    manual: bool,
    message: Option<String>,
}

impl UpdateCheck {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        Self::with_feed(storage, FEED_URL.into(), env!("CARGO_PKG_VERSION").into())
    }

    pub fn with_feed(
        storage: Option<&dyn eframe::Storage>,
        feed_url: String,
        current: String,
    ) -> Self {
        let setting = storage
            .and_then(|storage| storage.get_string(STORAGE_KEY))
            .and_then(|json| serde_json::from_str(&json).ok());
        let mut check = Self {
            setting,
            can_ask: storage.is_some(),
            feed_url,
            current,
            pending: None,
            available: None,
            manual: false,
            message: None,
        };
        if check.setting == Some(true) {
            check.start();
        }
        check
    }

    pub fn store(&self, storage: &mut dyn eframe::Storage) {
        if let Some(setting) = self.setting {
            storage.set_string(STORAGE_KEY, setting.to_string());
        }
    }

    pub fn enabled(&self) -> bool {
        self.setting == Some(true)
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.setting = Some(enabled);
        if enabled {
            if self.pending.is_none() && self.available.is_none() {
                self.start();
            }
        } else {
            self.pending = None;
            self.available = None;
        }
    }

    /// The newer version found, if any.
    pub fn available(&self) -> Option<&Feed> {
        self.available.as_ref()
    }

    /// Whether the first-launch question is waiting for an answer.
    pub fn needs_answer(&self) -> bool {
        self.setting.is_none() && self.can_ask
    }

    /// Checks now, as asked from the Help menu, and says so if there is nothing newer.
    pub fn check_now(&mut self) {
        self.manual = true;
        self.message = None;
        self.start();
    }

    fn start(&mut self) {
        let (sender, receiver) = channel();
        let url = self.feed_url.clone();
        std::thread::spawn(move || {
            let _ = sender.send(fetch_feed(&url));
        });
        self.pending = Some(receiver);
    }

    /// Collects a finished fetch. Returns whether a check is still running.
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.pending else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => Err("the update check stopped".into()),
        };
        self.pending = None;
        let manual = std::mem::take(&mut self.manual);
        match result {
            Ok(feed) if feed.is_newer_than(&self.current) => self.available = Some(feed),
            Ok(_) if manual => self.message = Some("You have the latest version.".into()),
            Ok(_) => {}
            // A failed background check stays quiet; the user only hears about one they asked for.
            Err(error) if manual => self.message = Some(error),
            Err(_) => {}
        }
        false
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if self.poll() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if self.needs_answer() {
            let mut answer = None;
            egui::Window::new("Check for updates?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(
                        "Let StarScript Map Maker check GitHub for new versions when it starts?",
                    );
                    ui.label("It only asks for a small file and never installs anything. You can change this in the Help menu.");
                    ui.horizontal(|ui| {
                        if ui.button("Check for updates").clicked() {
                            answer = Some(true);
                        }
                        if ui.button("Don't check").clicked() {
                            answer = Some(false);
                        }
                    });
                });
            if let Some(answer) = answer {
                self.set_enabled(answer);
            }
            return;
        }
        let mut dismissed = false;
        if let Some(feed) = &self.available {
            egui::Window::new("Update available")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -40.0))
                .show(ctx, |ui| {
                    ui.label(format!("Version {} is available.", feed.version));
                    ui.hyperlink_to("Download", &feed.url);
                    if let Some(notes) = &feed.notes_url {
                        ui.hyperlink_to("Release notes", notes);
                    }
                    dismissed = ui.button("Dismiss").clicked();
                });
        } else if let Some(message) = &self.message {
            egui::Window::new("Updates")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(message);
                    dismissed = ui.button("OK").clicked();
                });
        }
        if dismissed {
            self.available = None;
            self.message = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::Storage;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    /// Serves `body` once on a local port and returns the feed URL.
    fn serve(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/latest.json", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        url
    }

    fn wait(check: &mut UpdateCheck) {
        for _ in 0..200 {
            if !check.poll() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("the check never finished");
    }

    #[test]
    fn versions_compare_numerically() {
        let feed = |version: &str| Feed {
            version: version.into(),
            url: String::new(),
            notes_url: None,
        };
        assert!(feed("0.10.0").is_newer_than("0.9.9"));
        assert!(feed("v1.0.0").is_newer_than("0.1.0"));
        assert!(!feed("0.1.0").is_newer_than("0.1.0"));
        assert!(!feed("0.1.0").is_newer_than("0.2.0"));
        assert!(!feed("garbage").is_newer_than("0.1.0"));
        assert_eq!(parse_version("1.2.3-rc.1"), Some([1, 2, 3]));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn the_feed_parses_with_or_without_notes() {
        let feed =
            Feed::parse(r#"{"version":"1.2.3","url":"https://x/d","notes_url":"https://x/n"}"#)
                .unwrap();
        assert_eq!(feed.notes_url.as_deref(), Some("https://x/n"));
        assert_eq!(
            Feed::parse(r#"{"version":"1.2.3","url":"u"}"#)
                .unwrap()
                .notes_url,
            None
        );
        assert!(Feed::parse(r#"{"version":"x","url":"u"}"#).is_err());
        assert!(Feed::parse("not json").is_err());
    }

    #[test]
    fn the_app_detects_a_newer_version_from_the_feed() {
        let url = serve(r#"{"version":"9.9.9","url":"https://example.com/d"}"#);
        let mut storage = MemoryStorage::default();
        storage.set_string(STORAGE_KEY, "true".into());
        let mut check = UpdateCheck::with_feed(Some(&storage), url, "0.1.0".into());
        wait(&mut check);
        assert_eq!(check.available().unwrap().version, "9.9.9");
    }

    #[test]
    fn an_older_or_equal_feed_announces_nothing() {
        let url = serve(r#"{"version":"0.1.0","url":"https://example.com/d"}"#);
        let mut storage = MemoryStorage::default();
        storage.set_string(STORAGE_KEY, "true".into());
        let mut check = UpdateCheck::with_feed(Some(&storage), url, "0.1.0".into());
        wait(&mut check);
        assert!(check.available().is_none());
    }

    #[test]
    fn nothing_is_fetched_until_the_user_agrees() {
        let storage = MemoryStorage::default();
        let mut check =
            UpdateCheck::with_feed(Some(&storage), "http://127.0.0.1:1/".into(), "0.1.0".into());
        assert!(check.needs_answer());
        assert!(check.pending.is_none());

        check.set_enabled(false);
        assert!(!check.needs_answer());
        assert!(check.pending.is_none());
    }

    #[test]
    fn without_storage_the_user_is_not_asked() {
        let check = UpdateCheck::with_feed(None, "http://127.0.0.1:1/".into(), "0.1.0".into());
        assert!(!check.needs_answer());
        assert!(check.pending.is_none());
    }

    #[test]
    fn the_setting_is_remembered_and_can_be_turned_off() {
        let mut storage = MemoryStorage::default();
        let mut check =
            UpdateCheck::with_feed(Some(&storage), "http://127.0.0.1:1/".into(), "0.1.0".into());
        check.set_enabled(false);
        check.store(&mut storage);
        let reloaded =
            UpdateCheck::with_feed(Some(&storage), "http://127.0.0.1:1/".into(), "0.1.0".into());
        assert!(!reloaded.enabled());
        assert!(!reloaded.needs_answer());
        assert!(reloaded.pending.is_none());
    }
}
