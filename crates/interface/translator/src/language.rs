use std::borrow::Cow;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Chinese,
    ChineseTraditional,
    English,
    Japanese,
}

impl Language {
    pub fn all() -> [Self; 4] {
        [
            Self::Chinese,
            Self::ChineseTraditional,
            Self::English,
            Self::Japanese,
        ]
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "chs" | "zh" | "zho" | "chi" | "chinese" | "chinese simplified" | "简体中文" => {
                Some(Self::Chinese)
            }
            "cht" | "chinese traditional" | "繁体中文" => Some(Self::ChineseTraditional),
            "en" | "eng" | "english" | "英文" => Some(Self::English),
            "ja" | "jp" | "jpn" | "japanese" | "日文" | "日语" => Some(Self::Japanese),
            _ => None,
        }
    }

    pub fn from_639_1(s: &str) -> Option<Self> {
        match s {
            "zh" => Some(Self::Chinese),
            "en" => Some(Self::English),
            "ja" => Some(Self::Japanese),
            _ => None,
        }
    }

    pub fn from_639_3(s: &str) -> Option<Self> {
        match s {
            "zho" | "chi" => Some(Self::Chinese),
            "eng" => Some(Self::English),
            "jpn" => Some(Self::Japanese),
            _ => None,
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::parse(s)
    }

    pub fn to_639_1(self) -> Option<&'static str> {
        match self {
            Self::Chinese => Some("zh"),
            Self::ChineseTraditional => None,
            Self::English => Some("en"),
            Self::Japanese => Some("ja"),
        }
    }

    pub fn to_639_3(self) -> Option<&'static str> {
        match self {
            Self::Chinese | Self::ChineseTraditional => Some("zho"),
            Self::English => Some("eng"),
            Self::Japanese => Some("jpn"),
        }
    }

    pub fn to_name(self) -> Option<&'static str> {
        Some(self.name())
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Chinese => "Chinese",
            Self::ChineseTraditional => "Chinese Traditional",
            Self::English => "English",
            Self::Japanese => "Japanese",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Chinese => "chs",
            Self::ChineseTraditional => "cht",
            Self::English => "en",
            Self::Japanese => "ja",
        }
    }

    pub fn display_zh(self) -> &'static str {
        match self {
            Self::Chinese => "简体中文",
            Self::ChineseTraditional => "繁体中文",
            Self::English => "英文",
            Self::Japanese => "日文",
        }
    }
}

impl Default for Language {
    fn default() -> Self {
        Self::Chinese
    }
}

#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug)]
pub struct LanguageWrapper(pub Language);

impl JsonSchema for LanguageWrapper {
    fn schema_name() -> Cow<'static, str> {
        "LanguageWrapper".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::", "LanguageWrapper").into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let mut map = serde_json::Map::new();
        map.insert(
            "oneOf".into(),
            Value::Array(
                [
                    ("chs", "简体中文"),
                    ("cht", "繁体中文"),
                    ("en", "英文"),
                    ("ja", "日文"),
                ]
                .into_iter()
                .map(|(code, title)| {
                    let mut s = serde_json::Map::new();
                    s.insert("const".into(), Value::String(code.into()));
                    s.insert("title".into(), Value::String(title.into()));
                    Value::Object(s)
                })
                .collect(),
            ),
        );
        schemars::Schema::from(map)
    }
}

impl<'de> Deserialize<'de> for LanguageWrapper {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Language::parse(&value)
            .map(LanguageWrapper)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid lang code: \"{value}\"")))
    }
}

impl Serialize for LanguageWrapper {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.code())
    }
}

pub trait Detector {
    fn detect_language(&self, text: &str) -> Option<Language>;
}

#[derive(Clone, Default)]
pub struct LangIdDetector;

impl LangIdDetector {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self)
    }
}

impl Detector for LangIdDetector {
    fn detect_language(&self, text: &str) -> Option<Language> {
        let mut kana = 0usize;
        let mut han = 0usize;
        let mut latin = 0usize;
        let mut traditional = 0usize;
        for ch in text.chars() {
            match ch {
                '\u{3040}'..='\u{30FF}' => kana += 1,
                '\u{4E00}'..='\u{9FFF}' => {
                    han += 1;
                    if "發國語東門關來對時會學經體點".contains(ch) {
                        traditional += 1;
                    }
                }
                'A'..='Z' | 'a'..='z' => latin += 1,
                _ => {}
            }
        }
        if kana > 0 {
            Some(Language::Japanese)
        } else if han > 0 {
            Some(if traditional > 0 {
                Language::ChineseTraditional
            } else {
                Language::Chinese
            })
        } else if latin > 0 {
            Some(Language::English)
        } else {
            None
        }
    }
}

pub fn is_valuable_text(text: &str) -> bool {
    text.chars().any(|ch| {
        ch.is_alphabetic()
            || ('\u{3040}'..='\u{30FF}').contains(&ch)
            || ('\u{4E00}'..='\u{9FFF}').contains(&ch)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codes() {
        assert_eq!(Language::parse("chs"), Some(Language::Chinese));
        assert_eq!(Language::parse("CHT"), Some(Language::ChineseTraditional));
        assert_eq!(Language::parse("en"), Some(Language::English));
        assert_eq!(Language::parse("日文"), Some(Language::Japanese));
        assert_eq!(Language::parse("xx"), None);
    }

    #[test]
    fn wrapper_roundtrip() {
        let raw = serde_json::to_string(&LanguageWrapper(Language::Chinese)).unwrap();
        assert_eq!(raw, "\"chs\"");
        let back: LanguageWrapper = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.0, Language::Chinese);
    }

    #[test]
    fn detect_kana_is_japanese() {
        let d = LangIdDetector;
        assert_eq!(d.detect_language("こんにちは"), Some(Language::Japanese));
        assert_eq!(d.detect_language("hello"), Some(Language::English));
    }

    #[test]
    fn valuable_text() {
        assert!(is_valuable_text("あ"));
        assert!(is_valuable_text("Hi"));
        assert!(!is_valuable_text("..."));
        assert!(!is_valuable_text("123"));
    }
}
