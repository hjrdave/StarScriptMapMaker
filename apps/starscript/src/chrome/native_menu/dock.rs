//! Dock-click reopen. macOS only.
//!
//! Clicking the Dock icon while the app has no visible window sends the app delegate
//! `applicationShouldHandleReopen:hasVisibleWindows:`. winit owns the delegate and does not
//! forward that message, so the method is added to its class at runtime. The method only records
//! the click, orders the window front, and wakes the event loop; the app resets to Home on its
//! next frame.
//!
//! This is the only `unsafe` code in the app (the Objective-C runtime has no safe way to add a
//! method to an existing class). It has not been compiled or run by CI.
//! `docs/release/macos-local-build.md` says how to check it by hand.

#![allow(unsafe_code)]

use std::mem;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;
use objc2::ffi::class_addMethod;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, ProtocolObject, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSApplication, NSApplicationDelegate};

static CLICKED: AtomicBool = AtomicBool::new(false);
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// Installs the Dock hook. Must run on the main thread after winit set its app delegate.
pub fn install(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else {
        // macOS only, where stderr is kept (a terminal, or the system log for a bundle), and
        // nothing the user can act on: the Dock icon just does not reopen the window.
        eprintln!("no app delegate:the Dock icon will not reopen the window");
        return;
    };
    let object: *const AnyObject = Retained::as_ptr(&delegate).cast();
    // SAFETY: the delegate is a live object, held by `delegate` for this whole function.
    let class: &AnyClass = unsafe { &*object }.class();
    let handler: extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, Bool) -> Bool = should_reopen;
    // SAFETY: the signature is `BOOL (id self, SEL, NSApplication *, BOOL)`, which is what
    // the encoding `c@:@c` says, and the function is `Imp`-compatible. `class_addMethod` does not
    // replace a method the class already has.
    let added = unsafe {
        class_addMethod(
            std::ptr::from_ref(class).cast_mut(),
            sel!(applicationShouldHandleReopen:hasVisibleWindows:),
            mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, Bool) -> Bool, Imp>(
                handler,
            ),
            c"c@:@c".as_ptr(),
        )
    };
    if !added.as_bool() {
        // As above: macOS keeps stderr, and there is nothing for the user to act on.
        eprintln!("could not hook the Dock icon: the app delegate already handles reopen");
        return;
    }
    // AppKit asks the delegate which methods it has when the delegate is set, not on each
    // message. Setting it again makes it see the new one.
    let delegate: Retained<ProtocolObject<dyn NSApplicationDelegate>> = delegate;
    app.setDelegate(None);
    app.setDelegate(Some(&delegate));
}

extern "C-unwind" fn should_reopen(
    _this: &AnyObject,
    _sel: Sel,
    _app: &AnyObject,
    has_visible_windows: Bool,
) -> Bool {
    if !has_visible_windows.as_bool() {
        CLICKED.store(true, Ordering::SeqCst);
        // A hidden window may get no frame until it is on screen, so show it here.
        if let Some(mtm) = MainThreadMarker::new() {
            let windows = NSApplication::sharedApplication(mtm).windows();
            for index in 0..windows.count() {
                windows.objectAtIndex(index).makeKeyAndOrderFront(None);
            }
        }
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }
    // The app reopens its own window; the system need not.
    Bool::NO
}

/// Whether the Dock icon was clicked with no visible window since the last call.
pub fn take_click() -> bool {
    CLICKED.swap(false, Ordering::SeqCst)
}
