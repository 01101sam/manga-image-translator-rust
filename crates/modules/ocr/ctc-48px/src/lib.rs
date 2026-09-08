mod decode;

use std::{fs::read_to_string, ops::Deref as _, sync::Arc};

use base_util::onnx::{new_session, Providers};
use interface_detector::textlines::Quadrilateral;
use interface_image::{ImageOp, RawImage};
use interface_model::{
    impl_model_helpers, impl_model_load_helpers, Model, ModelLoad, ModelRead, ModelSource,
    ModelWrap,
};
use interface_ocr::{Ocr, OcrOptions, QuadrilateralInfo};
use maplit::hashmap;
use ort::session::Session;
use parking_lot::Mutex;
use util::{average::AvgMeter, ocr, spawn_blocking};

pub struct Ctc48pxOcr {
    model: ModelWrap<(std::sync::Mutex<Session>, Vec<String>)>,
    providers: Arc<Vec<Providers>>,
    max_batch_size: usize,
}

impl Ctc48pxOcr {
    pub fn new(providers: Arc<Vec<Providers>>, max_batch_size: usize) -> Self {
        Self {
            model: Default::default(),
            providers,
            max_batch_size,
        }
    }
}

#[async_trait::async_trait]
impl ModelLoad for Ctc48pxOcr {
    impl_model_load_helpers!(model, (std::sync::Mutex<Session>, Vec<String>));

    async fn reload(&self) -> anyhow::Result<ModelRead<'_, Self::T>> {
        let model = self.download_model("model", "model.onnx").await?;
        let dict = self
            .download_model("alphabet-all-v5", "alphabet-all-v5.txt")
            .await?;
        let dict = read_to_string(dict)
            .unwrap()
            .lines()
            .map(|v| v.trim_end().to_string())
            .collect::<Vec<String>>();
        let model = std::sync::Mutex::new(new_session(&self.providers)?.commit_from_file(&model)?);

        *self.model.write().await = Some((model, dict));
        let m = self.model.as_ref().read().await;
        let m = ModelRead::new(m);
        Ok(m)
    }
}

#[async_trait::async_trait]
impl Model for Ctc48pxOcr {
    impl_model_helpers!("ocr", "ctc-48px", model);

    fn models(&self) -> std::collections::HashMap<&'static str, interface_model::ModelSource> {
        hashmap! {
            "model" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/ctc-48px/model.onnx", hash: "###" },
            "alphabet-all-v5" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/ctc-48px/alphabet-all-v5.txt", hash: "###" }
        }
    }
}

fn post_process(
    texts: Vec<Vec<(i64, f32, f32, f32, f32, f32, f32, f32)>>,
    dict: &[String],
    areas: &[Arc<Mutex<Quadrilateral>>],
) -> Vec<QuadrilateralInfo> {
    let mut out = vec![];
    for (i, single_line) in texts.into_iter().enumerate() {
        if single_line.is_empty() {
            continue;
        }
        let mut cur_texts = String::new();
        let mut total = AvgMeter::default();
        let mut avgs = [AvgMeter::default(); 6];
        for (chid, logprob, fr, fg, fb, br, bg, bb) in single_line {
            let mut ch = dict[chid as usize].as_str();
            if ch == "<SP>" {
                ch = " ";
            } else {
                avgs[0].update((fr * 255.0) as i32);
                avgs[1].update((fg * 255.0) as i32);
                avgs[2].update((fb * 255.0) as i32);
                avgs[3].update((br * 255.0) as i32);
                avgs[4].update((bg * 255.0) as i32);
                avgs[5].update((bb * 255.0) as i32);
            }
            cur_texts.push_str(ch);
            total.update(logprob);
        }
        let prob = total.average().exp();

        out.push(QuadrilateralInfo {
            text: cur_texts,
            fg: Some([
                avgs[0].average() as u8,
                avgs[1].average() as u8,
                avgs[2].average() as u8,
            ]),
            bg: Some([
                avgs[3].average() as u8,
                avgs[4].average() as u8,
                avgs[5].average() as u8,
            ]),
            // allow:clone[arc]
            pos: areas[i].clone(),
            prob,
        });
    }
    out
}

#[async_trait::async_trait]
impl Ocr for Ctc48pxOcr {
    async fn detect(
        &self,
        image: &RawImage,
        areas: &[Arc<parking_lot::Mutex<Quadrilateral>>],
        options: OcrOptions,
        _: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<Vec<QuadrilateralInfo>> {
        let text_height = 48;
        let items = spawn_blocking!(|| {
            ocr::prepare(
                image,
                areas,
                text_height,
                self.max_batch_size,
                &options.debug_path,
            )
        })??;

        //TODO: ignore bubble
        let mut out = vec![];

        let m = self.load().await?;
        let (model, dict) = m.deref();
        let dict = &*dict;
        for (images, _, areas) in items {
            let texts = decode::decode(model, images, 0)?;
            out.extend(spawn_blocking!(|| post_process(texts, dict, &areas))?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base_util::onnx::all_providers;
    use interface_detector::textlines::Quadrilateral;
    use interface_image::{CpuImageProcessor, ImageOp, RawImage};
    use interface_ocr::{Ocr as _, OcrOptions};
    use parking_lot::Mutex;

    use crate::Ctc48pxOcr;

    // #[tokio::test]
    // async fn ocr_test2() {
    //     let img = RawImage::new("0_input.png").unwrap();
    //     let mut mocr = Ctc48pxOcr::new(Arc::new(all_providers()));
    //     let pts:Vec<Quadrilateral> = serde_json::from_slice(include_bytes!("1_quadrilateral.json")).unwrap();
    //     let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
    //     let inp = pts
    //         .into_iter()
    //         .map(|v| Arc::new(Mutex::new(v)))
    //         .collect::<Vec<_>>();
    //     let v = mocr.detect(&Arc::new(img), &inp, &ip).await.unwrap();
    //     println!("{:?}", v);
    // }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ocr_test() {
        let img = RawImage::new("./imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png")
            .expect("Failed to load image");
        let mocr = Ctc48pxOcr::new(Arc::new(all_providers()), 16);
        let inp = vec![
            Arc::new(Mutex::new(Quadrilateral::new(
                vec![(208, 4), (246, 4), (246, 192), (208, 192)],
                1.0,
            ))),
            Arc::new(Mutex::new(Quadrilateral::new(
                vec![(76, 1788), (128, 1788), (128, 1930), (76, 1930)],
                1.0,
            ))),
        ];
        let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
        let mut v = mocr
            .detect(&Arc::new(img), &inp, OcrOptions::default(), &ip)
            .await
            .unwrap();
        v.sort_by_key(|a| a.text.len());
        assert_eq!(v[0].pos.lock().pts()[0].x, 76);
        assert_eq!(v[1].text, "そうだなあ");
        assert_eq!(v[0].text, "ふふっ、");
        assert_eq!(v.len(), 2);
    }
}
