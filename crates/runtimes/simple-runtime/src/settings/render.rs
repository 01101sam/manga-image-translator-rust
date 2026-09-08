use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RenderSettings {
    pub renderer: Renderer,
    pub alignment: Alignment,
    pub direction: Direction,
    pub disable_font_border: bool,
    pub font_size: Option<f32>,
    pub font_size_offset: f32,
    pub font_size_minimum: f32,
    pub line_spacing: Option<f32>,
    pub rtl: bool,
    pub force_simple_sort: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            renderer: Renderer::default(),
            alignment: Alignment::Auto,
            direction: Direction::Auto,
            disable_font_border: false,
            font_size: None,
            font_size_offset: 0.0,
            font_size_minimum: -1.0,
            line_spacing: None,
            rtl: true,
            force_simple_sort: false,
        }
    }
}

#[derive(Serialize, Deserialize, Default, JsonSchema, PartialEq, Eq)]
pub enum Renderer {
    #[default]
    Png,
    Raw,
    Html,
}

impl Renderer {
    pub fn extension(&self) -> &str {
        match self {
            Renderer::Png => "png",
            Renderer::Raw => "mit.bin",
            Renderer::Html => "html",
        }
    }
}

#[derive(Serialize, Deserialize, Default, JsonSchema, PartialEq, Eq, Copy, Clone)]
pub enum Alignment {
    #[default]
    Auto,
    Left,
    Center,
    Right,
}

#[derive(Serialize, Deserialize, Default, JsonSchema, PartialEq, Eq, Copy, Clone)]
pub enum Direction {
    #[default]
    Auto,
    Horizontal,
    Vertical,
}
