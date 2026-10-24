use super::{NativeContextMenu, NativeMenuItem};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained, sel};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSMenu, NSMenuItem};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSString, ns_string,
};
use std::cell::Cell;

define_class!(
    // SAFETY: NSObject has no subclass requirements; all access is main-thread-only.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<Option<u32>>]
    struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(selectItem:))]
        fn select_item(&self, item: &NSMenuItem) {
            self.ivars().set(u32::try_from(item.tag()).ok());
        }
    }
);

fn build_menu(
    mtm: MainThreadMarker,
    items: &[NativeMenuItem],
    target: &MenuTarget,
) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for entry in items {
        let item = match entry {
            NativeMenuItem::Separator => NSMenuItem::separatorItem(mtm),
            NativeMenuItem::Action {
                id,
                label,
                enabled,
                checked,
            } => {
                // SAFETY: selector has one NSMenuItem argument and a live target.
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        mtm.alloc(),
                        &NSString::from_str(label),
                        Some(sel!(selectItem:)),
                        ns_string!(""),
                    )
                };
                item.setTag(*id as isize);
                item.setEnabled(*enabled);
                item.setState(if *checked { 1 } else { 0 });
                // SAFETY: target outlives this menu and the synchronous popup call.
                unsafe { item.setTarget(Some(target)) };
                item
            }
            NativeMenuItem::Submenu { label, items } => {
                let item = NSMenuItem::new(mtm);
                item.setTitle(&NSString::from_str(label));
                item.setSubmenu(Some(&build_menu(mtm, items, target)));
                item
            }
        };
        menu.addItem(&item);
    }
    menu
}

pub(super) fn initialize_worker() -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("Native menu requires the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();
    Ok(())
}

pub(super) fn show(model: &NativeContextMenu) -> Result<Option<u32>, String> {
    let mtm = MainThreadMarker::new().ok_or("Native menu requires the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let target = MenuTarget::alloc(mtm).set_ivars(Cell::new(None));
    // SAFETY: standard NSObject initializer with our initialized ivars.
    let target: Retained<MenuTarget> = unsafe { msg_send![super(target), init] };
    let menu = build_menu(mtm, &model.items, &target);
    if app.respondsToSelector(sel!(activate)) {
        app.activate();
    } else {
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
    menu.popUpMenuPositioningItem_atLocation_inView(
        None,
        NSPoint::new(model.position[0], model.position[1]),
        None,
    );
    Ok(target.ivars().get())
}
