//! AppKit must run on the process main thread. This test opens no windows,
//! changes no clipboard contents, and never activates the test application.
#[cfg(target_os = "macos")]
fn main() {
    use daw_ui::with_native_dialog_menu;
    use objc2::sel;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventModifierFlags, NSEventType,
        NSMenu, NSMenuItem, NSTextView,
    };
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRange, ns_string};

    let mtm = MainThreadMarker::new().expect("AppKit test must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let menu = NSMenu::new(mtm);
    let existing = NSMenuItem::new(mtm);
    existing.setTitle(ns_string!("Application"));
    menu.addItem(&existing);
    app.setMainMenu(Some(&menu));

    let text = NSTextView::new(mtm);
    text.setString(ns_string!("Neon 🎹 Circuit.wav"));
    text.setSelectedRange(NSRange::new(4, 0));
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown,
        NSPoint::new(0.0, 0.0),
        NSEventModifierFlags::Command,
        0.0,
        0,
        None,
        ns_string!("a"),
        ns_string!("a"),
        false,
        0,
    ).unwrap();
    assert!(!menu.performKeyEquivalent(&event));
    assert_eq!(text.selectedRange(), NSRange::new(4, 0));

    let returned = with_native_dialog_menu(|| {
        assert_eq!(app.mainMenu().as_deref(), Some(&*menu));
        assert_eq!(menu.numberOfItems(), 2);
        let edit = menu.itemAtIndex(1).unwrap().submenu().unwrap();
        assert!(edit.autoenablesItems());
        for (index, action, key) in [
            (0, sel!(cut:), "x"),
            (1, sel!(copy:), "c"),
            (2, sel!(paste:), "v"),
            (3, sel!(selectAll:), "a"),
        ] {
            let item = edit.itemAtIndex(index).unwrap();
            assert_eq!(item.action(), Some(action));
            assert!(item.target().is_none());
            assert_eq!(item.keyEquivalent().to_string(), key);
            assert_eq!(
                item.keyEquivalentModifierMask(),
                NSEventModifierFlags::Command
            );
        }
        // Supply a native text target without making a window key or stealing
        // focus. Production items retain nil targets for the responder chain.
        let select_all = edit.itemAtIndex(3).unwrap();
        unsafe { select_all.setTarget(Some(&text)) };
        assert!(menu.performKeyEquivalent(&event));
        assert_eq!(text.selectedRange(), NSRange::new(0, 19));
        unsafe { select_all.setTarget(None) };
        Some("chosen path")
    });
    assert_eq!(returned, Some("chosen path"));
    assert_eq!(menu.numberOfItems(), 1);
    assert_eq!(menu.itemAtIndex(0).as_deref(), Some(&*existing));
    assert!(!menu.performKeyEquivalent(&event));

    let cancelled: Option<&str> = with_native_dialog_menu(|| None);
    assert!(cancelled.is_none());
    assert_eq!(menu.numberOfItems(), 1);
    with_native_dialog_menu(|| {
        with_native_dialog_menu(|| assert_eq!(menu.numberOfItems(), 3));
        assert_eq!(menu.numberOfItems(), 2);
    });
    assert_eq!(menu.numberOfItems(), 1);

    let panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panic = std::panic::catch_unwind(|| {
        with_native_dialog_menu(|| panic!("Simulated picker failure"));
    });
    std::panic::set_hook(panic_hook);
    assert!(panic.is_err());
    assert_eq!(menu.numberOfItems(), 1);
    println!(
        "Native picker menu: key dispatch, Unicode selection, cancellation and cleanup passed"
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {
    assert_eq!(daw_ui::with_native_dialog_menu(|| Some(42)), Some(42));
    assert_eq!(daw_ui::with_native_dialog_menu(|| None::<u8>), None);
}
