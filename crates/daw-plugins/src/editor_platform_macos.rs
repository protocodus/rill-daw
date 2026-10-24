//! AppKit setup and snapshots, owned exclusively by the isolated editor helper.
use anyhow::{Context, Result, ensure};
use objc2::{
    MainThreadMarker,
    rc::{Retained, autoreleasepool},
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBitmapImageFileType, NSEventMask,
};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSDictionary, NSString};
use std::path::Path;

pub(super) struct Platform {
    app: Retained<NSApplication>,
}
impl Platform {
    pub fn new() -> Result<Self> {
        let mtm =
            MainThreadMarker::new().context("Native editor must run on the helper main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        // Initialization must never activate an app while capturing library thumbnails.
        app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        app.finishLaunching();
        Ok(Self { app })
    }
    pub fn configure(&self, title: &str, visible: bool) -> Result<()> {
        let windows = self.app.windows();
        let window = windows
            .iter()
            .find(|w| w.title().to_string() == title)
            .context("Native editor window is no longer available")?;
        if visible {
            self.app
                .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            window.makeKeyAndOrderFront(None);
            use objc2_foundation::NSObjectProtocol;
            if self.app.respondsToSelector(objc2::sel!(activate)) {
                self.app.activate();
            } else {
                #[allow(deprecated)]
                self.app.activateIgnoringOtherApps(true);
            }
        } else {
            window.orderOut(None);
        }
        Ok(())
    }
    pub fn pump(&self) -> Result<()> {
        autoreleasepool(|_| {
            let until = NSDate::dateWithTimeIntervalSinceNow(0.001);
            // SAFETY: AppKit is initialized and this loop owns the process main thread.
            while let Some(event) = unsafe {
                self.app.nextEventMatchingMask_untilDate_inMode_dequeue(
                    NSEventMask::Any,
                    Some(&until),
                    NSDefaultRunLoopMode,
                    true,
                )
            } {
                self.app.sendEvent(&event);
            }
            self.app.updateWindows();
        });
        Ok(())
    }
    pub fn closed_by_user(&self) -> bool {
        false
    }

    pub fn context_menu_position(&self, title: &str, x: i32, y: i32) -> Result<[f64; 2]> {
        use objc2_foundation::NSPoint;
        let point = if let Some(window) = self
            .app
            .windows()
            .iter()
            .find(|window| window.title().to_string() == title)
            && let Some(view) = window.contentView()
        {
            // VST uses the editor's top-left origin; AppKit views may be flipped.
            let bounds = view.bounds();
            let y = if view.isFlipped() {
                f64::from(y)
            } else {
                bounds.size.height - f64::from(y)
            };
            let local = NSPoint::new(f64::from(x), y);
            window.convertPointToScreen(view.convertPoint_toView(local, None))
        } else {
            objc2_app_kit::NSEvent::mouseLocation()
        };
        Ok([point.x, point.y])
    }
    pub fn capture(&self, title: &str, path: &Path) -> Result<(u32, u32)> {
        autoreleasepool(|_| capture_view(&self.app, title, path))
    }

    #[cfg(debug_assertions)]
    pub fn mouse_drag(&self, title: &str, from: [f64; 2], to: [f64; 2]) -> Result<()> {
        use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
        use objc2_foundation::NSPoint;
        ensure!(
            from.iter()
                .chain(&to)
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "Invalid test mouse coordinates"
        );
        let windows = self.app.windows();
        let window = windows
            .iter()
            .find(|w| w.title().to_string() == title)
            .context("Missing native test window")?;
        let size = window
            .contentView()
            .context("Missing native content view")?
            .bounds()
            .size;
        for step in 0..=9 {
            let amount = (step.min(8) as f64) / 8.0;
            let location = NSPoint::new(
                (from[0] + (to[0] - from[0]) * amount) * size.width,
                (1.0 - from[1] - (to[1] - from[1]) * amount) * size.height,
            );
            let kind = match step {
                0 => NSEventType::LeftMouseDown,
                9 => NSEventType::LeftMouseUp,
                _ => NSEventType::LeftMouseDragged,
            };
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(kind,location,NSEventModifierFlags::empty(),NSDate::date().timeIntervalSinceReferenceDate(),window.windowNumber(),None,step,1,1.0).context("Cannot create native mouse event")?;
            self.app.sendEvent(&event);
        }
        Ok(())
    }
}

fn capture_view(app: &NSApplication, title: &str, path: &Path) -> Result<(u32, u32)> {
    let windows = app.windows();
    let window = windows
        .iter()
        .find(|window| window.title().to_string() == title)
        .context("Native editor window is no longer available")?;
    let view = window
        .contentView()
        .context("Native editor has no content view")?;
    let bounds = view.bounds();
    ensure!(
        bounds.size.width > 0.0
            && bounds.size.height > 0.0
            && bounds.size.width <= 16_384.0
            && bounds.size.height <= 16_384.0,
        "Invalid native editor dimensions"
    );
    view.displayIfNeeded();
    let bitmap = view
        .bitmapImageRepForCachingDisplayInRect(bounds)
        .context("Native editor cannot create a bitmap snapshot")?;
    view.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
    // SAFETY: the empty dictionary has no entries that could violate the property types.
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .context("Native editor bitmap could not be encoded as PNG")?;
    let destination =
        NSString::from_str(path.to_str().context("Capture path must be valid UTF-8")?);
    ensure!(
        png.writeToFile_atomically(&destination, true),
        "Cannot write native editor screenshot"
    );
    Ok((bitmap.pixelsWide() as u32, bitmap.pixelsHigh() as u32))
}
