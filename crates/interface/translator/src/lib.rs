mod agent;
mod backend;
mod language;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use language::{is_valuable_text, Detector, LangIdDetector, Language, LanguageWrapper};

use crate::agent::{run_agent, UreqTransport};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    #[default]
    #[serde(rename = "openai")]
    OpenAi,
    Anthropic,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingStrength {
    Low,
    #[default]
    High,
    Max,
}

impl ThinkingStrength {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::High => "high",
            Self::Max => "max",
        }
    }

    pub fn budget_tokens(self) -> u32 {
        match self {
            Self::Low => 2048,
            Self::High => 8192,
            Self::Max => 16384,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "high" => Some(Self::High),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

pub trait Transport: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value>;
}

#[derive(Clone)]
pub struct LlmConfig {
    pub backend: Backend,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub thinking: bool,
    pub thinking_strength: ThinkingStrength,
    pub web_search_key: Option<String>,
    pub max_iters: usize,
}

#[derive(Clone)]
pub struct LlmTranslator {
    pub(crate) backend: Backend,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) model: String,
    pub(crate) thinking: bool,
    pub(crate) thinking_strength: ThinkingStrength,
    pub(crate) web_search_key: Option<String>,
    pub(crate) max_iters: usize,
    pub(crate) transport: Arc<dyn Transport>,
}

impl LlmTranslator {
    pub fn from_config(config: LlmConfig) -> Self {
        Self {
            backend: config.backend,
            base_url: config.base_url,
            api_key: config.api_key,
            model: config.model,
            thinking: config.thinking,
            thinking_strength: config.thinking_strength,
            web_search_key: config.web_search_key,
            max_iters: if config.max_iters == 0 {
                24
            } else {
                config.max_iters
            },
            transport: Arc::new(UreqTransport),
        }
    }

    pub fn with_transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = transport;
        self
    }

    pub fn with_max_iters(mut self, max_iters: usize) -> Self {
        self.max_iters = max_iters;
        self
    }

    pub fn translate_blocking(
        &self,
        ocr_json: &str,
        tags: Option<&str>,
        target: Language,
    ) -> anyhow::Result<String> {
        run_agent(self, ocr_json, tags, target)
    }
}

#[async_trait::async_trait]
pub trait AsyncTranslator: Send + Sync {
    async fn translate(
        &self,
        ocr_json: &str,
        tags: Option<&str>,
        target: Language,
    ) -> anyhow::Result<String>;
}

#[async_trait::async_trait]
impl AsyncTranslator for LlmTranslator {
    async fn translate(
        &self,
        ocr_json: &str,
        tags: Option<&str>,
        target: Language,
    ) -> anyhow::Result<String> {
        let this = self.clone();
        let ocr_json = ocr_json.to_owned();
        let tags = tags.map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            this.translate_blocking(&ocr_json, tags.as_deref(), target)
        })
        .await?
    }
}

pub use agent::{system_prompt, tool_specs, user_message};
