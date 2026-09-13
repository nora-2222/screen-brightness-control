use std::io::{self, Write};

use brightness_core::Result;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, MOD_CONTROL, VK_NEXT, VK_PRIOR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    TranslateMessage, MSG, WM_DESTROY, WM_HOTKEY, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP,
};
use windows::core::PCWSTR;

const CLASS_NAME: &str = "BrightnessPrototypeHotkeyWindow";
const HOTKEY_BRIGHTER: i32 = 1;
const HOTKEY_DARKER: i32 = 2;

pub fn run_hotkey_loop(
    step: u8,
    mut on_adjust: impl FnMut(i8) -> Result<String>,
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

            match on_adjust(delta) {
                Ok(status) => {
                    print!("\r{status}   ");
                    let _ = io::stdout().flush();
                }
                Err(err) => eprintln!("\nerror: {err}"),
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

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
