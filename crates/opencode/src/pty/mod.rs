use anyhow::Result;
use portable_pty::{Child, CommandBuilder, PtyPair, PtySize as PortablePtySize, PtySystem};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

const BUFFER_LIMIT: usize = 1024 * 1024 * 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyID(pub String);

impl PtyID {
    pub fn new() -> Self {
        Self(ulid::Ulid::new().to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyInfo {
    pub id: PtyID,
    pub title: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub status: PtyStatus,
    pub pid: u32,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PtyStatus {
    Running,
    Exited,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyCreateInput {
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyUpdateInput {
    pub title: Option<String>,
    pub size: Option<PtySize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

pub struct PtySession {
    pub info: PtyInfo,
    // Sliding-window output buffer. We track two monotonic counters in absolute
    // bytes-since-spawn terms:
    //   - `buffer_cursor` = absolute byte offset of buffer[0]
    //   - `cursor`        = absolute byte offset of "one past last byte written"
    // The window invariant is: cursor - buffer_cursor == buffer.len().
    // When the buffer exceeds BUFFER_LIMIT we drop bytes from the front and
    // advance `buffer_cursor` by the same amount so consumers can detect when
    // they've fallen behind.
    pub buffer: Arc<std::sync::Mutex<Vec<u8>>>,
    pub buffer_cursor: Arc<AtomicUsize>,
    pub cursor: Arc<AtomicUsize>,
    pub pair: Arc<std::sync::Mutex<Option<PtyPair>>>,
    pub child: Arc<std::sync::Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    pub writer: Arc<std::sync::Mutex<Option<Box<dyn std::io::Write + Send>>>>,
    output_tx: mpsc::Sender<Vec<u8>>,
    killed: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
}

pub struct PtyService {
    sessions: Arc<RwLock<HashMap<String, PtySession>>>,
    event_bus: crate::bus::EventBus,
}

impl PtyService {
    pub fn new(event_bus: crate::bus::EventBus) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
        }
    }

    pub async fn list(&self) -> Vec<PtyInfo> {
        let sessions = self.sessions.read().await;
        sessions.values().map(|s| s.info.clone()).collect()
    }

    pub async fn get(&self, id: &PtyID) -> Option<PtyInfo> {
        let sessions = self.sessions.read().await;
        sessions.get(&id.0).map(|s| s.info.clone())
    }

    pub async fn create(&self, input: PtyCreateInput) -> Result<PtyInfo> {
        let id = PtyID::new();
        let command = input.command.clone().unwrap_or_else(|| {
            if std::env::consts::OS == "windows" {
                "cmd.exe".to_string()
            } else {
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
            }
        });
        let args = input.args.clone().unwrap_or_default();
        let cwd = input.cwd.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });
        let title = input
            .title
            .clone()
            .unwrap_or_else(|| format!("Terminal {}", &id.0[id.0.len().saturating_sub(4)..]));

        let pty_system: Box<dyn PtySystem + Send> = portable_pty::native_pty_system();
        let pair = pty_system.openpty(PortablePtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(&command);
        cmd.args(&args);
        cmd.cwd(&cwd);

        cmd.env("TERM", "xterm-256color");
        cmd.env("OPENCODE_TERMINAL", "1");

        if let Some(custom_env) = &input.env {
            for (key, value) in custom_env {
                cmd.env(key, value);
            }
        }

        let child = pair.slave.spawn_command(cmd)?;
        let pid = child.process_id().unwrap_or(0);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (output_tx, _output_rx) = mpsc::channel::<Vec<u8>>(256);

        let info = PtyInfo {
            id: id.clone(),
            title,
            command,
            args,
            cwd,
            status: PtyStatus::Running,
            pid,
            exit_code: None,
        };

        let sessions_clone = self.sessions.clone();
        let event_bus_clone = self.event_bus.clone();
        let session_id = id.0.clone();
        let buffer = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let buffer_cursor = Arc::new(AtomicUsize::new(0));
        let cursor = Arc::new(AtomicUsize::new(0));
        let pair_arc = Arc::new(std::sync::Mutex::new(Some(pair)));
        let child_arc: Arc<std::sync::Mutex<Option<Box<dyn Child + Send + Sync>>>> =
            Arc::new(std::sync::Mutex::new(Some(child)));
        let writer_arc: Arc<std::sync::Mutex<Option<Box<dyn std::io::Write + Send>>>> =
            Arc::new(std::sync::Mutex::new(Some(writer)));
        let killed = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));

        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(
                id.0.clone(),
                PtySession {
                    info: info.clone(),
                    buffer: buffer.clone(),
                    buffer_cursor: buffer_cursor.clone(),
                    cursor: cursor.clone(),
                    pair: pair_arc.clone(),
                    child: child_arc.clone(),
                    writer: writer_arc.clone(),
                    output_tx: output_tx.clone(),
                    killed: killed.clone(),
                    exited: exited.clone(),
                },
            );
        }

        self.event_bus
            .publish(crate::bus::Event::session_create(&id.0));

        // Reader: portable-pty's reader is blocking, so run it on a dedicated
        // OS thread. Use only sync primitives + blocking_send to avoid
        // block_on(handle) which can deadlock the runtime under load.
        let buffer_reader = buffer.clone();
        let buffer_cursor_reader = buffer_cursor.clone();
        let cursor_reader = cursor.clone();
        let output_tx_reader = output_tx.clone();
        let killed_reader = killed.clone();

        std::thread::spawn(move || {
            use std::io::Read;
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                if killed_reader.load(Ordering::SeqCst) {
                    break;
                }
                match reader.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let data = buf[..n].to_vec();
                        cursor_reader.fetch_add(n, Ordering::SeqCst);
                        {
                            let mut guard = buffer_reader.lock().unwrap();
                            guard.extend_from_slice(&data);
                            if guard.len() > BUFFER_LIMIT {
                                let excess = guard.len() - BUFFER_LIMIT;
                                guard.drain(0..excess);
                                buffer_cursor_reader.fetch_add(excess, Ordering::SeqCst);
                            }
                        }
                        // Best-effort: subscribers that have hung up don't
                        // matter, and we don't want to block reading from the
                        // PTY if no one is listening.
                        let _ = output_tx_reader.blocking_send(data);
                    }
                    Ok(_) | Err(_) => break,
                }
            }
        });

        // Wait for child exit on a dedicated OS thread; mutate shared state
        // through sync primitives + a tokio-spawned task for the async map
        // update so we don't block_on the current runtime handle.
        let exited_wait = exited.clone();
        let killed_wait = killed.clone();
        let child_wait = child_arc.clone();
        let sessions_for_wait = sessions_clone.clone();
        let session_id_for_wait = session_id.clone();
        let handle = tokio::runtime::Handle::current();

        std::thread::spawn(move || {
            let status = {
                let mut guard = child_wait.lock().unwrap();
                match guard.as_mut() {
                    Some(c) => c.wait(),
                    None => return,
                }
            };
            if killed_wait.load(Ordering::SeqCst) {
                return;
            }
            if let Ok(status) = status {
                let exit_code = status.exit_code() as i32;
                exited_wait.store(true, Ordering::SeqCst);
                let sessions = sessions_for_wait.clone();
                let sid = session_id_for_wait.clone();
                let bus = event_bus_clone.clone();
                handle.spawn(async move {
                    let mut sessions = sessions.write().await;
                    if let Some(session) = sessions.get_mut(&sid) {
                        if !session.killed.load(Ordering::SeqCst) {
                            session.info.status = PtyStatus::Exited;
                            session.info.exit_code = Some(exit_code);
                        }
                    }
                    bus.publish(crate::bus::Event::session_update(&sid));
                });
            }
        });

        Ok(info)
    }

    pub async fn update(&self, id: &PtyID, input: PtyUpdateInput) -> Option<PtyInfo> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&id.0) {
            if session.exited.load(Ordering::SeqCst) {
                return None;
            }
            if let Some(title) = input.title {
                session.info.title = title;
            }
            if let Some(size) = input.size {
                if let Some(pair) = session.pair.lock().unwrap().as_mut() {
                    let _ = pair.master.resize(PortablePtySize {
                        rows: size.rows,
                        cols: size.cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            }
            self.event_bus
                .publish(crate::bus::Event::session_update(&id.0));
            return Some(session.info.clone());
        }
        None
    }

    pub async fn remove(&self, id: &PtyID) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.remove(&id.0) {
            session.killed.store(true, Ordering::SeqCst);
            if let Some(c) = session.child.lock().unwrap().as_mut() {
                let _ = c.kill();
            }
            self.event_bus
                .publish(crate::bus::Event::session_delete(&id.0));
        }
        Ok(())
    }

    pub async fn resize(&self, id: &PtyID, cols: u16, rows: u16) -> Result<()> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(&id.0) {
            if session.exited.load(Ordering::SeqCst) || session.killed.load(Ordering::SeqCst) {
                return Ok(());
            }
            if let Some(pair) = session.pair.lock().unwrap().as_mut() {
                pair.master.resize(PortablePtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })?;
            }
        }
        Ok(())
    }

    pub async fn write(&self, id: &PtyID, data: &[u8]) -> Result<()> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(&id.0) {
            if session.exited.load(Ordering::SeqCst) || session.killed.load(Ordering::SeqCst) {
                return Err(anyhow::anyhow!("Session has exited"));
            }
            let data_clone = data.to_vec();
            let writer_arc = session.writer.clone();
            let killed_check = session.killed.clone();

            let join: Result<Result<()>, _> = tokio::task::spawn_blocking(move || -> Result<()> {
                use std::io::Write;
                if killed_check.load(Ordering::SeqCst) {
                    return Ok(());
                }
                if let Some(writer) = writer_arc.lock().unwrap().as_mut() {
                    writer.write_all(&data_clone)?;
                    writer.flush()?;
                }
                Ok(())
            })
            .await;
            join??;
        }
        Ok(())
    }

    /// Return any output between absolute byte offset `cursor` (defaults to 0)
    /// and the current write head, along with the new write-head offset that
    /// the caller should pass next time.
    ///
    /// If `cursor` is older than the buffer's start (the reader has rotated it
    /// out), the returned data starts from the oldest still-available byte and
    /// callers should treat that as a forced resync.
    pub async fn connect(&self, id: &PtyID, cursor: Option<usize>) -> Option<(Vec<u8>, usize)> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&id.0)?;

        let start = session.buffer_cursor.load(Ordering::SeqCst);
        let end = session.cursor.load(Ordering::SeqCst);
        let from = cursor.unwrap_or(0);

        let buffer = session.buffer.lock().unwrap();
        let offset = from.saturating_sub(start).min(buffer.len());
        let data = buffer[offset..].to_vec();
        Some((data, end))
    }

    pub async fn kill(&self, id: &PtyID) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&id.0) {
            if session.exited.load(Ordering::SeqCst) {
                return Ok(());
            }
            session.killed.store(true, Ordering::SeqCst);
            if let Some(c) = session.child.lock().unwrap().as_mut() {
                let _ = c.kill();
            }
            session.info.status = PtyStatus::Exited;
        }
        Ok(())
    }

    pub async fn wait_exit(&self, id: &PtyID) -> Option<i32> {
        let sessions = self.sessions.read().await;
        sessions.get(&id.0).and_then(|s| s.info.exit_code)
    }
}
