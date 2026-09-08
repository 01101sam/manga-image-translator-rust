use anyhow::anyhow;
use interface_translator::Detector;
use interface_translator::TranslationListOutput;
use log::info;
use textline_merge::TextBlock;

use crate::{
    settings::{Target, Translation, Translator, TranslatorSettings},
    setup::Models,
};

fn apply_skip_translate(textblocks: &mut [TextBlock], translators: &[Translation]) -> bool {
    match translators.first().map(|t| t.translator) {
        Some(Translator::None) => true,
        Some(Translator::Original) => {
            for tb in textblocks {
                if tb.skip_translate {
                    continue;
                }
                let src = tb.text.clone();
                tb.translations.insert("original".into(), src);
                tb.translations
                    .insert("last_trans".into(), "original".into());
            }
            true
        }
        _ => false,
    }
}

impl Models {
    pub async fn run_translators(
        &mut self,
        textblocks: Vec<TextBlock>,
        config: &TranslatorSettings,
    ) -> anyhow::Result<Vec<TextBlock>> {
        match &config.target {
            Target::Single(items) => self.run_translator_list(textblocks, items.as_slice()).await,
            Target::Selective(hash_map) => todo!("selective not implemented yet"),
        }
    }

    pub async fn run_translator_list(
        &mut self,
        mut textblocks: Vec<TextBlock>,
        translators: &[Translation],
    ) -> anyhow::Result<Vec<TextBlock>> {
        assert!(!textblocks.is_empty());
        if apply_skip_translate(&mut textblocks, translators) {
            return Ok(textblocks);
        }
        let mut textblocks_use = textblocks
            .iter_mut()
            .filter(|v| !v.skip_translate)
            .collect::<Vec<_>>();

        let texts = textblocks_use
            .iter()
            .map(|v| v.text.clone())
            .collect::<Vec<_>>();
        for tb in &textblocks_use {
            assert!(tb.translations.is_empty());
        }

        let d_str = texts.join(" ");
        let lang = self.lang_detector.detect_language(&d_str);

        let mut texts = TranslationListOutput { text: texts, lang };

        for translator in translators {
            let out = self.run_translator_item(texts, translator).await?;
            let lang_str = out.lang.map(|v| v.to_name().unwrap()).unwrap_or("unknown");
            for (i, item) in out.text.iter().enumerate() {
                textblocks_use[i]
                    .translations
                    .insert(lang_str.to_owned(), item.to_owned());
            }
            texts = out;
        }
        let lang_str = texts
            .lang
            .map(|v| v.to_name().unwrap())
            .unwrap_or("unknown");

        for item in textblocks_use.iter_mut() {
            item.translations
                .insert("last_trans".to_owned(), lang_str.to_owned());
        }
        Ok(textblocks)
    }
    pub async fn run_translator_item(
        &mut self,
        input: TranslationListOutput,
        translator_info: &Translation,
    ) -> anyhow::Result<TranslationListOutput> {
        info!("Run Translator: {:?}", translator_info.translator);
        let to = translator_info.target.0;
        let translator = self.get_translator(translator_info.translator).await?;
        // TODO: set fallback language in config
        let from = input.lang.ok_or(anyhow!("Failed to detect language"))?;
        let t = translator
            .translate_vec(&input.text, None, Some(from), &to)
            .await?;
        let d_str = t.text.join(" ");
        let lang = self.lang_detector.detect_language(&d_str);

        Ok(TranslationListOutput { text: t.text, lang })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Translation, Translator};
    use interface_translator::{Language, LanguageWrapper};

    fn block(text: &str) -> TextBlock {
        serde_json::from_value(serde_json::json!({
            "lines": [],
            "text": text,
            "font_size": 12,
            "angle": 0.0,
            "prob": 1.0,
            "skip_translate": false,
            "translations": {}
        }))
        .unwrap()
    }

    fn one(translator: Translator) -> Translation {
        Translation {
            translator,
            target: LanguageWrapper(Language::English),
        }
    }

    #[test]
    fn none_leaves_no_translation() {
        let mut blocks = vec![block("あ")];
        assert!(apply_skip_translate(&mut blocks, &[one(Translator::None)]));
        assert!(blocks[0].translation().is_none());
    }

    #[test]
    fn original_keeps_source_text() {
        let mut blocks = vec![block("あ")];
        assert!(apply_skip_translate(
            &mut blocks,
            &[one(Translator::Original)]
        ));
        assert_eq!(blocks[0].translation(), Some("あ"));
    }
}
