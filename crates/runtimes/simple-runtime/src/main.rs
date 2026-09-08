use std::{
    fs::{create_dir_all, File},
    io::Write,
    path::PathBuf,
    time::Instant,
};

use clap::Parser as _;
use config::Config;
use html::HtmlRenderer;
use log::{error, info, warn};
use png::{MyAlign, PngRenderConfig, PngRenderer, RenderDirection};
use tracing_subscriber::EnvFilter;
use walkdir::WalkDir;

use crate::{
    settings::{Alignment, Direction, RenderSettings, Renderer, Settings},
    setup::Models,
    update::{check_crate_version, check_cuda},
};

mod api;
mod cache;
pub mod cli;
mod debug;
mod dict;
mod execute;
pub mod settings;
pub mod setup;
mod ui;
mod update;

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();
    let (level, ort_level) = match cli.verbose {
        3 | 2 => ("debug", "ort=debug"),
        1 => ("info", "ort=warn"),
        _ => ("warn", "ort=error"),
    };

    let base_filter = EnvFilter::new(level);
    let filter = base_filter.add_directive(ort_level.parse().unwrap());

    tracing_subscriber::fmt()
        .with_level(true)
        .with_target(true)
        .with_env_filter(filter)
        .init();
    let cuda = check_cuda();
    if !cuda && cfg!(all(target_arch = "x86_64", not(target_os = "macos"))) {
        warn!("CUDA is not available")
    }
    let _ = check_crate_version("frederik-uni/manga-image-translator-rust").await;

    let startup = Instant::now();
    let mut models = Models::new(
        cli.max_batch_size_upscaler,
        cli.max_batch_size_ocr,
        true,
        cuda,
    )
    .await;
    eprintln!("PERF startup {}", startup.elapsed().as_millis());
    match cli.command {
        cli::Commands::Cli {
            input,
            output,
            config,
            overwrite,
            save_mask,
        } => {
            let mut input_list = WalkDir::new(&input)
                .into_iter()
                .filter_map(|v| v.ok())
                .map(|v| v.path().to_path_buf())
                .filter(|v| v.is_file())
                .filter(|v| !v.to_string_lossy().starts_with("."))
                .map(|v| v.strip_prefix(&input).map(|v| v.to_path_buf()).unwrap_or(v))
                //TODO: add other extensions
                .filter(|v| {
                    ["png", "jpg", "jpeg", "webp"].contains(
                        &v.extension()
                            .map(|v| v.to_string_lossy())
                            .unwrap_or_default()
                            .to_lowercase()
                            .as_str(),
                    )
                })
                .collect::<Vec<_>>();
            let mut settings = Config::builder();
            if let Some(config) = config {
                if !config.exists() {
                    panic!("Config file does not exist")
                }
                settings = settings.add_source(config::File::from(config));
            }
            let settings = settings.build().expect("Failed to build settings");
            let settings = settings.try_deserialize::<Settings>().unwrap_or_default();
            let out_ext = settings.render.renderer.extension();
            if !overwrite {
                input_list = input_list
                    .into_iter()
                    .filter(|v| {
                        let mut path = output.join(v);
                        path.set_extension(out_ext);
                        !path.exists()
                    })
                    .collect::<Vec<_>>();
            }

            for path in input_list {
                info!("Processing {}", path.display());
                let mut output = output.join(&path);
                let path = input.join(path);
                if !path.exists() || !path.is_file() {
                    warn!("File {} cant be found", path.display());
                    continue;
                }
                let img = match image::open(&path) {
                    Ok(img) => img,
                    Err(err) => {
                        error!("Failed to open image {}: {}", path.display(), err);
                        continue;
                    }
                };
                let debug_path = if cli.verbose > 2 {
                    let id = uuid::Uuid::new_v4();
                    let p = PathBuf::from(format!("debug/{}", id.to_string()));
                    create_dir_all(&p).expect("Failed to create debug directory");
                    Some(p)
                } else {
                    None
                };
                let mask_out = save_mask.then(|| {
                    let mut p = output.clone();
                    p.set_extension("mask.png");
                    if let Some(parent) = p.parent() {
                        create_dir_all(parent).expect("Failed to create mask directory");
                    }
                    p
                });
                let exp = models
                    .execute(img, &settings, debug_path, mask_out)
                    .await
                    .unwrap();
                let exp = match exp {
                    Some(v) => v,
                    None => {
                        info!("Failed to detect any translatable content");
                        continue;
                    }
                };
                let ocr: Vec<serde_json::Value> = exp
                    .blocks
                    .iter()
                    .map(|b| {
                        serde_json::json!({
                            "text": b.text,
                            "translation": b.translation(),
                        })
                    })
                    .collect();
                eprintln!(
                    "OCR_JSON {}",
                    serde_json::to_string(&ocr).expect("ocr json")
                );
                output.set_extension(out_ext);
                if let Some(parent) = output.parent() {
                    create_dir_all(parent).expect("Failed to create parent directory");
                }
                let render_t = Instant::now();
                match settings.render.renderer {
                    Renderer::Html => {
                        let (data, _) = HtmlRenderer::render(vec![exp], None, false);
                        html::copy_files(output.parent().unwrap_or(&output))
                            .expect("Failed to copy important js files");
                        File::create(output).unwrap().write_all(&data).unwrap();
                    }
                    Renderer::Raw => {
                        File::create(output)
                            .unwrap()
                            .write_all(&exp.export())
                            .unwrap();
                    }
                    Renderer::Png => {
                        let mut renderer = PngRenderer::default();
                        let img = renderer
                            .render(exp, render_config(&settings.render))
                            .expect("Failed to render png");
                        img.to_image()
                            .expect("Failed to encode png")
                            .save(&output)
                            .expect("Failed to save png");
                    }
                }
                eprintln!("PERF render {}", render_t.elapsed().as_millis());
            }
        }
        cli::Commands::Api { host, port } => api::main(&host, port).await.unwrap(),
        cli::Commands::Ui => {
            let native_options = eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_inner_size([400.0, 300.0])
                    .with_min_inner_size([300.0, 220.0]),
                // .with_icon(
                //     // NOTE: Adding an icon is optional
                //     eframe::icon_data::from_png_bytes(
                //         &include_bytes!("../assets/icon-256.png")[..],
                //     )
                //     .expect("Failed to load icon"),
                // ),
                ..Default::default()
            };
            eframe::run_native(
                "Manga Image Translator",
                native_options,
                Box::new(|cc| Ok(Box::new(ui::MitApp::new(cc)))),
            )
            .expect("Failed to run egui");
            return;
        }
    }
}

fn render_config(settings: &RenderSettings) -> PngRenderConfig {
    PngRenderConfig {
        align: match settings.alignment {
            Alignment::Left => MyAlign::Left,
            Alignment::Right => MyAlign::Right,
            Alignment::Auto | Alignment::Center => MyAlign::Center,
        },
        font_size: settings.font_size,
        font_size_offset: settings.font_size_offset,
        font_size_minimum: settings.font_size_minimum,
        line_height: settings.line_spacing,
        disable_font_border: settings.disable_font_border,
        direction: match settings.direction {
            Direction::Auto => RenderDirection::Auto,
            Direction::Horizontal => RenderDirection::Horizontal,
            Direction::Vertical => RenderDirection::Vertical,
        },
        ..PngRenderConfig::default()
    }
}
