use serde::Serialize;
use thiserror::Error;

use crate::permission::PermissionRule;

/// Errors that can occur during permission evaluation.
#[derive(Debug, Error, Serialize)]
pub enum PermissionError {
    /// The user rejected the permission request.
    #[error("The user rejected permission to use this specific tool call.")]
    RejectedError,

    /// The user rejected with feedback/correction.
    #[error("The user rejected permission with feedback: {feedback}")]
    CorrectedError {
        /// Correction feedback from the user.
        feedback: String,
    },

    /// A rule in the ruleset denied this permission.
    #[error("The user has specified a rule which prevents this tool call.")]
    DeniedError {
        /// The matching deny rules from the ruleset.
        #[serde(skip_serializing)]
        ruleset: Vec<PermissionRule>,
    },
}

impl PermissionError {
    /// Get the deny ruleset if this is a DeniedError.
    pub fn ruleset(&self) -> Option<&[PermissionRule]> {
        match self {
            PermissionError::DeniedError { ruleset } => Some(ruleset),
            _ => None,
        }
    }
}
