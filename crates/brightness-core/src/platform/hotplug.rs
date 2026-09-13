use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, mpsc::{self, Receiver, RecvTimeoutError, Sender}};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW, PostQuitMessage,
    RegisterClassW, TranslateMessage, MSG, WM_CLOSE, WM_DEVICECHANGE, WM_DISPLAYCHANGE,
    WM_DESTROY, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::PCWSTR;

use crate::error::{Error, Result};

const CLASS_NAME: &str = "BrightnessHotplugWatcher";
const DBT_DEVNODES_CHANGED: WPARAM = WPARAM(0x0007);
const DEBOUNCE: Duration = Duration::from_millis(300);

struct HotplugState {
    event_tx: Sender<()>,
    last_signal: Option<Instant>,
}

static HOTPLUG_STATE: Mutex<Option<HotplugState>> = Mutex::new(None);
static WATCHER_HWND: AtomicIsize = AtomicIsize::new(0);

/// Notifies embedders when display topology may have changed.
///
/// Poll with [`Self::try_recv`] from a UI timer/tick, then call
/// [`crate::MonitorManager::refresh`].
pub struct MonitorHotplugWatcher {
    rx: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl MonitorHotplugWatcher {
    pub fn new() -> Result<Self> {
        let (event_tx, event_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread = thread::Builder::new()
            .name("brightness-hotplug".into())
            .spawn(move || watcher_thread_main(event_tx, ready_tx))
            .map_err(|e| Error::Internal(format!("failed to spawn hotplug watcher: {e}")))?;

        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| Error::Internal("hotplug watcher startup timed out".into()))?;

        Ok(Self {
            rx: event_rx,
            thread: Some(thread),
        })
    }

    /// Returns `Some(())` when monitors may have been connected or disconnected.
    pub fn try_recv(&self) -> Option<()> {
        self.rx.try_recv().ok()
    }

    /// Blocks until a change notification or timeout.
    pub fn recv_timeout(&self, timeout: Duration) -> std::result::Result<(), RecvTimeoutError> {
        self.rx.recv_timeout(timeout).map(|_| ())
    }
}

impl Drop for MonitorHotplugWatcher {
    fn drop(&mut self) {
        let hwnd = WATCHER_HWND.load(Ordering::Acquire);
        if hwnd != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn watcher_thread_main(event_tx: Sender<()>, ready_tx: Sender<()>) {
    if let Err(err) = run_message_loop(&event_tx, ready_tx) {
        eprintln!("brightness-core: hotplug watcher failed: {err}");
    }
}

fn run_message_loop(event_tx: &Sender<()>, ready_tx: Sender<()>) -> Result<()> {
    let class_name = wide_string(CLASS_NAME);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: unsafe { GetModuleHandleW(None).unwrap_or_default().into() },
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };

    unsafe {
        RegisterClassW(&wc);
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        )
    }
    .map_err(|e| Error::Internal(format!("CreateWindowExW failed: {e}")))?;

    WATCHER_HWND.store(hwnd.0 as isize, Ordering::Release);
    HOTPLUG_STATE.lock().unwrap().replace(HotplugState {
        event_tx: event_tx.clone(),
        last_signal: None,
    });

    let _ = ready_tx.send(());

    let mut msg = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if result.0 == 0 || result.0 == -1 {
            break;
        }

        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    WATCHER_HWND.store(0, Ordering::Release);
    HOTPLUG_STATE.lock().unwrap().take();
    Ok(())
}

fn signal_change(state: &mut HotplugState) {
    let now = Instant::now();
    if let Some(last) = state.last_signal {
        if now.duration_since(last) < DEBOUNCE {
            return;
        }
    }

    state.last_signal = Some(now);
    let _ = state.event_tx.send(());
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DISPLAYCHANGE => {
            if let Ok(mut guard) = HOTPLUG_STATE.lock() {
                if let Some(state) = guard.as_mut() {
                    signal_change(state);
                }
            }
            LRESULT(0)
        }
        WM_DEVICECHANGE if wparam == DBT_DEVNODES_CHANGED => {
            if let Ok(mut guard) = HOTPLUG_STATE.lock() {
                if let Some(state) = guard.as_mut() {
                    signal_change(state);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        WM_CLOSE => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
