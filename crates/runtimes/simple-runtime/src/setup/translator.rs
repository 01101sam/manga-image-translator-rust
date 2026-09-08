use std::{collections::HashMap, env};

use interface_translator::{AsyncTranslator, ComputeType};

use crate::settings::Translator;
pub type TranslatorType = Box<dyn AsyncTranslator + Send + Sync>;

pub struct Translators {
    items: HashMap<Translator, TranslatorType>,
    cuda: bool,
}

async fn create_papago() -> Option<TranslatorType> {
    match tokio::spawn(interface_translator::PapagoTranslator::new(false)).await {
        Ok(Ok(t)) => Some(Box::new(t) as TranslatorType),
        Ok(Err(err)) => {
            warn!("Papago init failed: {err}");
            None
        }
        Err(err) => {
            warn!("Papago init panicked: {err}");
            None
        }
    }
}

use interface_translator::{
    BaiduTranslator, CaiyunTranslator, DeeplTranslator, GoogleTranslator, YoudaoTranslator,
};
use log::warn;

pub fn create_baidu_translator() -> Option<TranslatorType> {
    let app_id = env::var("BAIDU_APP_ID").ok();
    let secret_key = env::var("BAIDU_SECRET_KEY").ok();

    match (&app_id, &secret_key) {
        (Some(app_id), Some(secret_key)) => {
            Some(Box::new(BaiduTranslator::new(&app_id, &secret_key)) as TranslatorType)
        }
        _ => {
            if app_id.is_none() {
                warn!("BAIDU_APP_ID not set");
            }
            if secret_key.is_none() {
                warn!("BAIDU_SECRET_KEY not set");
            }
            None
        }
    }
}

pub fn create_caiyun_translator() -> Option<TranslatorType> {
    match env::var("CAIYUN_TOKEN") {
        Ok(token) => Some(Box::new(CaiyunTranslator::new(
            token,
            "manga-image-translator".to_string(),
        ))),
        Err(_) => {
            warn!("CAIYUN_TOKEN not set");
            None
        }
    }
}

pub fn create_deepl_translator() -> Option<TranslatorType> {
    match env::var("DEEPL_AUTH_KEY") {
        Ok(key) => Some(Box::new(DeeplTranslator::new(key))),
        Err(_) => {
            warn!("DEEPL_AUTH_KEY not set");
            None
        }
    }
}

pub fn create_google_translator() -> Option<TranslatorType> {
    match env::var("GOOGLE_API_KEY") {
        Ok(key) => Some(Box::new(GoogleTranslator::new(key))),
        Err(_) => {
            warn!("GOOGLE_API_KEY not set");
            None
        }
    }
}

pub fn create_youdao_translator() -> Option<TranslatorType> {
    let app_key = env::var("YOUDAO_APP_KEY").ok();
    let secret_key = env::var("YOUDAO_SECRET_KEY").ok();

    match (&app_key, &secret_key) {
        (Some(app_key), Some(secret_key)) => Some(Box::new(YoudaoTranslator::new(
            app_key.to_owned(),
            secret_key.to_owned(),
        )) as TranslatorType),
        _ => {
            if app_key.is_none() {
                warn!("YOUDAO_APP_KEY not set");
            }
            if secret_key.is_none() {
                warn!("YOUDAO_SECRET_KEY not set");
            }
            None
        }
    }
}
async fn create(key: Translator, cuda: bool) -> Option<TranslatorType> {
    match key {
        Translator::JParaCrawlSmall => Some(Box::new(
            interface_translator::JParaCrawlTranslator::new(
                false,
                cuda,
                ComputeType::DEFAULT,
                interface_translator::JParaCrawlSize::Small,
            ),
        ) as TranslatorType),
        Translator::JParaCrawlBase => Some(Box::new(
            interface_translator::JParaCrawlTranslator::new(
                false,
                cuda,
                ComputeType::DEFAULT,
                interface_translator::JParaCrawlSize::Base,
            ),
        ) as TranslatorType),
        Translator::JParaCrawlLarge => Some(Box::new(
            interface_translator::JParaCrawlTranslator::new(
                false,
                cuda,
                ComputeType::DEFAULT,
                interface_translator::JParaCrawlSize::Large,
            ),
        ) as TranslatorType),
        Translator::Baidu => create_baidu_translator(),
        Translator::Caiyun => create_caiyun_translator(),
        Translator::Deepl => create_deepl_translator(),
        Translator::Google => create_google_translator(),
        Translator::M2M100Small => Some(Box::new(interface_translator::M2M100Translator::new(
            cuda,
            ComputeType::DEFAULT,
            interface_translator::M2M100Size::Small,
        )) as TranslatorType),
        Translator::M2M100Large => Some(Box::new(interface_translator::M2M100Translator::new(
            cuda,
            ComputeType::DEFAULT,
            interface_translator::M2M100Size::Large,
        )) as TranslatorType),
        Translator::MyMemory => {
            Some(Box::new(interface_translator::MyMemoryTranslator::new()) as TranslatorType)
        }
        Translator::NLLBSmallDistilled => Some(Box::new(interface_translator::NLLBTranslator::new(
            cuda,
            ComputeType::DEFAULT,
            interface_translator::NLLBSize::SmallDistilled,
        )) as TranslatorType),
        Translator::NLLBBase => Some(Box::new(interface_translator::NLLBTranslator::new(
            cuda,
            ComputeType::DEFAULT,
            interface_translator::NLLBSize::Base,
        )) as TranslatorType),
        Translator::NLLBLarge => Some(Box::new(interface_translator::NLLBTranslator::new(
            cuda,
            ComputeType::DEFAULT,
            interface_translator::NLLBSize::Large,
        )) as TranslatorType),
        Translator::Papago => create_papago().await,
        Translator::Sugoi => Some(Box::new(interface_translator::SugoiTranslator::new(
            cuda,
            ComputeType::DEFAULT,
        )) as TranslatorType),
        Translator::Youdao => create_youdao_translator(),
        Translator::None | Translator::Original => None,
        Translator::MBart => Some(Box::new(interface_translator::MBart50Translator::new(
            cuda,
            ComputeType::DEFAULT,
        )) as TranslatorType),
    }
}

impl Translators {
    pub fn new(cuda: bool) -> Self {
        Self {
            items: HashMap::new(),
            cuda,
        }
    }

    pub async fn get(&mut self, translator: Translator) -> anyhow::Result<&mut TranslatorType> {
        if !self.items.contains_key(&translator) {
            let created = create(translator, self.cuda)
                .await
                .ok_or_else(|| anyhow::anyhow!("{translator:?} not available"))?;
            self.items.insert(translator, created);
        }
        Ok(self.items.get_mut(&translator).expect("inserted"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_does_not_construct_translators() {
        let t = Translators::new(false);
        assert!(t.items.is_empty());
    }
}
