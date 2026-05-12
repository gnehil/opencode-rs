//! Token budget arithmetic for deciding when to compact the session.
//!
//! A model has a `context` window measured in tokens. Some of that window
//! is reserved for the model's reply (`output`), so the practical input
//! budget is smaller. When the accumulated conversation usage approaches
//! this practical budget we trigger compaction, summarising prior turns
//! into a single message that takes the place of the older history.

use crate::provider::ModelInfo;

/// Fraction of usable_context at which compaction kicks in. Tuned to be
/// conservative — once we cross this line the *next* turn would likely
/// push past the model's hard ceiling.
pub const COMPACT_THRESHOLD: f64 = 0.85;

/// Compute the usable input budget for a model, in tokens.
///
/// Definition: `context - max_output - safety_buffer`. The safety buffer
/// is 1024 tokens to account for tokenization mismatch between the model
/// and our local counters (we report usage from the provider's response,
/// which doesn't include any system overhead the model itself adds).
pub fn usable_context(model: &ModelInfo) -> u64 {
    let limit = match &model.limit {
        Some(l) => l,
        None => return u64::MAX, // unknown — never trigger
    };
    let context = limit.context as i64;
    let output = limit.output as i64;
    let usable = context - output - 1024;
    if usable <= 0 { 0 } else { usable as u64 }
}

/// Should the orchestrator compact before the next turn?
///
/// `cumulative_input_tokens` is the sum of input tokens already consumed
/// in this session (we use the provider's reported input count for each
/// completed turn). Returns true when usage crosses `COMPACT_THRESHOLD`
/// of `usable_context`.
pub fn should_compact(cumulative_input_tokens: u64, model: &ModelInfo) -> bool {
    let budget = usable_context(model);
    if budget == 0 || budget == u64::MAX {
        return false;
    }
    (cumulative_input_tokens as f64) >= (budget as f64) * COMPACT_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ModelInfo, ModelLimit};

    fn model_with_context(context: u64, output: u64) -> ModelInfo {
        ModelInfo {
            id: None,
            name: None,
            family: None,
            reasoning: None,
            tool_call: None,
            attachment: None,
            temperature: None,
            interleaved: None,
            cost: None,
            limit: Some(ModelLimit {
                context: context as f64,
                input: None,
                output: output as f64,
            }),
            modalities: None,
            experimental: None,
            release_date: None,
            status: None,
            provider: None,
            options: None,
            headers: None,
            variants: None,
        }
    }

    #[test]
    fn under_threshold_does_not_compact() {
        // 200k context, 8k output → ~190k usable. 85% = ~161k.
        let model = model_with_context(200_000, 8_000);
        assert!(!should_compact(100_000, &model));
    }

    #[test]
    fn over_threshold_compacts() {
        let model = model_with_context(200_000, 8_000);
        assert!(should_compact(180_000, &model));
    }

    #[test]
    fn unknown_limit_never_compacts() {
        let model = ModelInfo {
            id: None, name: None, family: None, reasoning: None, tool_call: None,
            attachment: None, temperature: None, interleaved: None, cost: None,
            limit: None, modalities: None, experimental: None, release_date: None,
            status: None, provider: None, options: None, headers: None, variants: None,
        };
        assert!(!should_compact(1_000_000_000, &model));
    }
}
