//! Rust port GUI binary (initial skeleton; see Rust/PARITY.md).
//!
//! The Swift app lives in `Sources/view`. This binary is the Rust
//! replacement host: it loads a browser listing with `view_core`,
//! hosts it in an AppKit window, and will grow through the milestones
//! in `PARITY.md` (keyboard, scroll-follow, history, trash/undo,
//! Quick Look, menus, drag-out).

use std::path::PathBuf;

use objc2::{define_class, ClassType, DefinedClass, MainThreadOnly};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::NSObject;

use objc2_app_kit::{NSApplication, NSApplicationDelegate};
use objc2_foundation::{NSNotification, NSObjectProtocol};

#[derive(Default)]
struct AppState {
    #[allow(dead_code)]
    root: PathBuf,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = AppState]
    struct AppDelegate;

    impl AppDelegate {
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(AppState::default());
            unsafe { objc2::msg_send![super(this), init] }
        }
    }

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            // Skeleton: placeholder window. Listing and interaction arrive
            // in the following milestones.
            let _ = self.ivars();
        }
    }
);

fn main() {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let delegate: Retained<AppDelegate> = unsafe { objc2::msg_send![AppDelegate::class(), new] };
    app.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));
    app.run();
}
