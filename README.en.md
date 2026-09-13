# screen brightness control

[日本語](README.md)

Windows monitor brightness library written in Rust.

The library uses DDC/CI when the monitor and connection support it. When DDC/CI
is not available, it falls back to a software overlay that reduces glare without
changing backlight power.

Overlay dimming exists only while the host process is running. When the process
exits, overlay windows are removed automatically.

## Workspace layout

```
crates/
  brightness-core/   Library
  brightness-cli/    Debug console
```

## Build

```bash
cargo build
```

## CLI

```bash
cargo run -p brightness-cli -- list
cargo run -p brightness-cli -- get 0
cargo run -p brightness-cli -- set 0 50
cargo run -p brightness-cli -- watch
```

The `watch` command listens for monitor connect and disconnect events and
refreshes the monitor list automatically.

## Library usage

Add `brightness-core` to your application:

```toml
[dependencies]
brightness-core = { path = "../crates/brightness-core" }
```

Basic example:

```rust
use brightness_core::MonitorManager;

fn main() -> brightness_core::Result<()> {
    let mut manager = MonitorManager::new()?;

    for monitor in manager.list_monitors()? {
        println!("{} [{}]", monitor.name, monitor.id);
    }

    manager.set_brightness("0", 80)?;
    Ok(())
}
```

### Monitor IDs

Each monitor has a stable string id such as `ddc:display1` or
`overlay:display1`. You can also pass a list index or a partial name to most
API methods.

### Slider drag

Use async writes while dragging and a sync write on release:

```rust
// While dragging
manager.set_brightness_async(&monitor_id, value)?;

// On release
manager.set_brightness(&monitor_id, value)?;
```

### Hot-plug

Poll the watcher from a UI timer or background thread:

```rust
let watcher = manager.watch_hotplug()?;

if watcher.try_recv().is_some() {
    manager.refresh()?;
    // Rebuild combobox and sliders here.
}
```

`refresh` clears cached DDC handles, removes overlay windows for disconnected
displays, and reapplies overlay dimming for remaining overlay targets.

### Overlay applications

If your app uses overlay targets and the user sets brightness below 100%, keep
the process alive (for example as a tray application). DDC targets do not need
this.

## API summary

| Method | Purpose |
|--------|---------|
| `new` | Create manager and scan monitors |
| `list_monitors` | List monitors and control methods |
| `get_brightness` | Read brightness 0-100 |
| `set_brightness` | Set brightness and wait for DDC apply |
| `set_brightness_async` | Queue brightness change for slider drag |
| `adjust_brightness` | Change brightness by delta |
| `refresh` | Re-scan after hot-plug or layout change |
| `watch_hotplug` | Receive connect and disconnect notifications |
| `resolve_id` | Resolve index or name to monitor id |

## Platform support

Windows only. Other platforms return `Error::PlatformUnsupported`.

## DDC/CI notes

DDC/CI depends on the monitor, cable, and driver. DisplayPort and DVI usually
work better than HDMI. Some monitors require DDC/CI to be enabled in the OSD menu.
