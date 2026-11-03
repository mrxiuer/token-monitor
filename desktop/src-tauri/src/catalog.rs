use crate::ledger::Price;
use serde_json::Value;
use std::collections::HashMap;

pub const URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
pub fn bundled() -> HashMap<String, Price> {
    let value: Value = serde_json::from_str(include_str!("../../pricing-catalog.json"))
        .expect("checked bundled pricing catalog");
    parse(&value)
}
pub fn parse(value: &Value) -> HashMap<String, Price> {
    let mut result = HashMap::new();
    let Some(models) = value.get("rates").unwrap_or(value).as_object() else {
        return result;
    };
    for (model, raw) in models {
        let rate = |field: &str| {
            raw[field]
                .as_f64()
                .filter(|rate| rate.is_finite() && *rate >= 0.0)
        };
        let price = if value.get("rates").is_some() {
            Price {
                input: rate("input"),
                output: rate("output"),
                cache: rate("cache"),
            }
        } else {
            if raw["litellm_provider"] != "openai" {
                continue;
            }
            Price {
                input: rate("input_cost_per_token"),
                output: rate("output_cost_per_token"),
                cache: rate("cache_read_input_token_cost"),
            }
        };
        if price.input.is_none() && price.output.is_none() && price.cache.is_none() {
            continue;
        }
        result.insert(
            model
                .strip_prefix("openai/")
                .unwrap_or(model)
                .to_lowercase(),
            price,
        );
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_catalog_has_known_gpt_prices_and_no_guessed_unknown_model() {
        let catalog = bundled();
        let price = catalog.get("gpt-5").unwrap();
        assert_eq!(price.input, Some(1.25e-6));
        assert!(!catalog.contains_key("unknown"));
    }
    #[test]
    fn invalid_rates_and_other_providers_are_excluded() {
        let catalog = parse(
            &serde_json::json!({"x":{"litellm_provider":"other","input_cost_per_token":1},"openai/gpt-test":{"litellm_provider":"openai","input_cost_per_token":-1,"output_cost_per_token":0.01}}),
        );
        assert!(!catalog.contains_key("x"));
        assert!(catalog["gpt-test"].input.is_none());
        assert_eq!(catalog["gpt-test"].output, Some(0.01));
    }
}
