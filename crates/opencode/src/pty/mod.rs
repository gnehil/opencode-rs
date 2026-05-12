use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, RwLock};
use serde::{Deserialize, Serialize};
use portable_pty::{PtyPair, PtySize as PortablePtySize, CommandBuilder, PtySystem};
use anyhow::Result;

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
    pub buffer: Arc<RwLock<Vec<u8>>>,
    pub buffer_cursor: usize,
    pub cursor: Arc<RwLock<usize>>,
    pub pair: Arc<std::sync::Mutex<Option<PtyPair>>>,
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
        let cwd = input.cwd.clone().unwrap_or_else(|| std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string()));
        let title = input.title.clone()
            .unwrap_or_else(|| format!("Terminal {}", &id.0[id.0.len().saturating_sub(4)..]));

        let pty_system: PtySystem = portable_pty::native_pty_system();
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

        let mut child = pair.slave.spawn_command(cmd)?;
        let pid = child.pid();
        
        let reader = pair.master.take_reader()?;
        let (output_tx, _output_rx) = mpsc::channel::<Vec<u8>>(256);

        let info = PtyInfo {
            id: id.clone(),
            title,
            command,
            args,
            cwd,
            status: PtyStatus::Running,
            pid: pid as u32,
            exit_code: None,
        };

        let sessions_clone = self.sessions.clone();
        let event_bus_clone = self.event_bus.clone();
        let session_id = id.0.clone();
        let buffer = Arc::new(RwLock::new(Vec::new()));
        let cursor = Arc::new(RwLock::new(0usize));
        let pair_arc = Arc::new(std::sync::Mutex::new(Some(pair)));
        let killed = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));

        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(id.0.clone(), PtySession {
                info: info.clone(),
                buffer: buffer.clone(),
                buffer_cursor: 0,
                cursor: cursor.clone(),
                pair: pair_arc.clone(),
                output_tx: output_tx.clone(),
                killed: killed.clone(),
                exited: exited.clone(),
            });
        }

        self.event_bus.publish(crate::bus::Event::session_create(&id.0));

        let buffer_clone = buffer.clone();
        let cursor_clone = cursor.clone();
        let output_tx_clone = output_tx.clone();
        let killed_reader = killed.clone();

        tokio::task::spawn_blocking(move || {
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
                        let cursor_clone = cursor_clone.clone();
                        let buffer_clone = buffer_clone.clone();
                        let output_tx_clone = output_tx_clone.clone();
                        
                        tokio::runtime::Handle::current().block_on(async {
                            *cursor_clone.write().await += n;
                            buffer_clone.write().await.extend_from_slice(&data);
                            
                            if buffer_clone.read().await.len() > BUFFER_LIMIT {
                                let mut buffer = buffer_clone.write().await;
                                let excess = buffer.len() - BUFFER_LIMIT;
                                *buffer = buffer[excess..].to_vec();
                            }
                            
                            let _ = output_tx_clone.send(data).await;
                        });
                    }
                    Ok(_) | Err(_) => break,
                }
            }
        });

        let exited_wait = exited.clone();
        let killed_wait = killed.clone();

        tokio::task::spawn_blocking(move || {
            let result = child.wait();
            if killed_wait.load(Ordering::SeqCst) {
                return;
            }
            match result {
                Ok(status) => {
                    let exit_code = status.exit_code();
                    exited_wait.store(true, Ordering::SeqCst);
                    tokio::runtime::Handle::current().block_on(async {
                        let mut sessions = sessions_clone.write().await;
                        if let Some(session) = sessions.get_mut(&session_id) {
                            if !session.killed.load(Ordering::SeqCst) {
                                session.info.status = PtyStatus::Exited;
                                session.info.exit_code = Some(exit_code);
                            }
                        }
                        event_bus_clone.publish(crate::bus::Event::session_update(&session_id));
                    });
                }
                Err(_) => {}
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
            self.event_bus.publish(crate::bus::Event::session_update(&id.0));
            return Some(session.info.clone());
        }
        None
    }

    pub async fn remove(&self, id: &PtyID) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.remove(&id.0) {
            session.killed.store(true, Ordering::SeqCst);
            if let Some(pair) = session.pair.lock().unwrap().as_mut() {
                if let Some(mut child) = pair.slave.child() {
                    let _ = child.kill();
                }
            }
            self.event_bus.publish(crate::bus::Event::session_delete(&id.0));
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
            let pair_arc = session.pair.clone();
            let killed_check = session.killed.clone();
            
            tokio::task::spawn_blocking(move || {
                use std::io::Write;
                if killed_check.load(Ordering::SeqCst) {
                    return Ok(());
                }
                if let Some(pair) = pair_arc.lock().unwrap().as_mut() {
                    let mut writer = pair.master.take_writer()?;
                    writer.write_all(&data_clone)?;
                    writer.flush()?;
                }
                Ok(())
            }).await?;
        }
        Ok(())
    }

    pub async fn connect(&self, id: &PtyID, cursor: Option<usize>) -> Option<(Vec<u8>, usize)> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(&id.0) {
            let start = session.buffer_cursor;
            let end = *session.cursor.read().await;
            let from = cursor.unwrap_or(0);
            
            let offset = from.saturating_sub(start);
            let buffer = session.buffer.read().await;
            let data = if offset < buffer.len() {
                buffer[offset..].to_vec()
            } else {
                Vec::new()
            };

            Some((data, end))
        } else {
            None
        }
    }

    pub async fn subscribe(&self, id: &PtyID) -> Option<mpsc::Receiver<Vec<u8>>> {
        let sessions = self.sessions.read().await;
        sessions.get(&id.0).map(|s| s.output_tx.subscribe())
    }

    pub async fn kill(&self, id: &PtyID) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&id.0) {
            if session.exited.load(Ordering::SeqCst) {
                return Ok(());
            }
            session.killed.store(true, Ordering::SeqCst);
            if let Some(pair) = session.pair.lock().unwrap().as_mut() {
                if let Some(mut child) = pair.slave.child() {
                    let _ = child.kill();
                }
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