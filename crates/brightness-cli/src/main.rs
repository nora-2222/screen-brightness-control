mod hotkey;

use std::cell::RefCell;
use std::process;

use brightness_core::{ControlMethod, Error, MonitorManager};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "brightness", about = "Test console for brightness-core")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List connected monitors and their control methods.
    List,
    /// Read the current brightness (0-100).
    Get {
        /// Monitor ID, index, or name from `list`.
        id: String,
    },
    /// Set brightness for one monitor (0-100).
    Set {
        /// Monitor ID, index, or name from `list`.
        id: String,
        /// Brightness value from 0 (darkest) to 100 (brightest).
        value: u8,
        /// Exit immediately instead of keeping overlay active.
        #[arg(long)]
        no_wait: bool,
    },
    /// Set brightness for every monitor.
    SetAll {
        /// Brightness value from 0 (darkest) to 100 (brightest).
        value: u8,
        /// Exit immediately instead of keeping overlay active.
        #[arg(long)]
        no_wait: bool,
    },
    /// Re-scan monitors (hot-plug, DDC/CI availability changes).
    Refresh,
    /// Keep overlay windows active until Ctrl+C.
    Hold,
    /// Watch for monitor connect/disconnect and refresh the list automatically.
    Watch,
    /// Adjust brightness with Ctrl+PgUp / Ctrl+PgDn hotkeys.
    Listen {
        /// Monitor ID, index, or name from `list`. Defaults to the first monitor.
        #[arg(long)]
        id: Option<String>,
        /// Brightness change per hotkey press.
        #[arg(long, default_value_t = 5)]
        step: u8,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match run(cli) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    };

    process::exit(result);
}

fn run(cli: Cli) -> brightness_core::Result<()> {
    let mut manager = MonitorManager::new()?;

    match cli.command {
        Commands::List => cmd_list(&manager),
        Commands::Get { id } => cmd_get(&manager, &id),
        Commands::Set { id, value, no_wait } => cmd_set(&mut manager, &id, value, no_wait),
        Commands::SetAll { value, no_wait } => cmd_set_all(&mut manager, value, no_wait),
        Commands::Refresh => cmd_refresh(&mut manager),
        Commands::Hold => cmd_hold(&manager),
        Commands::Watch => cmd_watch(&mut manager),
        Commands::Listen { id, step } => cmd_listen(&mut manager, id, step),
    }
}

fn cmd_list(manager: &MonitorManager) -> brightness_core::Result<()> {
    let monitors = manager.list_monitors()?;

    if monitors.is_empty() {
        println!("No monitors found.");
        return Ok(());
    }

    for (index, monitor) in monitors.iter().enumerate() {
        let brightness = manager.get_brightness(&monitor.id)?;
        println!("[{index}] id: {}", monitor.id);
        println!("    name: {}", monitor.name);
        println!("    method: {}", monitor.method.as_str());
        println!(
            "    bounds: {}x{} at ({}, {})",
            monitor.bounds.width, monitor.bounds.height, monitor.bounds.x, monitor.bounds.y
        );
        println!("    brightness: {brightness}%");
        println!();
    }

    Ok(())
}

fn cmd_get(manager: &MonitorManager, id: &str) -> brightness_core::Result<()> {
    let brightness = manager.get_brightness(id)?;
    println!("{brightness}");
    Ok(())
}

fn cmd_set(
    manager: &mut MonitorManager,
    id: &str,
    value: u8,
    no_wait: bool,
) -> brightness_core::Result<()> {
    let resolved_id = manager.resolve_id(id)?;
    manager.set_brightness(id, value)?;

    let method = manager
        .list_monitors()?
        .into_iter()
        .find(|m| m.id == resolved_id)
        .map(|m| m.method)
        .unwrap_or(ControlMethod::Overlay);

    println!("Set {resolved_id} to {value}% via {}", method.as_str());

    if !no_wait && manager.uses_overlay(id) && value < 100 {
        hold_overlay(manager);
    }

    Ok(())
}

fn cmd_set_all(
    manager: &mut MonitorManager,
    value: u8,
    no_wait: bool,
) -> brightness_core::Result<()> {
    manager.set_all_brightness(value)?;
    println!("Set all monitors to {value}%");

    if !no_wait && manager.has_overlay_targets() && value < 100 {
        hold_overlay(manager);
    }

    Ok(())
}

fn cmd_refresh(manager: &mut MonitorManager) -> brightness_core::Result<()> {
    manager.refresh()?;
    println!("Monitor list refreshed.");
    cmd_list(manager)
}

fn cmd_hold(manager: &MonitorManager) -> brightness_core::Result<()> {
    hold_overlay(manager);
    Ok(())
}

fn cmd_watch(manager: &mut MonitorManager) -> brightness_core::Result<()> {
    let watcher = manager.watch_hotplug()?;
    println!("Watching for monitor changes. Press Ctrl+C to exit.");

    loop {
        match watcher.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(()) => {
                println!("\nMonitor change detected, refreshing...");
                manager.refresh()?;
                cmd_list(manager)?;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Error::Internal("hotplug watcher disconnected".into()));
            }
        }
    }
}

fn cmd_listen(
    manager: &mut MonitorManager,
    id: Option<String>,
    step: u8,
) -> brightness_core::Result<()> {
    let target = match id {
        Some(id) => manager.resolve_id(&id)?,
        None => manager
            .list_monitors()?
            .into_iter()
            .next()
            .map(|monitor| monitor.id)
            .ok_or_else(|| Error::NotFound("no monitors available".into()))?,
    };

    let ctx = RefCell::new((manager, target.clone()));
    hotkey::run_hotkey_loop(&target, step, |delta| {
        let id = {
            let borrowed = ctx.borrow();
            borrowed.1.clone()
        };
        ctx.borrow_mut().0.adjust_brightness(&id, delta)
    })
}

fn hold_overlay(manager: &MonitorManager) {
    println!("Overlay active. Press Ctrl+C to exit and remove dimming.");
    manager.wait_for_shutdown();
}
