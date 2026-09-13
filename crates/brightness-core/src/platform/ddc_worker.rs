use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HANDLE;

use super::enumerate;
use crate::error::{Error, Result};

const MIN_WRITE_INTERVAL: Duration = Duration::from_millis(100);
const WRITE_RETRY_DELAY: Duration = Duration::from_millis(150);
const MAX_WRITE_RETRIES: u8 = 5;
const WORKER_POLL: Duration = Duration::from_millis(4);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MonitorKey {
    pub display_name: String,
    pub description: String,
}

impl MonitorKey {
    pub fn new(display_name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            display_name: display_name.into(),
            description: description.into(),
        }
    }
}

enum WorkerCommand {
    SetBrightness {
        key: MonitorKey,
        value: u8,
        ack: Option<Sender<Result<()>>>,
    },
    GetBrightness {
        key: MonitorKey,
        reply: Sender<Result<u8>>,
    },
    InvalidateCache,
    Shutdown,
}

struct CachedMonitor {
    handle: HANDLE,
    max_brightness: u32,
    applied: u8,
    pending: Option<u8>,
    pending_ack: Option<(u8, Sender<Result<()>>)>,
    last_write: Option<Instant>,
    write_retries: u8,
}

pub struct DdcWorker {
    tx: Sender<WorkerCommand>,
    thread: Option<JoinHandle<()>>,
}

impl DdcWorker {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("brightness-ddc".into())
            .spawn(move || worker_main(rx))
            .expect("failed to spawn DDC worker thread");

        Self {
            tx,
            thread: Some(thread),
        }
    }

    pub fn set_brightness(
        &self,
        display_name: &str,
        description: &str,
        value: u8,
    ) -> Result<()> {
        self.set_brightness_inner(display_name, description, value, false)
    }

    pub fn set_brightness_and_wait(
        &self,
        display_name: &str,
        description: &str,
        value: u8,
    ) -> Result<()> {
        self.set_brightness_inner(display_name, description, value, true)
    }

    fn set_brightness_inner(
        &self,
        display_name: &str,
        description: &str,
        value: u8,
        wait: bool,
    ) -> Result<()> {
        let (ack, reply_rx) = if wait {
            let (reply_tx, reply_rx) = mpsc::channel();
            (Some(reply_tx), Some(reply_rx))
        } else {
            (None, None)
        };

        self.tx
            .send(WorkerCommand::SetBrightness {
                key: MonitorKey::new(display_name, description),
                value,
                ack,
            })
            .map_err(|_| Error::Ddc("DDC worker channel closed".into()))?;

        if let Some(reply_rx) = reply_rx {
            return reply_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| Error::Ddc("DDC worker write timed out".into()))?;
        }

        Ok(())
    }

    pub fn get_brightness(&self, display_name: &str, description: &str) -> Result<u8> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(WorkerCommand::GetBrightness {
                key: MonitorKey::new(display_name, description),
                reply: reply_tx,
            })
            .map_err(|_| Error::Ddc("DDC worker channel closed".into()))?;

        reply_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| Error::Ddc("DDC worker read timed out".into()))?
    }

    pub fn invalidate_cache(&self) -> Result<()> {
        self.tx
            .send(WorkerCommand::InvalidateCache)
            .map_err(|_| Error::Ddc("DDC worker channel closed".into()))
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(WorkerCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for DdcWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn worker_main(rx: Receiver<WorkerCommand>) {
    let mut monitors: HashMap<MonitorKey, CachedMonitor> = HashMap::new();
    let mut running = true;

    while running {
        drain_commands(&rx, &mut monitors, &mut running);

        if !running {
            break;
        }

        flush_pending(&mut monitors);

        match rx.recv_timeout(WORKER_POLL) {
            Ok(command) => dispatch_command(command, &mut monitors, &mut running),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    for monitor in monitors.values() {
        enumerate::release_ddc_handle(monitor.handle);
    }
}

fn drain_commands(
    rx: &Receiver<WorkerCommand>,
    monitors: &mut HashMap<MonitorKey, CachedMonitor>,
    running: &mut bool,
) {
    while let Ok(command) = rx.try_recv() {
        dispatch_command(command, monitors, running);
        if !*running {
            return;
        }
    }
}

fn dispatch_command(
    command: WorkerCommand,
    monitors: &mut HashMap<MonitorKey, CachedMonitor>,
    running: &mut bool,
) {
    match command {
        WorkerCommand::SetBrightness { key, value, ack } => {
            if let Err(err) = queue_brightness(monitors, &key, value, ack) {
                eprintln!("brightness-core: DDC queue failed for {}: {err}", key.description);
            }
        }
        WorkerCommand::GetBrightness { key, reply } => {
            let result = read_brightness(monitors, &key);
            let _ = reply.send(result);
        }
        WorkerCommand::InvalidateCache => invalidate_cache(monitors),
        WorkerCommand::Shutdown => *running = false,
    }
}

fn invalidate_cache(monitors: &mut HashMap<MonitorKey, CachedMonitor>) {
    for monitor in monitors.values() {
        enumerate::release_ddc_handle(monitor.handle);
    }
    monitors.clear();
}

fn queue_brightness(
    monitors: &mut HashMap<MonitorKey, CachedMonitor>,
    key: &MonitorKey,
    value: u8,
    ack: Option<Sender<Result<()>>>,
) -> Result<()> {
    let monitor = ensure_monitor(monitors, key)?;
    let current = effective_brightness(monitor);
    if current == value {
        if let Some(tx) = ack {
            let _ = tx.send(Ok(()));
        }
        return Ok(());
    }

    monitor.pending = Some(value);
    monitor.write_retries = 0;
    if let Some(tx) = ack {
        monitor.pending_ack = Some((value, tx));
    }
    Ok(())
}

fn effective_brightness(monitor: &CachedMonitor) -> u8 {
    monitor.pending.unwrap_or(monitor.applied)
}

fn read_brightness(monitors: &mut HashMap<MonitorKey, CachedMonitor>, key: &MonitorKey) -> Result<u8> {
    if let Some(monitor) = monitors.get(key) {
        if let Some(pending) = monitor.pending {
            return Ok(pending);
        }
        return Ok(monitor.applied);
    }

    let monitor = ensure_monitor(monitors, key)?;
    let (current, max) = enumerate::read_brightness(monitor.handle)?;
    let normalized = normalize(current, max);
    monitor.applied = normalized;
    Ok(normalized)
}

fn flush_pending(monitors: &mut HashMap<MonitorKey, CachedMonitor>) {
    let now = Instant::now();

    for monitor in monitors.values_mut() {
        let Some(target) = monitor.pending else {
            continue;
        };

        if target == monitor.applied && monitor.pending_ack.is_none() {
            monitor.pending = None;
            continue;
        }

        let min_interval = if monitor.write_retries > 0 {
            WRITE_RETRY_DELAY
        } else {
            MIN_WRITE_INTERVAL
        };

        if let Some(last_write) = monitor.last_write {
            if now.duration_since(last_write) < min_interval {
                continue;
            }
        }

        let raw = denormalize(target, monitor.max_brightness);
        match enumerate::write_brightness(monitor.handle, raw) {
            Ok(()) => {
                monitor.applied = target;
                monitor.pending = None;
                monitor.write_retries = 0;
                monitor.last_write = Some(now);
                if let Some((ack_value, ack)) = monitor.pending_ack.take() {
                    if ack_value == target {
                        let _ = ack.send(Ok(()));
                    } else {
                        monitor.pending_ack = Some((ack_value, ack));
                    }
                }
            }
            Err(err) => {
                monitor.write_retries = monitor.write_retries.saturating_add(1);
                monitor.last_write = Some(now);

                if monitor.write_retries >= MAX_WRITE_RETRIES {
                    eprintln!("brightness-core: DDC write failed after retries: {err}");
                    monitor.pending = None;
                    monitor.write_retries = 0;
                    if let Some((_, ack)) = monitor.pending_ack.take() {
                        let _ = ack.send(Err(err));
                    }
                }
            }
        }
    }
}

fn ensure_monitor<'a>(
    monitors: &'a mut HashMap<MonitorKey, CachedMonitor>,
    key: &MonitorKey,
) -> Result<&'a mut CachedMonitor> {
    if !monitors.contains_key(key) {
        let (handle, _) = enumerate::acquire_ddc_handle(&key.display_name, &key.description)?;
        let (current, max) = enumerate::read_brightness(handle)?;
        let applied = normalize(current, max);

        monitors.insert(
            key.clone(),
            CachedMonitor {
                handle,
                max_brightness: max,
                applied,
                pending: None,
                pending_ack: None,
                last_write: None,
                write_retries: 0,
            },
        );
    }

    Ok(monitors.get_mut(key).expect("monitor just inserted"))
}

fn normalize(current: u32, max: u32) -> u8 {
    if max == 0 {
        return 0;
    }
    ((current * 100) / max).min(100) as u8
}

fn denormalize(value: u8, max: u32) -> u32 {
    (u32::from(value) * max / 100).min(max)
}

#[cfg(test)]
mod tests {
    use super::{denormalize, normalize};

    #[test]
    fn brightness_mapping() {
        assert_eq!(normalize(50, 100), 50);
        assert_eq!(denormalize(50, 100), 50);
    }
}
