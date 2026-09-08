use std::env;

use anyhow::Context;
use interface_translator::{LlmConfig, LlmTranslator};
use log::warn;

use crate::settings::TranslatorSettings;

pub fn create_translator(settings: &TranslatorSettings) -> anyhow::Result<LlmTranslator> {
    let api_key = env::var("DEEPSEEK_API_KEY")
        .or_else(|_| match settings.backend {
            interface_translator::Backend::OpenAi => env::var("OPENAI_API_KEY"),
            interface_translator::Backend::Anthropic => env::var("ANTHROPIC_API_KEY"),
        })
        .context("DEEPSEEK_API_KEY not set")?;

    let base_url = settings
        .base_url
        .clone()
        .unwrap_or_else(|| match settings.backend {
            interface_translator::Backend::OpenAi => "https://api.deepseek.com".into(),
            interface_translator::Backend::Anthropic => "https://api.deepseek.com/anthropic".into(),
        });

    let web_search_key = if settings.web_search {
        match env::var("TAVILY_API_KEY") {
            Ok(k) => Some(k),
            Err(_) => {
                warn!("web_search enabled but TAVILY_API_KEY not set; tool omitted");
                None
            }
        }
    } else {
        None
    };

    Ok(LlmTranslator::from_config(LlmConfig {
        backend: settings.backend,
        base_url,
        api_key,
        model: settings.model.clone(),
        thinking: settings.thinking,
        thinking_strength: settings.thinking_strength,
        web_search_key,
        max_iters: 24,
    }))
}

#[cfg(test)]
mod tests {
    use crate::settings::TranslatorSettings;

    #[test]
    fn defaults_are_deepseek() {
        let s = TranslatorSettings::default();
        assert_eq!(s.model, "deepseek-v4-flash");
        assert!(s.thinking);
        assert!(!s.web_search);
    }
}
