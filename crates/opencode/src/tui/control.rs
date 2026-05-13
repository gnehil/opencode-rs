use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuiRequest {
    pub path: String,
    pub body: serde_json::Value,
}

pub struct TuiControl {
    request_tx: mpsc::Sender<TuiRequest>,
    request_rx: Mutex<mpsc::Receiver<TuiRequest>>,
    response_tx: mpsc::Sender<serde_json::Value>,
    response_rx: Mutex<mpsc::Receiver<serde_json::Value>>,
}

impl Default for TuiControl {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiControl {
    pub fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel(256);
        let (response_tx, response_rx) = mpsc::channel(256);
        Self {
            request_tx,
            request_rx: Mutex::new(request_rx),
            response_tx,
            response_rx: Mutex::new(response_rx),
        }
    }

    pub async fn submit_request(&self, request: TuiRequest) -> Result<()> {
        self.request_tx.send(request).await?;
        Ok(())
    }

    pub async fn next_request(&self) -> Option<TuiRequest> {
        self.request_rx.lock().await.recv().await
    }

    pub async fn submit_response(&self, response: serde_json::Value) -> Result<()> {
        self.response_tx.send(response).await?;
        Ok(())
    }

    pub async fn next_response(&self) -> Option<serde_json::Value> {
        self.response_rx.lock().await.recv().await
    }
}
