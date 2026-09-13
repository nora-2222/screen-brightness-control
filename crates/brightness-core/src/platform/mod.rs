mod ddc;
mod ddc_worker;
mod enumerate;
mod hotplug;
mod id;
mod overlay;

use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::types::{ControlMethod, MonitorInfo, Rect};
use id::normalize_monitor_id;

pub use hotplug::MonitorHotplugWatcher;

pub struct MonitorManager {
    monitors: Vec<MonitorEntry>,
    overlay: overlay::OverlayRuntime,
    ddc: ddc_worker::DdcWorker,
    overlay_brightness: HashMap<String, u8>,
}

struct MonitorEntry {
    info: MonitorInfo,
    backend: BackendKind,
}

enum BackendKind {
    Ddc {
        description: String,
        display_name: String,
    },
    Overlay { bounds: Rect },
}

impl MonitorManager {
    /// Creates a manager and scans connected monitors.
    pub fn new() -> Result<Self> {
        let mut manager = Self {
            monitors: Vec::new(),
            overlay: overlay::OverlayRuntime::new()?,
            ddc: ddc_worker::DdcWorker::new(),
            overlay_brightness: HashMap::new(),
        };
        manager.refresh()?;
        Ok(manager)
    }

    /// Re-scans monitors after connect, disconnect, or layout changes.
    ///
    /// Clears cached DDC handles, removes overlay windows for disconnected
    /// displays, and reapplies overlay dimming for remaining overlay targets.
    pub fn refresh(&mut self) -> Result<()> {
        self.ddc.invalidate_cache()?;

        let ddc_devices = ddc::list_ddc_monitors()?;
        let displays = enumerate::list_displays()?;

        let ddc_display_names: std::collections::HashSet<String> = ddc_devices
            .iter()
            .filter(|device| !device.display_name.is_empty())
            .map(|device| device.display_name.clone())
            .collect();

        let mut monitors = Vec::new();

        for device in ddc_devices {
            let bounds = displays
                .iter()
                .find(|display| display.device_name == device.display_name)
                .map(|display| display.bounds.clone())
                .unwrap_or(Rect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                });

            monitors.push(MonitorEntry {
                info: MonitorInfo {
                    id: device.id.clone(),
                    name: device.name,
                    bounds,
                    method: ControlMethod::DdcCi,
                    min_brightness: 0,
                    max_brightness: 100,
                },
                backend: BackendKind::Ddc {
                    description: device.description,
                    display_name: device.display_name,
                },
            });
        }

        for display in displays {
            if ddc_display_names.contains(&display.device_name) {
                continue;
            }

            let id = overlay::overlay_id(&display.device_name);
            let bounds = display.bounds.clone();
            self.overlay_brightness.entry(id.clone()).or_insert(100);

            monitors.push(MonitorEntry {
                info: MonitorInfo {
                    id: id.clone(),
                    name: display.friendly_name,
                    bounds: bounds.clone(),
                    method: ControlMethod::Overlay,
                    min_brightness: 0,
                    max_brightness: 100,
                },
                backend: BackendKind::Overlay { bounds },
            });
        }

        self.monitors = monitors;
        self.sync_overlay_state()?;
        Ok(())
    }

    fn sync_overlay_state(&mut self) -> Result<()> {
        let active_overlay_ids: std::collections::HashSet<String> = self
            .monitors
            .iter()
            .filter(|monitor| monitor.info.method == ControlMethod::Overlay)
            .map(|monitor| monitor.info.id.clone())
            .collect();

        self.overlay_brightness
            .retain(|id, _| active_overlay_ids.contains(id));

        let overlay_states: Vec<(String, Rect, u8)> = self
            .monitors
            .iter()
            .filter(|monitor| monitor.info.method == ControlMethod::Overlay)
            .map(|monitor| {
                let brightness = *self
                    .overlay_brightness
                    .get(&monitor.info.id)
                    .unwrap_or(&100);
                (
                    monitor.info.id.clone(),
                    monitor.info.bounds.clone(),
                    brightness,
                )
            })
            .collect();

        self.overlay.sync_monitors(&overlay_states)
    }

    pub fn list_monitors(&self) -> Result<Vec<MonitorInfo>> {
        Ok(self.monitors.iter().map(|m| m.info.clone()).collect())
    }

    pub fn resolve_id(&self, query: &str) -> Result<String> {
        if self.monitors.iter().any(|monitor| monitor.info.id == query) {
            return Ok(query.to_string());
        }

        let normalized_query = normalize_monitor_id(query);
        if let Some(monitor) = self
            .monitors
            .iter()
            .find(|monitor| normalize_monitor_id(&monitor.info.id) == normalized_query)
        {
            return Ok(monitor.info.id.clone());
        }

        if let Ok(index) = query.parse::<usize>() {
            if let Some(monitor) = self.monitors.get(index) {
                return Ok(monitor.info.id.clone());
            }
        }

        let query_lower = query.to_ascii_lowercase();
        let mut matches = self.monitors.iter().filter(|monitor| {
            monitor.info.id.eq_ignore_ascii_case(query)
                || monitor.info.name.eq_ignore_ascii_case(query)
                || monitor.info.id.to_ascii_lowercase().ends_with(&query_lower)
        });

        if let Some(monitor) = matches.next() {
            if matches.next().is_none() {
                return Ok(monitor.info.id.clone());
            }
        }

        Err(Error::NotFound(query.to_string()))
    }

    pub fn get_brightness(&self, query: &str) -> Result<u8> {
        let id = self.resolve_id(query)?;
        let entry = self
            .find_monitor(&id)
            .ok_or_else(|| Error::NotFound(query.to_string()))?;

        match &entry.backend {
            BackendKind::Ddc {
                description,
                display_name,
            } => self.ddc.get_brightness(display_name, description),
            BackendKind::Overlay { .. } => Ok(*self.overlay_brightness.get(&id).unwrap_or(&100)),
        }
    }

    pub fn set_brightness(&mut self, query: &str, value: u8) -> Result<()> {
        self.set_brightness_inner(query, value, true)
    }

    /// Queues a brightness change without waiting for hardware to apply it.
    ///
    /// Use while dragging a slider. Call [`Self::set_brightness`] on release
    /// to ensure the final value is applied.
    pub fn set_brightness_async(&mut self, query: &str, value: u8) -> Result<()> {
        self.set_brightness_inner(query, value, false)
    }

    pub fn set_all_brightness_async(&mut self, value: u8) -> Result<()> {
        validate_brightness(value)?;
        let ids: Vec<String> = self.monitors.iter().map(|m| m.info.id.clone()).collect();
        for id in ids {
            self.set_brightness_async(&id, value)?;
        }
        Ok(())
    }

    /// Starts listening for monitor connect/disconnect events.
    pub fn watch_hotplug(&self) -> Result<MonitorHotplugWatcher> {
        MonitorHotplugWatcher::new()
    }

    pub fn adjust_brightness(&mut self, query: &str, delta: i8) -> Result<Option<u8>> {
        let id = self.resolve_id(query)?;
        let current = self.get_brightness(&id)?;
        let new_value = (i16::from(current) + i16::from(delta)).clamp(0, 100) as u8;
        if new_value == current {
            return Ok(None);
        }
        self.set_brightness_inner(&id, new_value, false)?;
        Ok(Some(new_value))
    }

    fn set_brightness_inner(&mut self, query: &str, value: u8, wait: bool) -> Result<()> {
        validate_brightness(value)?;
        let id = self.resolve_id(query)?;

        let entry = self
            .monitors
            .iter()
            .find(|monitor| monitor.info.id == id)
            .ok_or_else(|| Error::NotFound(query.to_string()))?;

        match &entry.backend {
            BackendKind::Ddc {
                description,
                display_name,
            } => {
                if wait {
                    self.ddc
                        .set_brightness_and_wait(display_name, description, value)?;
                } else {
                    self.ddc
                        .set_brightness(display_name, description, value)?;
                }
                return Ok(());
            }
            BackendKind::Overlay { bounds } => {
                let bounds = bounds.clone();
                self.overlay.set_brightness(&id, &bounds, value)?;
                self.overlay_brightness.insert(id, value);
                let _ = wait;
                return Ok(());
            }
        }
    }

    pub fn set_all_brightness(&mut self, value: u8) -> Result<()> {
        validate_brightness(value)?;
        let ids: Vec<String> = self.monitors.iter().map(|m| m.info.id.clone()).collect();
        for id in ids {
            self.set_brightness(&id, value)?;
        }
        Ok(())
    }

    pub fn has_overlay_targets(&self) -> bool {
        self.monitors
            .iter()
            .any(|monitor| monitor.info.method == ControlMethod::Overlay)
    }

    pub fn uses_overlay(&self, query: &str) -> bool {
        self.resolve_id(query)
            .ok()
            .and_then(|id| {
                self.monitors
                    .iter()
                    .find(|monitor| monitor.info.id == id)
                    .map(|monitor| monitor.info.method == ControlMethod::Overlay)
            })
            .unwrap_or(false)
    }

    pub fn wait_for_shutdown(&self) {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    fn find_monitor(&self, id: &str) -> Option<&MonitorEntry> {
        self.monitors.iter().find(|m| m.info.id == id)
    }
}

impl Drop for MonitorManager {
    fn drop(&mut self) {
        self.ddc.shutdown();
        self.overlay.shutdown();
    }
}

fn validate_brightness(value: u8) -> Result<()> {
    if value > 100 {
        return Err(Error::InvalidBrightness(value));
    }
    Ok(())
}
