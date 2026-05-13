//! Exponential-backoff retry for provider calls.
//!
//! Real-world LLM providers fail transiently in three flavors:
//!   * **429 Too Many Requests** — rate-limit or quota throttling.
//!   * **5xx** — provider-side incidents.
//!   * **Network/connection** — timeouts, broken pipes during streaming.
//!
//! None of these mean "the request is invalid"; they mean "try again in a
//! moment." Without retry, the entire agent turn fails on the first
//! glitch — unacceptable for long-running sessions.
//!
//! We deliberately do **not** retry on:
//!   * 4xx other than 429 (bad request, auth failure, model not found).
//!   * Serialization errors (our bug, not theirs).
//!   * Missing API key.

use std::time::Duration;

use crate::provider::{
    CompletionRequest, CompletionResponse, Provider, ProviderError, ProviderResult,
};

/// Default retry policy: up to 4 attempts (= 3 retries) with base delay
/// 500ms, doubled each attempt. Worst-case wait: 0.5 + 1 + 2 = 3.5s
/// between the first failure and the last attempt.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 4;
pub const DEFAULT_BASE_DELAY_MS: u64 = 500;

pub fn is_retryable(err: &ProviderError) -> bool {
    match err {
        // reqwest::Error covers timeouts, connection resets, DNS failures.
        ProviderError::Http(e) => {
            // `is_timeout` and `is_connect` are the two transient cases.
            // `is_decode` (bad JSON in response body) is treated as
            // transient too — it usually means the provider returned an
            // HTML error page on a bad day.
            e.is_timeout() || e.is_connect() || e.is_request() || e.is_decode()
        }
        ProviderError::Api { status, .. } => *status == 429 || *status >= 500,
        ProviderError::StreamError(_) => true,
        ProviderError::Serialization(_)
        | ProviderError::MissingApiKey
        | ProviderError::ModelNotFound(_) => false,
    }
}

/// Run `provider.complete(request)` with retry on transient errors.
///
/// Returns the first successful response, or the **last** error (so
/// callers see the most recent failure, not a stale early one).
pub async fn complete_with_retry(
    provider: &dyn Provider,
    request: CompletionRequest,
    max_attempts: u32,
    base_delay: Duration,
) -> ProviderResult<CompletionResponse> {
    let mut last_err: Option<ProviderError> = None;
    for attempt in 0..max_attempts {
        match provider.complete(request.clone()).await {
            Ok(r) => return Ok(r),
            Err(e) => {
                let retryable = is_retryable(&e);
                if !retryable || attempt + 1 == max_attempts {
                    return Err(e);
                }
                let delay = base_delay * 2u32.pow(attempt);
                tracing::warn!(
                    "provider call failed (attempt {} of {}), retrying in {:?}: {}",
                    attempt + 1,
                    max_attempts,
                    delay,
                    e
                );
                tokio::time::sleep(delay).await;
                last_err = Some(e);
            }
        }
    }
    // Loop body always returns on the last attempt, so this is
    // unreachable in practice. Keep it for type safety.
    Err(last_err.unwrap_or(ProviderError::stream("retry loop exhausted")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_429_is_retryable() {
        let e = ProviderError::api(429, "slow down");
        assert!(is_retryable(&e));
    }

    #[test]
    fn api_503_is_retryable() {
        let e = ProviderError::api(503, "upstream down");
        assert!(is_retryable(&e));
    }

    #[test]
    fn api_400_is_not_retryable() {
        let e = ProviderError::api(400, "bad request");
        assert!(!is_retryable(&e));
    }

    #[test]
    fn api_401_is_not_retryable() {
        let e = ProviderError::api(401, "unauthorized");
        assert!(!is_retryable(&e));
    }

    #[test]
    fn missing_api_key_is_not_retryable() {
        assert!(!is_retryable(&ProviderError::MissingApiKey));
    }

    #[test]
    fn model_not_found_is_not_retryable() {
        assert!(!is_retryable(&ProviderError::ModelNotFound(
            "x".to_string()
        )));
    }

    #[test]
    fn stream_error_is_retryable() {
        assert!(is_retryable(&ProviderError::stream("connection reset")));
    }
}
