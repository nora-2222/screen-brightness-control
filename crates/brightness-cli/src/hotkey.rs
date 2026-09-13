#[cfg(windows)]
use brightness_core::Result;
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_CONTROL, VK_NEXT, VK_PRIOR,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    TranslateMessage, MSG, WM_DESTROY, WM_HOTKEY, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP,
};
#[cfg(windows)]
use windows::core::PCWSTR;

#[cfg(windows)]
const CLASS_NAME: &str = "BrightnessHotkeyWindow";
#[cfg(windows)]
const HOTKEY_BRIGHTER: i32 = 1;
#[cfg(windows)]
const HOTKEY_DARKER: i32 = 2;

#[cfg(windows)]
pub fn run_hotkey_loop(
    target_id: &str,
    step: u8,
    mut adjust: impl FnMut(i8) -> Result<Option<u8>>,
) -> Result<()> {
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
    .map_err(|e| brightness_core::Error::Internal(format!("CreateWindowExW failed: {e}")))?;

    unsafe {
        RegisterHotKey(
            Some(hwnd),
            HOTKEY_BRIGHTER,
            MOD_CONTROL,
            VK_PRIOR.0 as u32,
        )
        .map_err(|e| {
            brightness_core::Error::Internal(format!("RegisterHotKey Ctrl+PgUp failed: {e}"))
        })?;

        RegisterHotKey(
            Some(hwnd),
            HOTKEY_DARKER,
            MOD_CONTROL,
            VK_NEXT.0 as u32,
        )
        .map_err(|e| {
            brightness_core::Error::Internal(format!("RegisterHotKey Ctrl+PgDn failed: {e}"))
        })?;
    }

    let step_i8 = i8::try_from(step).unwrap_or(5);

    println!("Hotkeys active for {target_id} (step: {step}%).");
    println!("  Ctrl+PgUp   brighter");
    println!("  Ctrl+PgDn   darker");
    println!("  Ctrl+C      exit");

    let mut msg = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if result.0 == 0 || result.0 == -1 {
            break;
        }

        if msg.message == WM_HOTKEY {
            let delta = match msg.wParam.0 as i32 {
                HOTKEY_BRIGHTER => step_i8,
                HOTKEY_DARKER => -step_i8,
                _ => {
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    continue;
                }
            };

            match adjust(delta) {
                Ok(Some(value)) => println!("brightness: {value}%"),
                Ok(None) => {}
                Err(err) => eprintln!("error: {err}"),
            }
        }

        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_BRIGHTER);
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_DARKER);
    }

    Ok(())
}

#[cfg(windows)]
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(windows)]
fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(not(windows))]
pub fn run_hotkey_loop(
    _target_id: &str,
    _step: u8,
    _adjust: impl FnMut(i8) -> brightness_core::Result<Option<u8>>,
) -> brightness_core::Result<()> {
    Err(brightness_core::Error::PlatformUnsupported)
}
