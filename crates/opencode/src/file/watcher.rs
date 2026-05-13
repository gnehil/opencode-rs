use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct FileWatcher {
    watcher: RecommendedWatcher,
    events_rx: mpsc::Receiver<FileEvent>,
}

#[derive(Debug, Clone)]
pub struct FileEvent {
    pub path: PathBuf,
    pub kind: FileEventKind,
}

#[derive(Debug, Clone)]
pub enum FileEventKind {
    Created,
    Modified,
    Deleted,
    Renamed { from: PathBuf, to: PathBuf },
}

impl FileWatcher {
    pub fn new(path: &PathBuf) -> anyhow::Result<Self> {
        let (events_tx, events_rx) = mpsc::channel::<FileEvent>(256);

        let event_handler = move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if let Some(file_event) = Self::convert_event(event) {
                    let _ = events_tx.blocking_send(file_event);
                }
            }
        };

        let mut watcher = RecommendedWatcher::new(event_handler, notify::Config::default())?;
        watcher.watch(path, RecursiveMode::Recursive)?;

        Ok(Self { watcher, events_rx })
    }

    pub fn events(&mut self) -> &mut mpsc::Receiver<FileEvent> {
        &mut self.events_rx
    }

    fn convert_event(event: Event) -> Option<FileEvent> {
        match event.kind {
            EventKind::Create(_) => event.paths.first().map(|p| FileEvent {
                path: p.clone(),
                kind: FileEventKind::Created,
            }),
            EventKind::Modify(_) => event.paths.first().map(|p| FileEvent {
                path: p.clone(),
                kind: FileEventKind::Modified,
            }),
            EventKind::Remove(_) => event.paths.first().map(|p| FileEvent {
                path: p.clone(),
                kind: FileEventKind::Deleted,
            }),
            EventKind::Any => {
                if event.paths.len() == 2 {
                    Some(FileEvent {
                        path: event.paths[1].clone(),
                        kind: FileEventKind::Renamed {
                            from: event.paths[0].clone(),
                            to: event.paths[1].clone(),
                        },
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
