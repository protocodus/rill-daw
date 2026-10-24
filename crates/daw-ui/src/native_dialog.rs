//! Native dialog integration shared by every file and folder picker.

/// Run a synchronous native picker with the platform's text editing shortcuts.
///
/// AppKit requires menu key equivalents for Command-A/C/V/X in native text
/// fields. winit's default application menu does not supply these actions.
/// Scope them to the picker so they cannot consume egui's editing shortcuts.
/// Call this on the UI thread; other platforms simply execute the callback.
pub fn with_native_dialog_menu<R>(open: impl FnOnce() -> R) -> R {
    #[cfg(target_os = "macos")]
    let _menu = macos::EditMenu::install();
    open()
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::{rc::Retained, sel};
    use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
    use objc2_foundation::{MainThreadMarker, ns_string};

    pub(super) struct EditMenu {
        menu_bar: Retained<NSMenu>,
        item: Retained<NSMenuItem>,
    }

    impl EditMenu {
        pub(super) fn install() -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            let menu_bar = NSApplication::sharedApplication(mtm).mainMenu()?;
            let edit = NSMenu::initWithTitle(mtm.alloc(), ns_string!("Edit"));
            for (title, action, key) in [
                (ns_string!("Cut"), sel!(cut:), ns_string!("x")),
                (ns_string!("Copy"), sel!(copy:), ns_string!("c")),
                (ns_string!("Paste"), sel!(paste:), ns_string!("v")),
                (ns_string!("Select All"), sel!(selectAll:), ns_string!("a")),
            ] {
                // These standard AppKit actions take one object argument. A nil
                // target routes them to the native dialog's first responder.
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        mtm.alloc(),
                        title,
                        Some(action),
                        key,
                    )
                };
                item.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
                edit.addItem(&item);
            }
            let item = NSMenuItem::new(mtm);
            item.setTitle(ns_string!("Edit"));
            item.setSubmenu(Some(&edit));
            menu_bar.addItem(&item);
            Some(Self { menu_bar, item })
        }
    }

    impl Drop for EditMenu {
        fn drop(&mut self) {
            // Remove only our item, including on cancel or unwinding. Preserve
            // the existing app menu and any changes made while the picker ran.
            self.menu_bar.removeItem(&self.item);
        }
    }
}
