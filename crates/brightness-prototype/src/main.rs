#[cfg(windows)]
mod hotkey;

use std::io::{self, Write};

use brightness_core::{Error, MonitorManager};

struct SelectedMonitor {
    number: usize,
    id: String,
    name: String,
}

fn main() {
    let code = match run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    };
    std::process::exit(code);
}

fn run() -> brightness_core::Result<()> {
    let mut manager = MonitorManager::new()?;
    let monitors = manager.list_monitors()?;

    if monitors.is_empty() {
        return Err(Error::NotFound("no monitors available".into()));
    }

    println!("Monitors:");
    for (index, monitor) in monitors.iter().enumerate() {
        let number = index + 1;
        let brightness = match manager.get_brightness(&monitor.id) {
            Ok(value) => format!("{value}%"),
            Err(_) => "n/a".to_string(),
        };
        println!(
            "  [{number}] {} ({}) {brightness}",
            monitor.name,
            monitor.method.as_str()
        );
    }
    println!();

    let selected = prompt_selection(monitors.len())?;
    let targets: Vec<SelectedMonitor> = selected
        .into_iter()
        .map(|index| {
            let monitor = &monitors[index];
            SelectedMonitor {
                number: index + 1,
                id: monitor.id.clone(),
                name: monitor.name.clone(),
            }
        })
        .collect();

    let labels: Vec<String> = targets
        .iter()
        .map(|target| format!("[{}] {}", target.number, target.name))
        .collect();
    println!("Selected: {}", labels.join(", "));
    println!("Hotkeys: Ctrl+PgUp brighter, Ctrl+PgDn darker, Ctrl+C exit");
    println!();

    let initial = render_status(&mut manager, &targets)?;
    write_status_line(&initial)?;

    #[cfg(windows)]
    hotkey::run_hotkey_loop(5, |delta| adjust_selected(&mut manager, &targets, delta))?;

    #[cfg(not(windows))]
    {
        let _ = adjust_selected;
        return Err(Error::PlatformUnsupported);
    }

    println!();
    Ok(())
}

fn prompt_selection(count: usize) -> brightness_core::Result<Vec<usize>> {
    loop {
        if count == 1 {
            print!("Select monitor [1]: ");
        } else {
            print!("Select monitor(s) comma separated [1-{count}]: ");
        }
        io::stdout()
            .flush()
            .map_err(|err| Error::Internal(err.to_string()))?;

        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|err| Error::Internal(err.to_string()))?;

        let trimmed = line.trim();
        let input = if trimmed.is_empty() && count == 1 {
            "1".to_string()
        } else if trimmed.is_empty() {
            eprintln!("Enter at least one monitor number.");
            continue;
        } else {
            trimmed.to_string()
        };

        match parse_selection(&input, count) {
            Ok(indices) => return Ok(indices),
            Err(err) => eprintln!("{err}"),
        }
    }
}

fn parse_selection(input: &str, count: usize) -> brightness_core::Result<Vec<usize>> {
    let mut indices = Vec::new();

    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let number: usize = part
            .parse()
            .map_err(|_| Error::Internal(format!("invalid number: {part}")))?;

        if number == 0 || number > count {
            return Err(Error::Internal(format!(
                "out of range: {number} (valid: 1-{count})"
            )));
        }

        let index = number - 1;
        if !indices.contains(&index) {
            indices.push(index);
        }
    }

    if indices.is_empty() {
        return Err(Error::Internal("no monitors selected".into()));
    }

    indices.sort_unstable();
    Ok(indices)
}

fn adjust_selected(
    manager: &mut MonitorManager,
    targets: &[SelectedMonitor],
    delta: i8,
) -> brightness_core::Result<String> {
    for target in targets {
        let _ = manager.adjust_brightness(&target.id, delta)?;
    }
    render_status(manager, targets)
}

fn render_status(
    manager: &MonitorManager,
    targets: &[SelectedMonitor],
) -> brightness_core::Result<String> {
    let mut parts = Vec::with_capacity(targets.len());
    for target in targets {
        let brightness = manager.get_brightness(&target.id)?;
        parts.push(format!(
            "[{}] {} {}%",
            target.number, target.name, brightness
        ));
    }
    Ok(parts.join(" | "))
}

fn write_status_line(status: &str) -> brightness_core::Result<()> {
    print!("\r{status}   ");
    io::stdout()
        .flush()
        .map_err(|err| Error::Internal(err.to_string()))
}
