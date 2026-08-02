//! Cost estimation. Pure arithmetic, honest about what it does not know.
//!
//! # This is a rough guide, not a quote
//!
//! It knows the price bands and a chars/4 token heuristic. It does **not** know Together's
//! tokenizer (measured 3.6x optimistic against a real job) and it cannot know the **minimum
//! charge**, which dominates small datasets: a job metering at $0.076 of tokens was billed
//! **$4.00**. For a real number, upload the file and ask
//! [`crate::provider::together_http::TogetherClient::estimate_price`] — it is free.
//!
//! Charter open question: **hosting is a second, ongoing charge.** A Together-tuned model
//! bills for its dedicated endpoint separately from training, so this reports training only
//! and *says so*. A single blended number would be the dishonest kind of simplification.

use serde::{Deserialize, Serialize};

use crate::provider::together;

/// Characters per token. A rough industry heuristic, and labelled as such wherever it lands.
const CHARS_PER_TOKEN: f64 = 4.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    /// Tokens in one pass over the dataset.
    pub dataset_tokens: u64,
    /// What actually gets billed: one pass per epoch.
    pub training_tokens: u64,
    pub epochs: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_per_mtok_usd: Option<f64>,
    /// `None` when the band does not cover this model — a refusal, not a guess.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub training_usd: Option<f64>,
    /// Everything the number does not include, in plain words.
    pub caveats: Vec<String>,
}

/// Tokens from raw character count. Approximate by construction.
pub fn tokens_from_chars(chars: u64) -> u64 {
    (chars as f64 / CHARS_PER_TOKEN).ceil() as u64
}

/// Estimate a Together LoRA fine-tune.
///
/// `params_b` is the base model's parameter count in billions — 27.0 for Qwen3.6-27B.
pub fn together_lora(dataset_chars: u64, epochs: u32, params_b: f64) -> Estimate {
    let dataset_tokens = tokens_from_chars(dataset_chars);
    let training_tokens = dataset_tokens.saturating_mul(epochs.max(1) as u64);
    let price = together::lora_price_per_mtok(params_b);

    let mut caveats = vec![
        format!(
            "token count is approximate (~{CHARS_PER_TOKEN:.0} chars/token), not a tokenizer count"
        ),
        "training only — hosting a tuned model is a separate ongoing charge".to_string(),
        "IGNORES THE MINIMUM CHARGE, which dominates small datasets — a real job metering at \
         $0.08 was billed $4.00. Use `job estimate` on an uploaded file for the real number."
            .to_string(),
    ];
    if price.is_none() {
        caveats.push(format!(
            "no published LoRA price band covers a {params_b}B model — \
             frontier architectures are priced per-model with minimums, so no figure is given"
        ));
    }
    caveats.push(
        "whether this base is accepted for fine-tuning is a separate question from its price band"
            .to_string(),
    );

    Estimate {
        dataset_tokens,
        training_tokens,
        epochs: epochs.max(1),
        price_per_mtok_usd: price,
        training_usd: price.map(|p| (training_tokens as f64 / 1_000_000.0) * p),
        caveats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_up_so_a_short_dataset_is_never_free() {
        assert_eq!(tokens_from_chars(0), 0);
        assert_eq!(tokens_from_chars(1), 1);
        assert_eq!(tokens_from_chars(4), 1);
        assert_eq!(tokens_from_chars(5), 2);
    }

    #[test]
    fn training_tokens_scale_with_epochs() {
        let e = together_lora(4_000_000, 3, 27.0);
        assert_eq!(e.dataset_tokens, 1_000_000);
        assert_eq!(e.training_tokens, 3_000_000);
    }

    #[test]
    fn qwen_27b_prices_in_the_middle_band() {
        let e = together_lora(4_000_000, 3, 27.0);
        assert_eq!(e.price_per_mtok_usd, Some(1.50));
        let usd = e.training_usd.expect("priced");
        assert!(
            (usd - 4.50).abs() < 1e-9,
            "3 Mtok at $1.50/Mtok = $4.50, got {usd}"
        );
    }

    /// The estimate must never quietly imply it covers running the thing afterwards.
    #[test]
    fn caveats_always_name_hosting_and_the_token_heuristic() {
        let e = together_lora(1000, 1, 27.0);
        let joined = e.caveats.join(" ");
        assert!(joined.contains("hosting"), "got {joined}");
        assert!(joined.contains("approximate"), "got {joined}");
    }

    #[test]
    fn an_unpriced_band_gives_no_figure_and_says_why() {
        let e = together_lora(4_000_000, 3, 400.0);
        assert_eq!(e.training_usd, None, "must refuse rather than guess");
        assert!(e
            .caveats
            .iter()
            .any(|c| c.contains("no published LoRA price band")));
    }

    #[test]
    fn zero_epochs_is_treated_as_one_pass_not_as_free() {
        let e = together_lora(4_000_000, 0, 27.0);
        assert_eq!(e.epochs, 1);
        assert_eq!(e.training_tokens, e.dataset_tokens);
    }
}
