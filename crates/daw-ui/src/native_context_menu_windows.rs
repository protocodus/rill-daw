use super::{NativeContextMenu, NativeMenuItem};
use std::ptr;
use windows_sys::Win32::{
    Foundation::HWND,
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DestroyMenu, DestroyWindow, HMENU,
        LWA_ALPHA, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, SW_SHOW,
        SetForegroundWindow, SetLayeredWindowAttributes, ShowWindow, TPM_RETURNCMD,
        TPM_RIGHTBUTTON, TrackPopupMenuEx, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_POPUP,
    },
};

struct Menu(HMENU);
impl Drop for Menu {
    fn drop(&mut self) {
        // SAFETY: this guard owns the menu and all submenus attached to it.
        unsafe { DestroyMenu(self.0) };
    }
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: this worker's main thread created and owns this window.
        unsafe { DestroyWindow(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn build_menu(items: &[NativeMenuItem], actions: &mut Vec<u32>) -> Result<Menu, String> {
    // SAFETY: allocating a menu on the worker UI thread.
    let menu = Menu(unsafe { CreatePopupMenu() });
    if menu.0.is_null() {
        return Err("Creating native menu failed".into());
    }
    for item in items {
        let mut child = None;
        let (flags, command, label) = match item {
            NativeMenuItem::Separator => (MF_SEPARATOR, 0, wide("")),
            NativeMenuItem::Action {
                id,
                label,
                enabled,
                checked,
            } => {
                actions.push(*id);
                let flags = MF_STRING
                    | if *enabled { 0 } else { MF_GRAYED }
                    | if *checked { MF_CHECKED } else { 0 };
                // Win32 reserves zero for cancellation. Never use the VST ID as
                // a native command: zero and the entire u32 range are valid.
                (flags, actions.len(), wide(&label.replace('&', "&&")))
            }
            NativeMenuItem::Submenu { label, items } => {
                child = Some(build_menu(items, actions)?);
                (
                    MF_POPUP,
                    child.as_ref().unwrap().0 as usize,
                    wide(&label.replace('&', "&&")),
                )
            }
        };
        // SAFETY: menu and UTF-16 label remain alive throughout this call.
        if unsafe { AppendMenuW(menu.0, flags, command, label.as_ptr()) } == 0 {
            return Err("Adding native menu item failed".into());
        }
        if let Some(child) = child {
            // AppendMenu transfers lifetime of attached submenu to parent.
            std::mem::forget(child);
        }
    }
    Ok(menu)
}

pub(super) fn show(model: &NativeContextMenu) -> Result<Option<u32>, String> {
    let mut actions = Vec::new();
    let menu = build_menu(&model.items, &mut actions)?;
    let class = wide("STATIC");
    let title = wide("Rill parameter menu");
    // A transparent tool window gives menu tracking a worker-owned focus target
    // without showing an extra application window or adding a taskbar entry.
    // SAFETY: built-in STATIC class, valid strings, no borrowed window procedure.
    let window = Window(unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            model.position[0] as i32,
            model.position[1] as i32,
            1,
            1,
            ptr::null_mut(),
            ptr::null_mut(),
            GetModuleHandleW(ptr::null()),
            ptr::null(),
        )
    });
    if window.0.is_null() {
        return Err("Creating native menu owner failed".into());
    }
    // SAFETY: valid worker-owned HWND and HMENU, no callback or output pointers.
    let selected = unsafe {
        SetLayeredWindowAttributes(window.0, 0, 0, LWA_ALPHA);
        ShowWindow(window.0, SW_SHOW);
        SetForegroundWindow(window.0);
        TrackPopupMenuEx(
            menu.0,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            model.position[0] as i32,
            model.position[1] as i32,
            window.0,
            ptr::null(),
        )
    };
    Ok(usize::try_from(selected)
        .ok()
        .and_then(|command| command.checked_sub(1))
        .and_then(|index| actions.get(index).copied()))
}
