use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::ptr;

use base64::engine::general_purpose;
use base64::Engine as _;
use export::Export;
use image::ExtendedColorType;
use image::ImageEncoder;
use interface_image::RawImage;
use serde::{Deserialize, Serialize};
use v_htmlescape::escape;

pub struct HtmlRenderer;

pub fn copy_files(path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;
    if !path.join("lazyInit.js").exists() {
        File::create(path.join("lazyInit.js"))?
            .write_all(include_bytes!("../static/lazyInit.js"))?;
    }
    if !path.join("style.css").exists() {
        File::create(path.join("style.css"))?.write_all(include_bytes!("../static/style.css"))?;
    }
    if !path.join("script.js").exists() {
        File::create(path.join("script.js"))?.write_all(include_bytes!("../static/script.js"))?;
    }
    Ok(())
}

impl HtmlRenderer {
    pub fn render(data: Vec<Export>, font: Option<String>, archive: bool) -> (Vec<u8>, bool) {
        let mut html = vec![r#"<meta charset="UTF-8" />"#.to_owned()];
        let mut files = vec![];
        for v in data {
            let mut jsons = vec![];

            macro_rules! insert {
                ($img:expr) => {{
                    if archive {
                        files.push($img);
                        (files.len() - 1, files.last().unwrap())
                    } else {
                        (0, &$img)
                    }
                }};
            }
            let mut img = v.get_image();
            let overlay = v.get_overlay();

            for patch in v.blocks {
                let last = patch.translations.get("last_trans").unwrap();
                let obb = patch.obb().unwrap();
                let (x, y) = obb.top_left();

                jsons.push(JsonData {
                    x: x as u32,
                    y: y as u32,
                    width: obb.w as u32,
                    height: obb.h as u32,
                    rotation: obb.angle_deg() as u32,
                    // 与 PNG 同一存储轴；JS 据此切 writing-mode。
                    axis: if patch.axis().is_vertical() {
                        "vertical"
                    } else {
                        "horizontal"
                    },
                    color: patch.fg_color.unwrap_or((0, 0, 0)),
                    shadow: patch
                        .bg_color
                        .map(|v| (v.0, v.1, v.2, 1.0))
                        .unwrap_or((255, 255, 255, 1.0)),
                    text: patch.translations.get(last).unwrap().to_owned(),
                });
            }
            img.apply_filter(&overlay, |a, b| unsafe {
                if *b.get_unchecked(3) > 128 {
                    ptr::copy_nonoverlapping(b.as_ptr(), a.as_mut_ptr(), 3);
                }
            });
            let (index, img) = insert!(img);

            html.push(generate(font.clone(), jsons, index, img, archive));
        }
        html.push("<!--<script>var maxWidth = 300;</script> -->".to_owned());
        html.push(r#"<script src="./lazyInit.js" defer></script>"#.to_owned());
        (html.join("\n").as_bytes().to_vec(), archive)
    }
}

#[derive(Deserialize, Serialize)]
pub struct JsonData {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    rotation: u32,
    axis: &'static str,
    color: (u8, u8, u8),
    shadow: (u8, u8, u8, f32),
    text: String,
}

fn to_base64_img(img: &RawImage) -> String {
    let mut data = vec![];
    let encoder = image::codecs::png::PngEncoder::new(&mut data);
    let ch = img.channels;
    encoder
        .write_image(
            &img.data,
            img.width as u32,
            img.height as u32,
            if ch == 4 {
                ExtendedColorType::Rgba8
            } else {
                ExtendedColorType::Rgb8
            },
        )
        .unwrap();
    let base64_str = general_purpose::STANDARD.encode(&data);
    format!(r#"data:image/png;base64,{}"#, base64_str)
}

fn generate(
    font: Option<String>,
    data: Vec<JsonData>,
    index: usize,
    img: &RawImage,
    archive: bool,
) -> String {
    let data = serde_json::to_string(&data).unwrap();
    let data_str = escape(&data);
    let font = font.unwrap_or_else(|| "arial".to_owned());
    let font_escaped = escape(&font);
    let path = match archive {
        true => format!("./{index}.png"),
        false => to_base64_img(img),
    };
    format!(
        r###"
        <div
            class="canvas-wrapper"
            style="
                --ui-font-family: {};
            "
            data-overlays='{}'
        >
            <img class="base-image" src="{}" alt="Image" />
        </div>
 "###,
        font_escaped, data_str, path
    )
}

#[cfg(test)]
mod tests {
    use super::JsonData;

    #[test]
    fn json_axis_uses_horizontal_vertical() {
        // 与 static/script.js 的 writing-mode 分支约定的取值。
        for axis in ["horizontal", "vertical"] {
            let json = serde_json::to_string(&JsonData {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                rotation: 0,
                axis,
                color: (0, 0, 0),
                shadow: (255, 255, 255, 1.0),
                text: String::new(),
            })
            .unwrap();
            assert!(json.contains(&format!(r#""axis":"{axis}""#)), "{json}");
        }
    }
}
