//! PixAI tagger v0.9 (frozen wd-eva02-large encoder + linear head): Danbooru-style tags for a whole
//! page, which the LLM translator receives as its `<tags>` context. The ONNX and `tags.json` come from
//! `scripts/pixai_tagger_export.py`.
use std::{
    collections::{BTreeSet, HashMap},
    fmt,
    ops::Deref,
    sync::{Arc, Mutex},
};

use anyhow::ensure;
use base_util::onnx::{new_session_mlprogram, Providers};
use interface_image::{DimType, ImageOp, Interpolation, RawImage};
use interface_model::{
    impl_model_helpers, impl_model_load_helpers, Model, ModelLoad, ModelRead, ModelSource,
    ModelWrap,
};
use maplit::hashmap;
use ort::{inputs, session::Session, value::Tensor};
use serde::Deserialize;

const INPUT_SIZE: DimType = 448;

/// Column layout of the model's `probs` output.
#[derive(Deserialize)]
struct TagTable {
    /// `names[..gen_tag_count]` are general tags, the rest character tags.
    gen_tag_count: usize,
    names: Vec<String>,
    /// Franchises of each character tag, aligned with `names[gen_tag_count..]`.
    ips: Vec<Vec<String>>,
}

impl TagTable {
    fn select(&self, probs: impl Iterator<Item = f32>, options: TaggerOptions) -> Tags {
        let mut general = Vec::new();
        let mut character = Vec::new();
        for (i, p) in probs.enumerate() {
            if i < self.gen_tag_count {
                if p > options.general_threshold {
                    general.push((p, i));
                }
            } else if p > options.character_threshold {
                character.push((p, i));
            }
        }
        general.sort_by(|a, b| b.0.total_cmp(&a.0));
        character.sort_by(|a, b| b.0.total_cmp(&a.0));
        let ip: BTreeSet<&str> = character
            .iter()
            .flat_map(|&(_, i)| &self.ips[i - self.gen_tag_count])
            .map(String::as_str)
            .collect();
        let names = |picked: Vec<(f32, usize)>| {
            picked
                .into_iter()
                .map(|(_, i)| self.names[i].clone())
                .collect()
        };
        Tags {
            general: names(general),
            character: names(character),
            ip: ip.into_iter().map(str::to_owned).collect(),
        }
    }
}

pub struct Loaded {
    session: Mutex<Session>,
    tags: TagTable,
}

/// Probability cut-offs, defaults from the model's handler.py.
#[derive(Clone, Copy, Debug)]
pub struct TaggerOptions {
    pub general_threshold: f32,
    pub character_threshold: f32,
}

impl Default for TaggerOptions {
    fn default() -> Self {
        Self {
            general_threshold: 0.3,
            character_threshold: 0.85,
        }
    }
}

/// Tags above threshold, most confident first. `ip` are the franchises of the characters found,
/// sorted and deduplicated.
#[derive(Debug, Default, PartialEq)]
pub struct Tags {
    pub general: Vec<String>,
    pub character: Vec<String>,
    pub ip: Vec<String>,
}

impl fmt::Display for Tags {
    /// One `group: a, b` line per non-empty group: the body of the translator's `<tags>` block.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let groups = [
            ("general", &self.general),
            ("character", &self.character),
            ("ip", &self.ip),
        ];
        let mut lines = groups.iter().filter(|(_, tags)| !tags.is_empty());
        if let Some((label, tags)) = lines.next() {
            write!(f, "{label}: {}", tags.join(", "))?;
        }
        for (label, tags) in lines {
            write!(f, "\n{label}: {}", tags.join(", "))?;
        }
        Ok(())
    }
}

pub struct PixaiTagger {
    providers: Arc<Vec<Providers>>,
    model: ModelWrap<Loaded>,
}

impl PixaiTagger {
    pub fn new(providers: Arc<Vec<Providers>>) -> Self {
        Self {
            providers,
            model: Default::default(),
        }
    }

    pub async fn tag(
        &self,
        image: &RawImage,
        options: TaggerOptions,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<Tags> {
        let loaded = self.load().await?;
        let loaded = loaded.deref();
        // handler.py: PIL bilinear resize to 448x448 (antialiased, hence BilinearExact), then
        // ToTensor + Normalize(0.5, 0.5), i.e. x / 127.5 - 1.
        let resized = img_processor.resize(
            image.view(),
            INPUT_SIZE,
            INPUT_SIZE,
            Interpolation::BilinearExact,
        )?;
        let input =
            img_processor.substract_mean_normalize(&resized, &[127.5; 3], &[1.0 / 127.5; 3]);
        // Synchronous `Session::run` on purpose; see `dbnet` for why `RunAsync` is avoided.
        let mut session = loaded.session.lock().expect("session mutex poisoned");
        let outputs = session.run(inputs!["input" => Tensor::from_array(input)?])?;
        let probs = outputs["probs"].try_extract_array::<f32>()?;
        Ok(loaded.tags.select(probs.iter().copied(), options))
    }
}

#[async_trait::async_trait]
impl ModelLoad for PixaiTagger {
    impl_model_load_helpers!(model, Loaded);

    async fn reload(&self) -> anyhow::Result<ModelRead<'_, Self::T>> {
        let model = self.download_model("model", "model.onnx").await?;
        let tags = self.download_model("tags", "tags.json").await?;
        let tags: TagTable = serde_json::from_slice(&std::fs::read(tags)?)?;
        ensure!(
            tags.names.len() == tags.gen_tag_count + tags.ips.len(),
            "tags.json: {} names but {} general + {} character",
            tags.names.len(),
            tags.gen_tag_count,
            tags.ips.len()
        );
        // MLProgram runs the whole graph as one CoreML partition once the export rewrites RoPE's Neg
        // and pre-transposes the Gemm weights (scripts/pixai_tagger_export.py): 106 ms per page on an
        // M3 Max, 0.7 s session load from the compiled-model cache. The CPU EP takes 826 ms per page
        // and the NeuralNetwork format 1.6 s (its builder rejects the attention MatMuls, 171
        // partitions). All three produce the same tags.
        let session = new_session_mlprogram(&self.providers)?.commit_from_file(&model)?;
        *self.model.write().await = Some(Loaded {
            session: Mutex::new(session),
            tags,
        });
        Ok(self.get_model().await.expect("set before"))
    }
}

impl Model for PixaiTagger {
    impl_model_helpers!("tagger", "pixai", model);

    fn models(&self) -> HashMap<&'static str, ModelSource> {
        hashmap! {
            "model" => ModelSource {
                url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/pixai-tagger-v0.9/model.onnx",
                hash: "###",
            },
            "tags" => ModelSource {
                url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/pixai-tagger-v0.9/tags.json",
                hash: "###",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use base_util::onnx::all_providers;
    use interface_image::CpuImageProcessor;

    use super::*;

    fn table() -> TagTable {
        TagTable {
            gen_tag_count: 2,
            names: ["1girl", "smile", "mika_(blue_archive)", "reed_(arknights)"]
                .map(str::to_owned)
                .to_vec(),
            ips: vec![vec!["blue_archive".into()], vec!["arknights".into()]],
        }
    }

    #[test]
    fn select_thresholds_orders_and_maps_ips() {
        let tags = table().select([0.4, 0.9, 0.86, 0.5].into_iter(), TaggerOptions::default());
        assert_eq!(tags.general, ["smile", "1girl"]);
        assert_eq!(tags.character, ["mika_(blue_archive)"]);
        assert_eq!(tags.ip, ["blue_archive"]);
        assert_eq!(
            tags.to_string(),
            "general: smile, 1girl\ncharacter: mika_(blue_archive)\nip: blue_archive"
        );
    }

    #[test]
    fn display_skips_empty_groups() {
        let tags = table().select([0.1, 0.5, 0.2, 0.3].into_iter(), TaggerOptions::default());
        assert_eq!(tags.to_string(), "general: smile");
        assert_eq!(Tags::default().to_string(), "");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tags_manga_page() {
        let img = RawImage::new("./imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png")
            .expect("Failed to load image");
        let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
        let tagger = PixaiTagger::new(Arc::new(all_providers()));
        let tags = tagger
            .tag(&img, TaggerOptions::default(), &ip)
            .await
            .expect("tag");
        // PyTorch reference (scripts/pixai_tagger_export.py --check): 34 general tags led by
        // comic 0.99, 2girls 0.95, hoodie 0.92; no character above 0.85.
        assert_eq!(tags.general[0], "comic");
        assert!(tags.general.iter().any(|t| t == "2girls"), "{tags}");
        assert!(tags.general.iter().any(|t| t == "hoodie"), "{tags}");
        assert!((30..=38).contains(&tags.general.len()), "{tags}");
        assert!(tags.character.is_empty(), "{tags}");
    }
}
