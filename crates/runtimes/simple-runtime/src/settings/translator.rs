use interface_translator::{Backend, Language, LanguageWrapper, ThinkingStrength};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Copy, Clone, JsonSchema, Debug, PartialEq, Eq)]
pub enum TranslatorMode {
    #[default]
    Llm,
    Original,
    None,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TranslatorSettings {
    pub mode: TranslatorMode,
    pub backend: Backend,
    pub base_url: Option<String>,
    pub model: String,
    pub thinking: bool,
    pub thinking_strength: ThinkingStrength,
    pub web_search: bool,
    pub target: LanguageWrapper,
    pub filter_lang: Vec<LanguageWrapper>,
    pub pre_dict: Option<String>,
    pub post_dict: Option<String>,
}

impl Default for TranslatorSettings {
    fn default() -> Self {
        Self {
            mode: TranslatorMode::Llm,
            backend: Backend::OpenAi,
            base_url: None,
            model: "deepseek-v4-flash".into(),
            thinking: true,
            thinking_strength: ThinkingStrength::High,
            web_search: false,
            target: LanguageWrapper(Language::Chinese),
            filter_lang: vec![],
            pre_dict: None,
            post_dict: None,
        }
    }
}
