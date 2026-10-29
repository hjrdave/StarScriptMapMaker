//! Keeps the macOS window buttons (close, minimise, zoom) vertically centered in the tab strip.
//! macOS only.
//!
//! AppKit places the buttons for its own title bar and puts them back on resize, full screen
//! exit and similar events, so `center` runs every frame and only writes a frame that differs.
//! It has not been compiled or run by CI; see `docs/release/macos-local-build.md`.

#![allow(
    unsafe_code,
    reason = "NSView::superview is an unsafe fn in objc2-app-kit"
)]

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindowButton, NSWindowStyleMask};

const BUTTONS: [NSWindowButton; 3] = [
    NSWindowButton::CloseButton,
    NSWindowButton::MiniaturizeButton,
    NSWindowButton::ZoomButton,
];

/// Grows the title bar container of every window to `bar` points and centers the buttons in it.
pub(crate) fn center(bar: f64) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows() {
        if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
            continue;
        }
        let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
            continue;
        };
        // The button sits in the title bar view, inside the container that AppKit sizes.
        // SAFETY: `mtm` proves this is the main thread, the only thread AppKit views may be
        // used on, and `close` is a live view of a window that `NSApplication` just listed.
        let container = unsafe { close.superview().and_then(|view| view.superview()) };
        if let Some(container) = container {
            let mut frame = container.frame();
            let top = window.frame().size.height;
            if (frame.size.height - bar).abs() > 0.5 || (frame.origin.y - (top - bar)).abs() > 0.5 {
                frame.size.height = bar;
                frame.origin.y = top - bar;
                container.setFrame(frame);
            }
        }
        for kind in BUTTONS {
            let Some(button) = window.standardWindowButton(kind) else {
                continue;
            };
            let mut frame = button.frame();
            let y = ((bar - frame.size.height) / 2.0).round();
            if (frame.origin.y - y).abs() > 0.5 {
                frame.origin.y = y;
                button.setFrame(frame);
            }
        }
    }
}
