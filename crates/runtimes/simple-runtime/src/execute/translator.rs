use anyhow::{anyhow, Context};
use interface_translator::AsyncTranslator;
use log::info;
use serde_json::Value;
use textline_merge::TextBlock;

use crate::{
    settings::{TranslatorMode, TranslatorSettings},
    setup::{create_translator, Models},
};

fn apply_skip_translate(textblocks: &mut [TextBlock], mode: TranslatorMode) -> bool {
    match mode {
        TranslatorMode::None => true,
        TranslatorMode::Original => {
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
        TranslatorMode::Llm => false,
    }
}

fn ocr_json(blocks: &[&mut TextBlock]) -> anyhow::Result<String> {
    let items: Vec<Value> = blocks
        .iter()
        .map(|tb| serde_json::json!({ "text": tb.text }))
        .collect();
    Ok(serde_json::to_string(&items)?)
}

fn parse_translated_texts(translated: &str) -> anyhow::Result<Vec<String>> {
    let v: Value = serde_json::from_str(translated.trim())
        .with_context(|| format!("finish translation is not JSON: {translated}"))?;
    let items = v
        .as_array()
        .ok_or_else(|| anyhow!("translation must be a JSON array"))?;
    items
        .iter()
        .map(|item| match item {
            Value::String(s) => Ok(s.clone()),
            Value::Object(m) => m
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("ocr item missing text")),
            _ => Err(anyhow!("invalid ocr item")),
        })
        .collect()
}

fn apply_translation(
    blocks: &mut [&mut TextBlock],
    translated: &str,
    lang: &str,
) -> anyhow::Result<()> {
    let texts = parse_translated_texts(translated)?;
    if texts.len() != blocks.len() {
        anyhow::bail!(
            "translation length mismatch: got {}, expected {}",
            texts.len(),
            blocks.len()
        );
    }
    for (tb, text) in blocks.iter_mut().zip(texts) {
        tb.translations.insert(lang.to_owned(), text);
        tb.translations.insert("last_trans".into(), lang.to_owned());
    }
    Ok(())
}

impl Models {
    pub async fn run_translators(
        &mut self,
        mut textblocks: Vec<TextBlock>,
        config: &TranslatorSettings,
        tags: Option<&str>,
    ) -> anyhow::Result<Vec<TextBlock>> {
        assert!(!textblocks.is_empty());
        if apply_skip_translate(&mut textblocks, config.mode) {
            return Ok(textblocks);
        }

        let mut used: Vec<&mut TextBlock> = textblocks
            .iter_mut()
            .filter(|v| !v.skip_translate)
            .collect();
        if used.is_empty() {
            return Ok(textblocks);
        }

        info!(
            "Run Translator: {:?} {} -> {}",
            config.backend,
            config.model,
            config.target.0.display_zh()
        );
        let payload = ocr_json(&used)?;
        let translator = create_translator(config)?;
        let translated = translator
            .translate(&payload, tags, config.target.0)
            .await?;
        apply_translation(&mut used, &translated, config.target.0.name())?;
        Ok(textblocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::TranslatorMode;
    use interface_translator::Language;

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

    #[test]
    fn none_leaves_no_translation() {
        let mut blocks = vec![block("あ")];
        assert!(apply_skip_translate(&mut blocks, TranslatorMode::None));
        assert!(blocks[0].translation().is_none());
    }

    #[test]
    fn original_keeps_source_text() {
        let mut blocks = vec![block("あ")];
        assert!(apply_skip_translate(&mut blocks, TranslatorMode::Original));
        assert_eq!(blocks[0].translation(), Some("あ"));
    }

    #[test]
    fn apply_matches_input_shape() {
        let mut a = block("a");
        let mut b = block("b");
        let mut refs = vec![&mut a, &mut b];
        apply_translation(
            &mut refs,
            r#"[{"text":"A"},{"text":"B"}]"#,
            Language::Chinese.name(),
        )
        .unwrap();
        assert_eq!(a.translation(), Some("A"));
        assert_eq!(b.translation(), Some("B"));
    }
}
