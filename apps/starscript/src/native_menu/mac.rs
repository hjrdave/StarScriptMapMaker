//! Puts the native menu on screen with `muda`. macOS only; the mapping it draws is in the
//! parent module, where it is tested.
//!
//! This code has not been compiled or run by CI. `docs/release/macos-local-build.md` says how
//! to check it by hand.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

use muda::accelerator::{Key, KeyAccelerator, Modifiers};
use muda::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};

use super::{Accelerator, NativeEntry, NativeItem, NativeMenu};
use crate::menu::Command;

/// What choosing a native item does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Command(Command),
    OpenRecent(PathBuf),
    /// Quit goes through the window's close request, so unsaved Maps are asked about.
    Quit,
}

/// Everything the native menu shows. It is rebuilt when this changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub app: Vec<NativeItem>,
    pub menus: Vec<NativeMenu>,
    pub recent: Vec<PathBuf>,
}

/// A menu as built, with the two submenus macOS treats specially.
struct Built {
    menu: Menu,
    actions: HashMap<MenuId, Action>,
    window: Option<Submenu>,
    help: Option<Submenu>,
}

pub struct MacMenu {
    /// Kept alive: dropping a `Menu` removes it.
    built: Option<Built>,
    shown: Option<Spec>,
    events: mpsc::Receiver<MenuId>,
}

impl MacMenu {
    /// Must run on the main thread once the event loop has started.
    pub fn new(ctx: &eframe::egui::Context) -> Self {
        let (sender, events) = mpsc::channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = sender.send(event.id);
            // Wakes the event loop so the choice is handled now, not at the next input.
            ctx.request_repaint();
        }));
        Self {
            built: None,
            shown: None,
            events,
        }
    }

    /// Shows `spec`, if it differs from what is showing.
    pub fn sync(&mut self, spec: Spec) {
        if self.shown.as_ref() == Some(&spec) {
            return;
        }
        match build(&spec) {
            Ok(built) => {
                if let Some(old) = self.built.take() {
                    old.menu.remove_for_nsapp();
                }
                built.menu.init_for_nsapp();
                // muda needs these called after `init_for_nsapp`.
                if let Some(window) = &built.window {
                    window.set_as_windows_menu_for_nsapp();
                }
                if let Some(help) = &built.help {
                    help.set_as_help_menu_for_nsapp();
                }
                self.built = Some(built);
                self.shown = Some(spec);
            }
            Err(error) => eprintln!("could not build the native menu: {error}"),
        }
    }

    /// The items chosen since the last call.
    pub fn take_actions(&mut self) -> Vec<Action> {
        let Some(built) = &self.built else {
            return Vec::new();
        };
        let actions: Vec<_> = self
            .events
            .try_iter()
            .filter_map(|id| built.actions.get(&id).cloned())
            .collect();
        if !actions.is_empty() {
            // muda ticks a checkmark item itself when it is chosen; drawing the menu again
            // puts the tick back to what the model says.
            self.shown = None;
        }
        actions
    }
}

struct Builder {
    actions: HashMap<MenuId, Action>,
}

impl Builder {
    fn id(&mut self, action: Action) -> MenuId {
        let id = MenuId::new(format!("ssmap-{}", self.actions.len()));
        self.actions.insert(id.clone(), action);
        id
    }

    fn item(&mut self, item: &NativeItem) -> Result<Box<dyn IsMenuItem>, muda::Error> {
        let id = self.id(Action::Command(item.command));
        Ok(match item.checked {
            Some(on) => Box::new(CheckMenuItem::with_id(
                id,
                &item.title,
                item.enabled,
                on,
                None,
            )),
            None => {
                let native = MenuItem::with_id(id, &item.title, item.enabled, None);
                native.set_key_accelerator(item.accelerator.as_ref().map(key_accelerator))?;
                Box::new(native)
            }
        })
    }
}

fn key_accelerator(accelerator: &Accelerator) -> KeyAccelerator {
    let mut modifiers = Modifiers::empty();
    if accelerator.command {
        modifiers |= Modifiers::META;
    }
    if accelerator.shift {
        modifiers |= Modifiers::SHIFT;
    }
    if accelerator.alt {
        modifiers |= Modifiers::ALT;
    }
    KeyAccelerator::new(modifiers, Key::Character(accelerator.key.clone()))
}

fn build(spec: &Spec) -> Result<Built, muda::Error> {
    let mut builder = Builder {
        actions: HashMap::new(),
    };
    let menu = Menu::new();

    let app = Submenu::new("StarScript Map Maker", true);
    for item in &spec.app {
        app.append(&*builder.item(item)?)?;
    }
    app.append(&PredefinedMenuItem::separator())?;
    app.append(&PredefinedMenuItem::services(None))?;
    app.append(&PredefinedMenuItem::separator())?;
    app.append(&PredefinedMenuItem::hide(None))?;
    app.append(&PredefinedMenuItem::hide_others(None))?;
    app.append(&PredefinedMenuItem::show_all(None))?;
    app.append(&PredefinedMenuItem::separator())?;
    let quit = MenuItem::with_id(
        builder.id(Action::Quit),
        "Quit StarScript Map Maker",
        true,
        None,
    );
    quit.set_key_accelerator(Some(KeyAccelerator::new(
        Modifiers::META,
        Key::Character("q".to_owned()),
    )))?;
    app.append(&quit)?;
    menu.append(&app)?;

    let (mut window, mut help) = (None, None);
    for native in &spec.menus {
        let submenu = Submenu::new(native.title, true);
        for entry in &native.entries {
            match entry {
                NativeEntry::Separator => submenu.append(&PredefinedMenuItem::separator())?,
                NativeEntry::Item(item) => submenu.append(&*builder.item(item)?)?,
                NativeEntry::OpenRecent => {
                    let recent = Submenu::new("Open Recent", !spec.recent.is_empty());
                    for path in &spec.recent {
                        let title = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |name| name.to_string_lossy().into_owned(),
                        );
                        let id = builder.id(Action::OpenRecent(path.clone()));
                        recent.append(&MenuItem::with_id(id, title, true, None))?;
                    }
                    submenu.append(&recent)?;
                }
            }
        }
        menu.append(&submenu)?;
        match native.title {
            "Window" => window = Some(submenu),
            "Help" => help = Some(submenu),
            _ => {}
        }
    }
    Ok(Built {
        menu,
        actions: builder.actions,
        window,
        help,
    })
}
