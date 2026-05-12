use serde::{Deserialize, Serialize};

use crate::id::SessionID;

use super::model::SessionModel;
use super::revert::SessionRevert;
use super::share::SessionShare;
use super::summary::SessionSummary;
use super::time::SessionTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: SessionID,
    pub slug: String,
    pub project_id: String,
    pub workspace_id: Option<String>,
    pub directory: String,
    pub path: Option<String>,
    pub parent_id: Option<SessionID>,
    pub title: String,
    pub agent: Option<String>,
    pub model: Option<SessionModel>,
    pub version: String,
    pub summary: Option<SessionSummary>,
    pub share: Option<SessionShare>,
    pub revert: Option<SessionRevert>,
    pub permission: Option<Vec<String>>,
    pub time: SessionTime,
}
