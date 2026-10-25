//! Win32 editor capture. All window procedures and GDI work stay in the helper.
use anyhow::{Result, ensure};
use std::{cell::Cell, ffi::c_void, fs::File, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, GdiFlush, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SelectObject,
    },
    Storage::Xps::{PW_CLIENTONLY, PrintWindow},
    UI::WindowsAndMessaging::{
        DispatchMessageW, EnumWindows, GetClientRect, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, HWND_BOTTOM, IsWindow, MSG, PM_REMOVE, PeekMessageW, SW_SHOW,
        SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE, SetForegroundWindow,
        SetWindowPos, ShowWindow, TranslateMessage, WM_QUIT,
    },
};

pub(super) struct Platform {
    window: Cell<HWND>,
}

impl Platform {
    pub fn new() -> Result<Self> {
        Ok(Self {
            window: Cell::new(ptr::null_mut()),
        })
    }

    pub fn configure(&self, title: &str, visible: bool) -> Result<()> {
        let window = find_window(title)?;
        self.window.set(window);
        // SAFETY: HWND is an existing top-level window in this helper process.
        unsafe {
            if visible {
                ShowWindow(window, SW_SHOW);
                SetForegroundWindow(window);
            } else {
                ensure!(
                    SetWindowPos(
                        window,
                        HWND_BOTTOM,
                        -30_000,
                        -30_000,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER
                    ) != 0,
                    "Moving thumbnail editor outside the desktop failed"
                );
                ShowWindow(window, SW_SHOWNOACTIVATE);
            }
        }
        Ok(())
    }

    pub fn pump(&self) -> Result<()> {
        // Dispatch without the plug-in mutex: controls can call back into VST3.
        let mut message = MSG::default();
        for _ in 0..256 {
            // SAFETY: `message` is valid output storage; this is the UI thread.
            if unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) } == 0 {
                break;
            }
            if message.message == WM_QUIT {
                break;
            }
            // SAFETY: PeekMessageW initialized the message.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }

    pub fn closed_by_user(&self) -> bool {
        let window = self.window.get();
        // SAFETY: IsWindow accepts stale handles and reports that they are gone.
        !window.is_null() && unsafe { IsWindow(window) } == 0
    }

    pub fn context_menu_position(&self, title: &str, x: i32, y: i32) -> Result<[f64; 2]> {
        use windows_sys::Win32::{Foundation::POINT, Graphics::Gdi::ClientToScreen};
        let window = find_window(title)?;
        let mut point = POINT { x, y };
        // SAFETY: this helper owns the HWND and point is writable storage.
        ensure!(
            unsafe { ClientToScreen(window, &mut point) } != 0,
            "Locating context menu failed"
        );
        Ok([f64::from(point.x), f64::from(point.y)])
    }

    pub fn capture(&self, title: &str, path: &Path) -> Result<(u32, u32)> {
        let window = find_window(title)?;
        let mut rect = RECT::default();
        // SAFETY: valid HWND and writable rectangle, confined to helper UI thread.
        ensure!(
            unsafe { GetClientRect(window, &mut rect) } != 0,
            "Reading native editor bounds failed"
        );
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        ensure!(
            width > 0
                && height > 0
                && width <= 8192
                && height <= 8192
                && i64::from(width) * i64::from(height) <= 16 * 1024 * 1024,
            "Native editor image is empty or exceeds capture limits"
        );
        let bitmap = CaptureBitmap::new(window, width, height)?;
        // PW_RENDERFULLCONTENT (2) asks modern composited controls to render their
        // actual client pixels. It works while the editor is positioned offscreen.
        // This synchronous call is bounded by the parent helper watchdog.
        ensure!(
            unsafe { PrintWindow(window, bitmap.memory, PW_CLIENTONLY | 2) } != 0,
            "This native editor cannot be captured with PrintWindow"
        );
        // CreateDIBSection requires GDI/CPU synchronization before reading its
        // bits. PrintWindow can return with same-thread drawing still batched.
        // SAFETY: flush this UI thread's GDI work while the selected DIB is alive.
        ensure!(
            unsafe { GdiFlush() } != 0,
            "Flushing native editor capture pixels failed"
        );
        let length = width as usize * height as usize * 4;
        // SAFETY: top-down 32bpp DIB allocated exactly width*height*4 bytes and
        // remains selected/alive until conversion completes.
        let bgra = unsafe { std::slice::from_raw_parts(bitmap.pixels.cast::<u8>(), length) };
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for pixel in bgra.as_chunks::<4>().0 {
            rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        let mut encoder = png::Encoder::new(File::create(path)?, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&rgb)?;
        Ok((width as u32, height as u32))
    }
}

struct WindowSearch<'a> {
    title: &'a str,
    window: HWND,
}

unsafe extern "system" fn visit_window(window: HWND, parameter: LPARAM) -> i32 {
    // SAFETY: EnumWindows synchronously calls with the live stack search object.
    let search = unsafe { &mut *(parameter as *mut WindowSearch<'_>) };
    let mut process = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut process);
    }
    if process != std::process::id() {
        return 1;
    }
    let length = unsafe { GetWindowTextLengthW(window) };
    if !(1..=16_384).contains(&length) {
        return 1;
    }
    let mut buffer = vec![0; length as usize + 1];
    let written = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
    if written > 0 && String::from_utf16_lossy(&buffer[..written as usize]) == search.title {
        search.window = window;
        return 0;
    }
    1
}

fn find_window(title: &str) -> Result<HWND> {
    let mut search = WindowSearch {
        title,
        window: ptr::null_mut(),
    };
    // SAFETY: callback and pointer remain valid throughout synchronous enumeration.
    unsafe {
        EnumWindows(
            Some(visit_window),
            &mut search as *mut WindowSearch<'_> as LPARAM,
        );
    }
    ensure!(
        !search.window.is_null(),
        "Cannot locate this helper's native editor window"
    );
    Ok(search.window)
}

struct CaptureBitmap {
    window: HWND,
    source: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut c_void,
}

impl CaptureBitmap {
    fn new(window: HWND, width: i32, height: i32) -> Result<Self> {
        // Initialize the guard first so every partial allocation is released.
        let mut bitmap = Self {
            window,
            source: ptr::null_mut(),
            memory: ptr::null_mut(),
            bitmap: ptr::null_mut(),
            previous: ptr::null_mut(),
            pixels: ptr::null_mut(),
        };
        // SAFETY: caller verified positive bounded dimensions and current-process HWND.
        unsafe {
            bitmap.source = GetDC(window);
            ensure!(!bitmap.source.is_null(), "GetDC for native editor failed");
            bitmap.memory = CreateCompatibleDC(bitmap.source);
            ensure!(
                !bitmap.memory.is_null(),
                "Creating editor capture context failed"
            );
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            bitmap.bitmap = CreateDIBSection(
                bitmap.source,
                &info,
                DIB_RGB_COLORS,
                &mut bitmap.pixels,
                ptr::null_mut(),
                0,
            );
            ensure!(
                !bitmap.bitmap.is_null() && !bitmap.pixels.is_null(),
                "Allocating editor capture bitmap failed"
            );
            bitmap.previous = SelectObject(bitmap.memory, bitmap.bitmap);
            ensure!(
                !bitmap.previous.is_null() && bitmap.previous as isize != -1,
                "Selecting editor capture bitmap failed"
            );
        }
        Ok(bitmap)
    }
}

impl Drop for CaptureBitmap {
    fn drop(&mut self) {
        // SAFETY: restore selection before deleting the owned GDI bitmap/context.
        unsafe {
            if !self.previous.is_null() && self.previous as isize != -1 {
                SelectObject(self.memory, self.previous);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.memory.is_null() {
                DeleteDC(self.memory);
            }
            if !self.source.is_null() {
                ReleaseDC(self.window, self.source);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use windows_sys::Win32::{
        Foundation::{LRESULT, WPARAM},
        Graphics::Gdi::{
            BLACK_BRUSH, BeginPaint, EndPaint, FillRect, GetStockObject, PAINTSTRUCT, WHITE_BRUSH,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, UnregisterClassW,
            WM_PAINT, WM_PRINT, WM_PRINTCLIENT, WNDCLASSW, WS_POPUP, WS_VISIBLE,
        },
    };

    static PRINT_MESSAGES: AtomicU32 = AtomicU32::new(0);

    unsafe fn paint(window: HWND, dc: HDC) {
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(window, &mut rect);
            FillRect(dc, &rect, GetStockObject(BLACK_BRUSH));
            rect.right /= 2;
            FillRect(dc, &rect, GetStockObject(WHITE_BRUSH));
        }
    }

    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_PRINT {
            PRINT_MESSAGES.fetch_or(1, Ordering::Relaxed);
        }
        if message == WM_PRINTCLIENT {
            PRINT_MESSAGES.fetch_or(2, Ordering::Relaxed);
            unsafe {
                paint(window, wparam as HDC);
            }
            return 0;
        }
        if message == WM_PAINT {
            let mut state = PAINTSTRUCT::default();
            unsafe {
                let dc = BeginPaint(window, &mut state);
                paint(window, dc);
                EndPaint(window, &state);
            }
            return 0;
        }
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    #[test]
    fn captures_real_win32_client_pixels_without_window_chrome() -> Result<()> {
        let platform = Platform::new()?;
        let title = format!("Rill capture test {} - VST3", std::process::id());
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        // SAFETY: valid class strings/procedure live until destruction below.
        let (module, window) = unsafe {
            let module = GetModuleHandleW(ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: module,
                lpszClassName: wide.as_ptr(),
                ..Default::default()
            };
            ensure!(
                RegisterClassW(&class) != 0,
                "Register capture test window failed"
            );
            let window = CreateWindowExW(
                0,
                wide.as_ptr(),
                wide.as_ptr(),
                WS_POPUP | WS_VISIBLE,
                -30_000,
                -30_000,
                24,
                16,
                ptr::null_mut(),
                ptr::null_mut(),
                module,
                ptr::null(),
            );
            ensure!(!window.is_null(), "Create capture test window failed");
            (module, window)
        };
        let result = (|| -> Result<()> {
            platform.configure(&title, false)?;
            platform.pump()?;
            let temporary = tempfile::tempdir()?;
            let image = temporary.path().join("window.png");
            assert_eq!(platform.capture(&title, &image)?, (24, 16));
            let mut reader =
                png::Decoder::new(std::io::BufReader::new(File::open(image)?)).read_info()?;
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut pixels)?;
            assert_eq!(
                &pixels[0..3],
                &[255, 255, 255],
                "Print message flags: {} (WM_PRINT=1, WM_PRINTCLIENT=2)",
                PRINT_MESSAGES.load(Ordering::Relaxed)
            );
            assert_eq!(&pixels[23 * 3..24 * 3], &[0, 0, 0]);
            Ok(())
        })();
        unsafe {
            DestroyWindow(window);
            UnregisterClassW(wide.as_ptr(), module);
        }
        assert!(platform.closed_by_user());
        result
    }
}
