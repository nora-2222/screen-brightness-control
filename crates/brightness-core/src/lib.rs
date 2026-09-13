//! Windows monitor brightness control.
//!
//! Each display uses DDC/CI when the hardware supports it. Otherwise the library
//! falls back to a software overlay that dims the screen without changing
//! backlight power.
//!
//! # Quick start
//!
//! ```no_run
//! use brightness_core::MonitorManager;
//!
//! fn main() -> brightness_core::Result<()> {
//!     let mut manager = MonitorManager::new()?;
//!     for monitor in manager.list_monitors()? {
//!         println!("{} ({})", monitor.name, monitor.method.as_str());
//!     }
//!     manager.set_brightness("0", 80)?;
//!     Ok(())
//! }
//! ```
//!
//! # Slider drag
//!
//! Call [`MonitorManager::set_brightness_async`] while the user drags a slider,
//! then call [`MonitorManager::set_brightness`] when the slider is released.
//!
//! # Hot-plug
//!
//! ```no_run
//! use brightness_core::MonitorManager;
//!
//! fn main() -> brightness_core::Result<()> {
//!     let mut manager = MonitorManager::new()?;
//!     let watcher = manager.watch_hotplug()?;
//!
//!     loop {
//!         if watcher.try_recv().is_some() {
//!             manager.refresh()?;
//!             // Rebuild monitor list UI here.
//!         }
//!         std::thread::sleep(std::time::Duration::from_millis(200));
//!     }
//! }
//! ```
//!
//! # Overlay lifetime
//!
//! Overlay dimming exists only while the host process is running. When the
//! process exits, overlay windows are removed automatically.

mod error;
mod types;

#[cfg(windows)]
mod platform;

pub use error::{Error, Result};
pub use types::{ControlMethod, MonitorInfo, Rect};

#[cfg(windows)]
pub use platform::MonitorHotplugWatcher;

#[cfg(windows)]
pub use platform::MonitorManager;

#[cfg(not(windows))]
pub struct MonitorHotplugWatcher;

#[cfg(not(windows))]
impl MonitorHotplugWatcher {
    pub fn new() -> Result<Self> {
        Err(Error::PlatformUnsupported)
    }

    pub fn try_recv(&self) -> Option<()> {
        None
    }

    pub fn recv_timeout(
        &self,
        _timeout: std::time::Duration,
    ) -> std::result::Result<(), std::sync::mpsc::RecvTimeoutError> {
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    }
}

#[cfg(not(windows))]
pub struct MonitorManager;

#[cfg(not(windows))]
impl MonitorManager {
    pub fn new() -> Result<Self> {
        Err(Error::PlatformUnsupported)
    }

    pub fn list_monitors(&self) -> Result<Vec<MonitorInfo>> {
        Err(Error::PlatformUnsupported)
    }

    pub fn get_brightness(&self, _id: &str) -> Result<u8> {
        Err(Error::PlatformUnsupported)
    }

    pub fn set_brightness(&mut self, _id: &str, _value: u8) -> Result<()> {
        Err(Error::PlatformUnsupported)
    }

    pub fn set_brightness_async(&mut self, _id: &str, _value: u8) -> Result<()> {
        Err(Error::PlatformUnsupported)
    }

    pub fn set_all_brightness(&mut self, _value: u8) -> Result<()> {
        Err(Error::PlatformUnsupported)
    }

    pub fn set_all_brightness_async(&mut self, _value: u8) -> Result<()> {
        Err(Error::PlatformUnsupported)
    }

    pub fn refresh(&mut self) -> Result<()> {
        Err(Error::PlatformUnsupported)
    }

    pub fn resolve_id(&self, query: &str) -> Result<String> {
        Err(Error::NotFound(query.to_string()))
    }

    pub fn adjust_brightness(&mut self, _query: &str, _delta: i8) -> Result<Option<u8>> {
        Err(Error::PlatformUnsupported)
    }

    pub fn watch_hotplug(&self) -> Result<MonitorHotplugWatcher> {
        Err(Error::PlatformUnsupported)
    }

    pub fn has_overlay_targets(&self) -> bool {
        false
    }

    pub fn uses_overlay(&self, _id: &str) -> bool {
        false
    }

    pub fn wait_for_shutdown(&self) {}
}
