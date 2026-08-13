// SPDX-License-Identifier: Apache-2.0
//! Built-in list price table, USD per million tokens.
//!
//! Used to turn a token-count discrepancy into the number a buyer actually
//! cares about: how much extra they were charged. Matching is by longest
//! substring so `claude-opus-4-5-20251101` and `anthropic/claude-opus-4.5`
//! both resolve, while an unknown model resolves to `None` — the audit then
//! reports the token ratio and explicitly says no price was applied, rather
//! than inventing a dollar figure.

#[derive(Debug, Clone, Copy)]
pub struct Price {
    pub family: &'static str,
    /// USD per million input tokens.
    pub input: f64,
    /// USD per million output tokens.
    pub output: f64,
}

/// Longest pattern wins, so `gpt-4o-mini` never matches the `gpt-4o` row.
const TABLE: &[(&str, Price)] = &[
    // Anthropic
    (
        "claude-opus",
        Price {
            family: "Claude Opus",
            input: 15.0,
            output: 75.0,
        },
    ),
    (
        "claude-sonnet",
        Price {
            family: "Claude Sonnet",
            input: 3.0,
            output: 15.0,
        },
    ),
    (
        "claude-haiku",
        Price {
            family: "Claude Haiku",
            input: 0.80,
            output: 4.0,
        },
    ),
    (
        "claude-3-5-sonnet",
        Price {
            family: "Claude 3.5 Sonnet",
            input: 3.0,
            output: 15.0,
        },
    ),
    (
        "claude-3-5-haiku",
        Price {
            family: "Claude 3.5 Haiku",
            input: 0.80,
            output: 4.0,
        },
    ),
    (
        "claude-3-opus",
        Price {
            family: "Claude 3 Opus",
            input: 15.0,
            output: 75.0,
        },
    ),
    // OpenAI
    (
        "gpt-4o-mini",
        Price {
            family: "GPT-4o mini",
            input: 0.15,
            output: 0.60,
        },
    ),
    (
        "gpt-4o",
        Price {
            family: "GPT-4o",
            input: 2.50,
            output: 10.0,
        },
    ),
    (
        "gpt-4.1-mini",
        Price {
            family: "GPT-4.1 mini",
            input: 0.40,
            output: 1.60,
        },
    ),
    (
        "gpt-4.1-nano",
        Price {
            family: "GPT-4.1 nano",
            input: 0.10,
            output: 0.40,
        },
    ),
    (
        "gpt-4.1",
        Price {
            family: "GPT-4.1",
            input: 2.0,
            output: 8.0,
        },
    ),
    (
        "gpt-4-turbo",
        Price {
            family: "GPT-4 Turbo",
            input: 10.0,
            output: 30.0,
        },
    ),
    (
        "gpt-3.5-turbo",
        Price {
            family: "GPT-3.5 Turbo",
            input: 0.50,
            output: 1.50,
        },
    ),
    (
        "o3-mini",
        Price {
            family: "o3-mini",
            input: 1.10,
            output: 4.40,
        },
    ),
    (
        "o4-mini",
        Price {
            family: "o4-mini",
            input: 1.10,
            output: 4.40,
        },
    ),
    (
        "o3",
        Price {
            family: "o3",
            input: 2.0,
            output: 8.0,
        },
    ),
    (
        "o1-mini",
        Price {
            family: "o1-mini",
            input: 1.10,
            output: 4.40,
        },
    ),
    (
        "o1",
        Price {
            family: "o1",
            input: 15.0,
            output: 60.0,
        },
    ),
    // Google
    (
        "gemini-2.5-pro",
        Price {
            family: "Gemini 2.5 Pro",
            input: 1.25,
            output: 10.0,
        },
    ),
    (
        "gemini-2.5-flash",
        Price {
            family: "Gemini 2.5 Flash",
            input: 0.30,
            output: 2.50,
        },
    ),
    (
        "gemini-2.0-flash",
        Price {
            family: "Gemini 2.0 Flash",
            input: 0.10,
            output: 0.40,
        },
    ),
    (
        "gemini-1.5-pro",
        Price {
            family: "Gemini 1.5 Pro",
            input: 1.25,
            output: 5.0,
        },
    ),
    // DeepSeek / Moonshot / Zhipu / Alibaba
    (
        "deepseek-reasoner",
        Price {
            family: "DeepSeek Reasoner",
            input: 0.55,
            output: 2.19,
        },
    ),
    (
        "deepseek-chat",
        Price {
            family: "DeepSeek Chat",
            input: 0.27,
            output: 1.10,
        },
    ),
    (
        "deepseek",
        Price {
            family: "DeepSeek",
            input: 0.27,
            output: 1.10,
        },
    ),
    (
        "kimi",
        Price {
            family: "Kimi",
            input: 0.60,
            output: 2.50,
        },
    ),
    (
        "moonshot",
        Price {
            family: "Moonshot",
            input: 0.60,
            output: 2.50,
        },
    ),
    (
        "glm-4",
        Price {
            family: "GLM-4",
            input: 0.60,
            output: 0.60,
        },
    ),
    (
        "glm",
        Price {
            family: "GLM",
            input: 0.60,
            output: 0.60,
        },
    ),
    (
        "qwen-max",
        Price {
            family: "Qwen Max",
            input: 1.60,
            output: 6.40,
        },
    ),
    (
        "qwen",
        Price {
            family: "Qwen",
            input: 0.40,
            output: 1.20,
        },
    ),
];

/// List price for a model ID, or `None` when we have no entry.
pub fn lookup(model: &str) -> Option<Price> {
    let m = model.trim().to_ascii_lowercase().replace('.', "-");
    let mut best: Option<(usize, Price)> = None;
    for (pat, price) in TABLE {
        let pat_norm = pat.replace('.', "-");
        if m.contains(&pat_norm) {
            let len = pat_norm.len();
            if best.map(|(l, _)| len > l).unwrap_or(true) {
                best = Some((len, *price));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// USD for a given token split. Callers must have a `Price` in hand, which is
/// only obtainable from `lookup`, so there is no path to a fabricated cost.
pub fn cost(price: &Price, input_tokens: u32, output_tokens: u32) -> f64 {
    (input_tokens as f64 * price.input + output_tokens as f64 * price.output) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_pattern_wins() {
        // The killer case: gpt-4o-mini must not be priced as gpt-4o.
        assert_eq!(lookup("gpt-4o-mini").unwrap().family, "GPT-4o mini");
        assert_eq!(lookup("gpt-4o").unwrap().family, "GPT-4o");
        assert_eq!(lookup("o3-mini").unwrap().family, "o3-mini");
        assert_eq!(
            lookup("deepseek-reasoner").unwrap().family,
            "DeepSeek Reasoner"
        );
    }

    #[test]
    fn resolves_dated_and_prefixed_ids() {
        assert_eq!(
            lookup("claude-opus-4-5-20251101").unwrap().family,
            "Claude Opus"
        );
        assert_eq!(
            lookup("anthropic/claude-sonnet-4.5").unwrap().family,
            "Claude Sonnet"
        );
        // Dot and dash version styles must land on the same row.
        assert_eq!(lookup("gpt-4.1-mini").unwrap().family, "GPT-4.1 mini");
        assert_eq!(lookup("gpt-4-1-mini").unwrap().family, "GPT-4.1 mini");
    }

    #[test]
    fn unknown_models_yield_no_price_rather_than_a_guess() {
        assert!(lookup("some-private-finetune-v3").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn cost_is_per_million_tokens() {
        let p = Price {
            family: "t",
            input: 3.0,
            output: 15.0,
        };
        // 1M input + 1M output at 3 + 15.
        assert!((cost(&p, 1_000_000, 1_000_000) - 18.0).abs() < 1e-9);
        assert_eq!(cost(&p, 0, 0), 0.0);
        // A realistic small call stays in the sub-cent range.
        assert!(cost(&p, 1000, 500) < 0.02);
    }
}
