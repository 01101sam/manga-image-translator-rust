use std::{
    collections::{HashMap, HashSet},
    slice,
};

use interface_translator::{Language, LanguageWrapper};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;

#[derive(
    Serialize, Deserialize, Default, EnumIter, Hash, PartialEq, Eq, Copy, Clone, JsonSchema, Debug,
)]
pub enum Translator {
    JParaCrawlSmall,
    JParaCrawlBase,
    JParaCrawlLarge,
    Baidu,
    Caiyun,
    Deepl,
    Google,
    M2M100Small,
    M2M100Large,
    MBart,
    MyMemory,
    NLLBSmallDistilled,
    NLLBBase,
    NLLBLarge,
    Papago,
    #[default]
    Sugoi,
    Youdao,
    Original,
    None,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TranslatorSettings {
    pub target: Target,
    /// Filters out languages that should not be translated
    pub filter_lang: Vec<LanguageWrapper>,
    pub pre_dict: Option<String>,
    pub post_dict: Option<String>,
}

impl Default for TranslatorSettings {
    fn default() -> Self {
        Self {
            target: Target::Single(SingleOrMultiple::Single(Translation {
                translator: Translator::default(),
                target: LanguageWrapper(Language::English),
            })),
            filter_lang: vec![],
            pre_dict: None,
            post_dict: None,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Target {
    Single(SingleOrMultiple),
    Selective(HashMap<Option<LanguageWrapper>, SingleOrMultiple>),
}

impl Default for Target {
    fn default() -> Self {
        Target::Single(SingleOrMultiple::Single(Translation {
            translator: Translator::default(),
            target: LanguageWrapper(Language::English),
        }))
    }
}

impl Target {
    pub fn validate(&self) -> Option<&'static str> {
        match self {
            Target::Single(_) => None,
            Target::Selective(hash_map) => {
                if hash_map.get(&None).is_none() {
                    return Some("no default");
                };
                for mut key in hash_map.keys().cloned() {
                    let mut keys_used = HashSet::new();
                    loop {
                        let value = hash_map.get(&key);
                        let value = match value {
                            Some(v) => v,
                            None => return None,
                        };
                        let v = keys_used.insert(key);
                        if !v {
                            return Some("loop detected");
                        }
                        let next = match value {
                            SingleOrMultiple::Single(translation) => translation.target,
                            SingleOrMultiple::Multiple(translations) => {
                                if translations.is_empty() {
                                    return Some("empty array");
                                }
                                translations
                                    .last()
                                    .expect("translations should not be empty")
                                    .target
                            }
                        };
                        key = Some(next);
                    }
                }
                None
            }
        }
    }
}

#[derive(Hash, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SingleOrMultiple {
    Single(Translation),
    Multiple(Vec<Translation>),
}

impl SingleOrMultiple {
    pub fn as_slice(&self) -> &[Translation] {
        match self {
            SingleOrMultiple::Single(t) => slice::from_ref(t),
            SingleOrMultiple::Multiple(ts) => ts,
        }
    }
}

#[derive(Hash, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Translation {
    pub translator: Translator,
    pub target: LanguageWrapper,
}
