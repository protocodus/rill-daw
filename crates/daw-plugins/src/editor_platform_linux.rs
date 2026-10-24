//! X11 editor pixels, used only inside the native editor helper process.
use anyhow::{Context, Result, ensure};
use std::{cell::Cell, collections::HashSet, fs::File, path::Path};
use x11rb::{
    connection::Connection,
    image::Image,
    protocol::{
        composite::{ConnectionExt as _, Redirect},
        res::{ClientIdMask, ClientIdSpec, ConnectionExt as _},
        xproto::{
            AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as _,
            InputFocus, MapState, StackMode, VisualClass, Window,
        },
    },
    rust_connection::RustConnection,
};

pub(super) struct Platform {
    connection: RustConnection,
    window: Cell<Option<Window>>,
    hidden: Cell<bool>,
}

impl Platform {
    pub fn new() -> Result<Self> {
        let (connection, _) = x11rb::connect(None).context(
            "Native plug-in editors require an X11 display (or XWayland); DISPLAY is unavailable",
        )?;
        Ok(Self {
            connection,
            window: Cell::new(None),
            hidden: Cell::new(false),
        })
    }

    pub fn configure(&self, title: &str, visible: bool) -> Result<()> {
        let window = self.find_window(Some(title))?;
        self.window.set(Some(window));
        self.hidden.set(!visible);
        if visible {
            self.connection
                .configure_window(
                    window,
                    &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                )?
                .check()?;
            self.connection
                .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)?
                .check()?;
        } else {
            self.connection
                .composite_query_version(0, 4)?
                .reply()
                .context("Hidden editor capture requires the X11 Composite extension")?;
            self.connection
                .composite_redirect_window(window, Redirect::AUTOMATIC)?
                .check()?;
            self.connection.unmap_window(window)?.check()?;
            self.connection
                .change_window_attributes(
                    window,
                    &ChangeWindowAttributesAux::new().override_redirect(1),
                )?
                .check()?;
            self.connection
                .configure_window(window, &ConfigureWindowAux::new().x(-30_000).y(-30_000))?
                .check()?;
            self.connection.map_window(window)?.check()?;
        }
        self.connection.flush()?;
        Ok(())
    }

    pub fn pump(&self) -> Result<()> {
        // The plug-in's own X11 events/timers are serviced by its VST3 IRunLoop;
        // PluginWindow owns its separate XCB connection. Never consume its events.
        self.connection.flush()?;
        Ok(())
    }

    pub fn closed_by_user(&self) -> bool {
        self.window.get().is_some_and(|window| {
            self.connection
                .get_window_attributes(window)
                .ok()
                .is_some_and(|cookie| cookie.reply().is_err())
        })
    }

    pub fn capture(&self, title: &str, path: &Path) -> Result<(u32, u32)> {
        let window = self.find_window(Some(title))?;
        self.window.set(Some(window));
        let attributes = self.connection.get_window_attributes(window)?.reply()?;
        ensure!(
            attributes.map_state == MapState::VIEWABLE,
            "Native editor is not visible"
        );
        let geometry = self.connection.get_geometry(window)?.reply()?;
        let width = u32::from(geometry.width);
        let height = u32::from(geometry.height);
        ensure!(
            width > 0
                && height > 0
                && width <= 8192
                && height <= 8192
                && u64::from(width) * u64::from(height) <= 16 * 1024 * 1024,
            "Native editor image is empty or exceeds capture limits"
        );
        let (image, visual_id) = if self.hidden.get() {
            let pixmap = self.connection.generate_id()?;
            self.connection
                .composite_name_window_pixmap(window, pixmap)?
                .check()?;
            let captured = Image::get(
                &self.connection,
                pixmap,
                0,
                0,
                geometry.width,
                geometry.height,
            );
            self.connection.free_pixmap(pixmap)?.check()?;
            let (image, _) =
                captured.context("Reading offscreen native editor pixels from XComposite")?;
            (image, attributes.visual)
        } else {
            Image::get(
                &self.connection,
                window,
                0,
                0,
                geometry.width,
                geometry.height,
            )
            .context("Reading native editor pixels from X11")?
        };
        let visual = self
            .connection
            .setup()
            .roots
            .iter()
            .flat_map(|screen| &screen.allowed_depths)
            .flat_map(|depth| &depth.visuals)
            .find(|visual| visual.visual_id == visual_id)
            .context("X11 editor visual is missing")?;
        ensure!(
            visual.class == VisualClass::TRUE_COLOR,
            "Editor capture requires an X11 TrueColor visual"
        );
        ensure!(
            visual.red_mask != 0 && visual.green_mask != 0 && visual.blue_mask != 0,
            "X11 editor visual has invalid RGB masks"
        );
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
        for y in 0..geometry.height {
            for x in 0..geometry.width {
                let pixel = image.get_pixel(x, y);
                pixels.extend_from_slice(&[
                    component(pixel, visual.red_mask),
                    component(pixel, visual.green_mask),
                    component(pixel, visual.blue_mask),
                ]);
            }
        }
        let mut encoder = png::Encoder::new(File::create(path)?, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&pixels)?;
        Ok((width, height))
    }

    fn find_window(&self, title: Option<&str>) -> Result<Window> {
        let mut pending: Vec<_> = self
            .connection
            .setup()
            .roots
            .iter()
            .map(|screen| (screen.root, 0))
            .collect();
        let mut visited = HashSet::new();
        while let Some((window, depth)) = pending.pop() {
            if depth > 8 || !visited.insert(window) {
                continue;
            }
            ensure!(
                visited.len() <= 8192,
                "Too many X11 windows to locate the native editor"
            );
            if let Ok(cookie) = self.connection.get_property(
                false,
                window,
                AtomEnum::WM_NAME,
                AtomEnum::ANY,
                0,
                4096,
            ) && let Ok(reply) = cookie.reply()
                && reply.format == 8
            {
                let name = String::from_utf8_lossy(&reply.value);
                let matches =
                    title.map_or_else(|| name.ends_with(" - VST3"), |title| name == title);
                if matches && self.owned_by_helper(window) {
                    return Ok(window);
                }
            }
            if let Ok(cookie) = self.connection.query_tree(window)
                && let Ok(tree) = cookie.reply()
            {
                pending.extend(tree.children.into_iter().map(|child| (child, depth + 1)));
            }
        }
        anyhow::bail!(
            "Cannot locate this helper's native editor window; X11 must expose local client PIDs through XRes"
        )
    }

    fn owned_by_helper(&self, window: Window) -> bool {
        // Do not capture another application's similarly titled window. VST3's
        // wrapper does not set _NET_WM_PID, so use the server's resource owner.
        self.connection
            .res_query_client_ids(&[ClientIdSpec {
                client: window,
                mask: ClientIdMask::LOCAL_CLIENT_PID,
            }])
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|reply| {
                reply
                    .ids
                    .iter()
                    .any(|id| id.value.first() == Some(&std::process::id()))
            })
    }
}

fn component(pixel: u32, mask: u32) -> u8 {
    let shift = mask.trailing_zeros();
    let maximum = mask >> shift;
    let value = (pixel & mask) >> shift;
    ((u64::from(value) * 255 + u64::from(maximum) / 2) / u64::from(maximum)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::protocol::xproto::{CreateGCAux, CreateWindowAux, PropMode, Rectangle, WindowClass};
    use x11rb::wrapper::ConnectionExt as _;

    #[test]
    fn visual_components_support_565_and_10_bit_color() {
        assert_eq!(component(0xf800, 0xf800), 255);
        assert_eq!(component(0x07e0, 0x07e0), 255);
        assert_eq!(component(0, 0x001f), 0);
        assert_eq!(component(512 << 20, 1023 << 20), 128);
    }

    #[test]
    #[ignore = "requires an X11 display; run under xvfb-run"]
    fn captures_real_x11_window_pixels() -> Result<()> {
        let platform = Platform::new()?;
        let connection = &platform.connection;
        let screen = &connection.setup().roots[0];
        let window = connection.generate_id()?;
        connection
            .create_window(
                screen.root_depth,
                window,
                screen.root,
                0,
                0,
                24,
                16,
                0,
                WindowClass::INPUT_OUTPUT,
                screen.root_visual,
                &CreateWindowAux::new().background_pixel(screen.black_pixel),
            )?
            .check()?;
        let title = format!("Rill capture test {} - VST3", std::process::id());
        connection
            .change_property8(
                PropMode::REPLACE,
                window,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                title.as_bytes(),
            )?
            .check()?;
        connection.map_window(window)?.check()?;
        let gc = connection.generate_id()?;
        connection
            .create_gc(
                gc,
                window,
                &CreateGCAux::new().foreground(screen.white_pixel),
            )?
            .check()?;
        connection
            .poly_fill_rectangle(
                window,
                gc,
                &[Rectangle {
                    x: 0,
                    y: 0,
                    width: 12,
                    height: 16,
                }],
            )?
            .check()?;
        connection.flush()?;
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("real-window.png");
        assert_eq!(platform.capture(&title, &path)?, (24, 16));
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(File::open(path)?)).read_info()?;
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels)?;
        assert_eq!(&pixels[0..3], &[255, 255, 255]);
        assert_eq!(&pixels[23 * 3..24 * 3], &[0, 0, 0]);
        // Plug-in editors commonly render in an embedded child window. Verify
        // offscreen capture preserves those real child pixels as well.
        platform.configure(&title, false)?;
        let child = connection.generate_id()?;
        connection
            .create_window(
                screen.root_depth,
                child,
                window,
                0,
                0,
                24,
                16,
                0,
                WindowClass::INPUT_OUTPUT,
                screen.root_visual,
                &CreateWindowAux::new().background_pixel(screen.black_pixel),
            )?
            .check()?;
        connection.map_window(child)?.check()?;
        connection
            .poly_fill_rectangle(
                child,
                gc,
                &[Rectangle {
                    x: 0,
                    y: 0,
                    width: 12,
                    height: 16,
                }],
            )?
            .check()?;
        connection.flush()?;
        let hidden = temp.path().join("hidden-window.png");
        assert_eq!(platform.capture(&title, &hidden)?, (24, 16));
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(File::open(hidden)?)).read_info()?;
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels)?;
        assert_eq!(&pixels[0..3], &[255, 255, 255]);
        assert_eq!(&pixels[23 * 3..24 * 3], &[0, 0, 0]);
        connection.destroy_window(window)?.check()?;
        assert!(platform.closed_by_user());
        Ok(())
    }
}
