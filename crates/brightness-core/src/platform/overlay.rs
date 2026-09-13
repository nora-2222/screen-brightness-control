use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetStockObject,
    ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    BLACK_BRUSH, BLENDFUNCTION, DIB_RGB_COLORS, HBRUSH, HDC,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, SetWindowPos, ShowWindow, TranslateMessage,
    UpdateLayeredWindow, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, MSG, PM_REMOVE, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CREATE, WM_DESTROY, WM_USER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::PCWSTR;

use crate::error::{Error, Result};
use crate::types::Rect;

const CLASS_NAME: &str = "BrightnessControlOverlay";
const WM_SET_BRIGHTNESS: u32 = WM_USER + 1;
const THREAD_SLEEP_MS: u64 = 16;

pub fn overlay_id(device_name: &str) -> String {
    format!("overlay:{}", super::id::display_slug(device_name))
}

enum OverlayRequest {
    SetBrightness {
        id: String,
        bounds: Rect,
        brightness: u8,
    },
    SyncMonitors {
        monitors: Vec<OverlayMonitorState>,
    },
}

#[derive(Clone)]
struct OverlayMonitorState {
    id: String,
    bounds: Rect,
    brightness: u8,
}

enum OverlayCommand {
    Request {
        request: OverlayRequest,
        reply: Sender<Result<()>>,
    },
    Shutdown,
}

pub struct OverlayRuntime {
    tx: Sender<OverlayCommand>,
    thread: Option<JoinHandle<()>>,
}

impl OverlayRuntime {
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("brightness-overlay".into())
            .spawn(move || overlay_thread_main(rx))
            .map_err(|e| Error::Overlay(format!("failed to spawn overlay thread: {e}")))?;

        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }

    pub fn set_brightness(&self, id: &str, bounds: &Rect, brightness: u8) -> Result<()> {
        self.send(OverlayRequest::SetBrightness {
            id: id.to_string(),
            bounds: bounds.clone(),
            brightness,
        })
    }

    pub fn sync_monitors(&self, monitors: &[(String, Rect, u8)]) -> Result<()> {
        let monitors = monitors
            .iter()
            .map(|(id, bounds, brightness)| OverlayMonitorState {
                id: id.clone(),
                bounds: bounds.clone(),
                brightness: *brightness,
            })
            .collect();

        self.send(OverlayRequest::SyncMonitors { monitors })
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(OverlayCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    fn send(&self, request: OverlayRequest) -> Result<()> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(OverlayCommand::Request {
                request,
                reply: reply_tx,
            })
            .map_err(|e| Error::Overlay(format!("overlay channel closed: {e}")))?;

        match reply_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => result,
            Err(_) => Err(Error::Overlay("overlay operation timed out".into())),
        }
    }
}

struct OverlayState {
    class_registered: bool,
    windows: HashMap<String, HWND>,
    running: bool,
}

fn overlay_thread_main(rx: Receiver<OverlayCommand>) {
    let mut state = OverlayState {
        class_registered: false,
        windows: HashMap::new(),
        running: true,
    };

    while state.running {
        while let Ok(command) = rx.try_recv() {
            if !handle_command(&mut state, command) {
                state.running = false;
                break;
            }
        }

        pump_window_messages();

        match rx.recv_timeout(Duration::from_millis(THREAD_SLEEP_MS)) {
            Ok(command) => {
                if !handle_command(&mut state, command) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn pump_window_messages() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn handle_command(state: &mut OverlayState, command: OverlayCommand) -> bool {
    match command {
        OverlayCommand::Request { request, reply } => {
            let result = match request {
                OverlayRequest::SetBrightness {
                    id,
                    bounds,
                    brightness,
                } => set_window_brightness(state, &id, &bounds, brightness),
                OverlayRequest::SyncMonitors { monitors } => sync_overlay_monitors(state, &monitors),
            };
            let _ = reply.send(result);
        }
        OverlayCommand::Shutdown => {
            for hwnd in state.windows.values() {
                unsafe {
                    let _ = PostMessageW(Some(*hwnd), WM_DESTROY, WPARAM(0), LPARAM(0));
                }
            }
            unsafe {
                PostQuitMessage(0);
            }
            return false;
        }
    }
    true
}

fn ensure_window(state: &mut OverlayState, id: &str, bounds: &Rect, brightness: u8) -> Result<()> {
    register_class(state)?;

    if state.windows.contains_key(id) {
        return set_window_brightness(state, id, bounds, brightness);
    }

    let hwnd = create_overlay_window(bounds)?;
    state.windows.insert(id.to_string(), hwnd);
    apply_brightness(hwnd, bounds, brightness)?;
    Ok(())
}

fn set_window_brightness(
    state: &mut OverlayState,
    id: &str,
    bounds: &Rect,
    brightness: u8,
) -> Result<()> {
    let hwnd = match state.windows.get(id) {
        Some(hwnd) => *hwnd,
        None => {
            ensure_window(state, id, bounds, brightness)?;
            return Ok(());
        }
    };

    reposition_window(hwnd, bounds)?;
    apply_brightness(hwnd, bounds, brightness)
}

fn sync_overlay_monitors(state: &mut OverlayState, monitors: &[OverlayMonitorState]) -> Result<()> {
    let keep: std::collections::HashSet<&str> = monitors.iter().map(|m| m.id.as_str()).collect();

    let stale: Vec<String> = state
        .windows
        .keys()
        .filter(|id| !keep.contains(id.as_str()))
        .cloned()
        .collect();

    for id in stale {
        remove_overlay_window(state, &id);
    }

    for monitor in monitors {
        set_window_brightness(state, &monitor.id, &monitor.bounds, monitor.brightness)?;
    }

    Ok(())
}

fn remove_overlay_window(state: &mut OverlayState, id: &str) {
    if let Some(hwnd) = state.windows.remove(id) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn register_class(state: &mut OverlayState) -> Result<()> {
    if state.class_registered {
        return Ok(());
    }

    let class_name = wide_string(CLASS_NAME);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: unsafe { GetModuleHandleW(None).unwrap_or_default().into() },
        lpszClassName: PCWSTR(class_name.as_ptr()),
        hbrBackground: HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0),
        style: CS_HREDRAW | CS_VREDRAW,
        ..Default::default()
    };

    unsafe {
        RegisterClassW(&wc);
    }

    state.class_registered = true;
    Ok(())
}

fn create_overlay_window(bounds: &Rect) -> Result<HWND> {
    let class_name = wide_string(CLASS_NAME);
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(
                WS_EX_LAYERED.0
                    | WS_EX_TRANSPARENT.0
                    | WS_EX_TOPMOST.0
                    | WS_EX_NOACTIVATE.0
                    | WS_EX_TOOLWINDOW.0,
            ),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            WINDOW_STYLE(WS_POPUP.0),
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            None,
            None,
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        )
    }
    .map_err(|e| Error::Overlay(format!("CreateWindowExW failed: {e}")))?;

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }

    Ok(hwnd)
}

fn reposition_window(hwnd: HWND, bounds: &Rect) -> Result<()> {
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .map_err(|e| Error::Overlay(format!("SetWindowPos failed: {e}")))?;
    }

    Ok(())
}

fn apply_brightness(hwnd: HWND, bounds: &Rect, brightness: u8) -> Result<()> {
    let alpha = brightness_to_alpha(brightness);
    let width = bounds.width.max(1);
    let height = bounds.height.max(1);

    if alpha == 0 {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        return Ok(());
    }

    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err(Error::Overlay("GetDC failed".into()));
        }

        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        if mem_dc.is_invalid() {
            ReleaseDC(None, screen_dc);
            return Err(Error::Overlay("CreateCompatibleDC failed".into()));
        }

        let result = render_overlay(hwnd, screen_dc, mem_dc, bounds, width, height, alpha);

        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);

        result?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }

    Ok(())
}

unsafe fn render_overlay(
    hwnd: HWND,
    screen_dc: HDC,
    mem_dc: HDC,
    bounds: &Rect,
    width: i32,
    height: i32,
    alpha: u8,
) -> Result<()> {
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    unsafe {
        let bitmap = CreateDIBSection(
            Some(mem_dc),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        )
        .map_err(|e| Error::Overlay(format!("CreateDIBSection failed: {e}")))?;

        if bits.is_null() {
            let _ = DeleteObject(bitmap.into());
            return Err(Error::Overlay("CreateDIBSection returned null bits".into()));
        }

        let pixel_count = (width as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| Error::Overlay("overlay bitmap size overflow".into()))?;
        let buffer =
            std::slice::from_raw_parts_mut(bits as *mut u8, pixel_count.checked_mul(4).unwrap());
        for pixel in buffer.chunks_exact_mut(4) {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            pixel[3] = alpha;
        }

        let _ = SelectObject(mem_dc, bitmap.into());

        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        UpdateLayeredWindow(
            hwnd,
            Some(screen_dc),
            Some(&POINT {
                x: bounds.x,
                y: bounds.y,
            }),
            Some(&SIZE {
                cx: width,
                cy: height,
            }),
            Some(mem_dc),
            Some(&POINT { x: 0, y: 0 }),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
        .map_err(|e| Error::Overlay(format!("UpdateLayeredWindow failed: {e}")))?;

        let _ = DeleteObject(bitmap.into());
    }

    Ok(())
}

fn brightness_to_alpha(brightness: u8) -> u8 {
    let clamped = brightness.min(100);
    (((100 - clamped) as u16 * 255) / 100) as u8
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => LRESULT(0),
        WM_SET_BRIGHTNESS => LRESULT(0),
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::brightness_to_alpha;

    #[test]
    fn alpha_mapping() {
        assert_eq!(brightness_to_alpha(100), 0);
        assert_eq!(brightness_to_alpha(0), 255);
        assert_eq!(brightness_to_alpha(50), 127);
    }
}
