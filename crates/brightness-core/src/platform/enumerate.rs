use std::mem;
use std::ptr;
use std::thread;
use std::time::Duration;

use windows::core::BOOL;
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitor, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, PHYSICAL_MONITOR,
    SetVCPFeature,
};
use windows::Win32::Foundation::{HANDLE, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};

use crate::error::{Error, Result};
use crate::types::Rect;

pub struct DisplayInfo {
    pub device_name: String,
    pub friendly_name: String,
    pub bounds: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DdcMonitorInfo {
    pub display_name: String,
    pub description: String,
}

pub fn list_displays() -> Result<Vec<DisplayInfo>> {
    let mut displays = Vec::new();
    let context = EnumContext {
        displays: &mut displays,
    };

    unsafe {
        let ok = EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitor_proc),
            LPARAM(ptr::from_ref(&context) as isize),
        );
        if !ok.as_bool() {
            return Err(Error::Internal("EnumDisplayMonitors failed".into()));
        }
    }

    Ok(displays)
}

pub fn probe_ddc_monitors() -> Result<Vec<DdcMonitorInfo>> {
    let mut monitors = Vec::new();
    let context = DdcProbeContext {
        monitors: &mut monitors,
    };

    unsafe {
        let ok = EnumDisplayMonitors(
            None,
            None,
            Some(ddc_probe_proc),
            LPARAM(ptr::from_ref(&context) as isize),
        );
        if !ok.as_bool() {
            return Err(Error::Internal("EnumDisplayMonitors failed".into()));
        }
    }

    Ok(monitors)
}

struct EnumContext<'a> {
    displays: &'a mut Vec<DisplayInfo>,
}

struct DdcProbeContext<'a> {
    monitors: &'a mut Vec<DdcMonitorInfo>,
}

unsafe extern "system" fn enum_monitor_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let context = unsafe { &mut *(lparam.0 as *mut EnumContext<'_>) };
    match monitor_info(hmonitor) {
        Ok(info) => context.displays.push(info),
        Err(err) => eprintln!("brightness-core: failed to read monitor info: {err}"),
    }
    TRUE
}

unsafe extern "system" fn ddc_probe_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let context = unsafe { &mut *(lparam.0 as *mut DdcProbeContext<'_>) };
    if let Ok(device_name) = display_device_name(hmonitor) {
        if let Err(err) = probe_physical_monitors(hmonitor, &device_name, context.monitors) {
            eprintln!("brightness-core: failed to probe DDC on {device_name}: {err}");
        }
    }
    TRUE
}

fn probe_physical_monitors(
    hmonitor: HMONITOR,
    device_name: &str,
    monitors: &mut Vec<DdcMonitorInfo>,
) -> Result<()> {
    let mut count = 0;
    unsafe {
        GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count)
            .map_err(|e| Error::Internal(format!("GetNumberOfPhysicalMonitorsFromHMONITOR failed: {e}")))?;
    }

    if count == 0 {
        return Ok(());
    }

    let mut physical_monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
    unsafe {
        GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut physical_monitors)
            .map_err(|e| Error::Internal(format!("GetPhysicalMonitorsFromHMONITOR failed: {e}")))?;
    }

    for monitor in physical_monitors {
        let description = {
            let raw = monitor.szPhysicalMonitorDescription;
            wide_buffer_to_string(&raw)
        };

        let handle = monitor.hPhysicalMonitor;
        if handle.is_invalid() {
            continue;
        }

        if supports_brightness_control(handle) {
            monitors.push(DdcMonitorInfo {
                display_name: device_name.to_string(),
                description,
            });
        }

        unsafe {
            let _ = DestroyPhysicalMonitor(handle);
        }
    }

    Ok(())
}

fn supports_brightness_control(handle: HANDLE) -> bool {
    let mut min = 0;
    let mut current = 0;
    let mut max = 0;

    unsafe {
        if GetMonitorBrightness(handle, &mut min, &mut current, &mut max) != 0 {
            return max > 0;
        }
    }

    let mut current = 0;
    let mut max = 0;
    unsafe {
        GetVCPFeatureAndVCPFeatureReply(handle, 0x10, None, &mut current, Some(&mut max)) != 0
    }
}

pub fn acquire_ddc_handle(display_name: &str, description: &str) -> Result<(HANDLE, u32)> {
    let mut hmonitors = Vec::new();

    unsafe {
        let ok = EnumDisplayMonitors(
            None,
            None,
            Some(collect_hmonitor_proc),
            LPARAM(ptr::from_ref(&mut hmonitors) as isize),
        );
        if !ok.as_bool() {
            return Err(Error::Internal("EnumDisplayMonitors failed".into()));
        }
    }

    for hmonitor in hmonitors {
        if display_device_name(hmonitor)? != display_name {
            continue;
        }

        let mut count = 0;
        unsafe {
            GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count)
                .map_err(|e| Error::Internal(format!("GetNumberOfPhysicalMonitorsFromHMONITOR failed: {e}")))?;
        }

        if count == 0 {
            continue;
        }

        let mut physical_monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
        unsafe {
            GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut physical_monitors)
                .map_err(|e| Error::Internal(format!("GetPhysicalMonitorsFromHMONITOR failed: {e}")))?;
        }

        for monitor in physical_monitors {
            let monitor_description = {
                let raw = monitor.szPhysicalMonitorDescription;
                wide_buffer_to_string(&raw)
            };

            let handle = monitor.hPhysicalMonitor;
            if handle.is_invalid() {
                continue;
            }

            if monitor_description == description {
                let (_, max) = read_brightness(handle)?;
                return Ok((handle, max));
            }

            unsafe {
                let _ = DestroyPhysicalMonitor(handle);
            }
        }
    }

    Err(Error::NotFound(format!("{display_name}#{description}")))
}

pub fn release_ddc_handle(handle: HANDLE) {
    if !handle.is_invalid() {
        unsafe {
            let _ = DestroyPhysicalMonitor(handle);
        }
    }
}

unsafe extern "system" fn collect_hmonitor_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let hmonitors = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
    hmonitors.push(hmonitor);
    TRUE
}

fn display_device_name(hmonitor: HMONITOR) -> Result<String> {
    let mut info = MONITORINFOEXW {
        monitorInfo: windows::Win32::Graphics::Gdi::MONITORINFO {
            cbSize: mem::size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };

    unsafe {
        let ok = GetMonitorInfoW(hmonitor, &mut info.monitorInfo);
        if !ok.as_bool() {
            return Err(Error::Internal("GetMonitorInfoW failed".into()));
        }
    }

    Ok(wide_buffer_to_string(&info.szDevice))
}

fn monitor_info(hmonitor: HMONITOR) -> Result<DisplayInfo> {
    let device_name = display_device_name(hmonitor)?;
    let mut info = MONITORINFOEXW {
        monitorInfo: windows::Win32::Graphics::Gdi::MONITORINFO {
            cbSize: mem::size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };

    unsafe {
        let ok = GetMonitorInfoW(hmonitor, &mut info.monitorInfo);
        if !ok.as_bool() {
            return Err(Error::Internal("GetMonitorInfoW failed".into()));
        }
    }

    let rect = info.monitorInfo.rcMonitor;
    let friendly_name = friendly_name_from_device(&device_name);

    Ok(DisplayInfo {
        device_name,
        friendly_name,
        bounds: Rect {
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
        },
    })
}

fn friendly_name_from_device(device_name: &str) -> String {
    let trimmed = device_name.trim_end_matches('\0');
    if trimmed.is_empty() {
        "Unknown display".to_string()
    } else {
        trimmed.to_string()
    }
}

fn wide_buffer_to_string(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

const READ_RETRY_COUNT: u8 = 5;
const READ_RETRY_DELAY: Duration = Duration::from_millis(100);

pub fn read_brightness(handle: HANDLE) -> Result<(u32, u32)> {
    let mut last_error = Error::Ddc("brightness query failed".into());

    for attempt in 0..READ_RETRY_COUNT {
        match read_brightness_once(handle) {
            Ok(value) => return Ok(value),
            Err(err) => {
                last_error = err;
                if attempt + 1 < READ_RETRY_COUNT {
                    thread::sleep(READ_RETRY_DELAY);
                }
            }
        }
    }

    Err(last_error)
}

fn read_brightness_once(handle: HANDLE) -> Result<(u32, u32)> {
    let mut min = 0;
    let mut current = 0;
    let mut max = 0;

    unsafe {
        if GetMonitorBrightness(handle, &mut min, &mut current, &mut max) != 0 {
            return Ok((current, max));
        }
    }

    let mut current = 0;
    let mut max = 0;
    unsafe {
        if GetVCPFeatureAndVCPFeatureReply(handle, 0x10, None, &mut current, Some(&mut max)) == 0 {
            return Err(Error::Ddc("brightness query failed".into()));
        }
    }

    Ok((current, max))
}

pub fn write_brightness(handle: HANDLE, raw_value: u32) -> Result<()> {
    unsafe {
        if SetVCPFeature(handle, 0x10, raw_value) == 0 {
            return Err(Error::Ddc("SetVCPFeature(0x10) failed".into()));
        }
    }
    Ok(())
}
