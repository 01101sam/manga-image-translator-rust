use std::{collections::HashMap, ptr};

use anyhow::bail;
use cosmic_text::{
    Align, Attrs, Buffer, Color, FontSystem, LayoutRun, Metrics, Shaping, Stretch, Style,
    SwashCache, Weight,
};

use export::Export;
use interface_image::{DimType, Mask, RawImage};

mod layout;
use layout::{BubbleFrame, fit_font_px, shrink_isotropic};
use opencv::{
    calib3d::{find_homography, RANSAC},
    core::{
        no_array, Mat, MatTraitConst, MatTraitManual as _, Point, Point2f, Scalar, Size, Vector,
        BORDER_CONSTANT,
    },
    imgproc::{self, dilate, morphology_default_border_value, warp_perspective, INTER_LINEAR},
};
use ordered_float::OrderedFloat;
use textline_merge::{ScriptAxis, TextBlock};

pub struct PngRenderer {
    font_system: FontSystem,
    cache: SwashCache,
}

pub struct PngRenderConfig {
    pub min_fontsize: f32,
    pub max_fontsize: f32,
    pub detect_offset: f32,
    pub fg_color: Option<(u8, u8, u8)>,
    pub bg_color: Option<(u8, u8, u8)>,
    pub align: MyAlign,
    pub letter_spacing: Option<f32>,
    pub font_size: Option<f32>,
    pub font_size_offset: f32,
    pub font_size_minimum: f32,
    pub line_height: Option<f32>,
    pub family: Option<String>,
    pub disable_font_border: bool,
    pub direction: RenderDirection,
}

impl Default for PngRenderConfig {
    fn default() -> Self {
        Self {
            min_fontsize: 1.0,
            max_fontsize: 200.0,
            detect_offset: 8.0,
            fg_color: None,
            bg_color: None,
            align: MyAlign::Center,
            letter_spacing: None,
            font_size: None,
            font_size_offset: 0.0,
            font_size_minimum: -1.0,
            line_height: None,
            family: None,
            disable_font_border: false,
            direction: RenderDirection::Auto,
        }
    }
}

pub enum MyAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy)]
pub enum RenderDirection {
    Auto,
    Horizontal,
    Vertical,
}

impl RenderDirection {
    /// 设置覆盖存储轴。覆盖只影响本次绘制，不写回 block。
    pub fn resolve(self, stored: ScriptAxis) -> ScriptAxis {
        match self {
            Self::Auto => stored,
            Self::Horizontal => ScriptAxis::Horizontal,
            Self::Vertical => ScriptAxis::VerticalLtr,
        }
    }
}

impl PngRenderer {
    pub fn render(&mut self, exp: Export, config: PngRenderConfig) -> anyhow::Result<RawImage> {
        let mut img = exp.get_image();
        let overlay = exp.get_overlay();
        if overlay.channels == 4 && overlay.width == img.width && overlay.height == img.height {
            img.apply_filter(&overlay, |a, b| unsafe {
                if *b.get_unchecked(3) > 128 {
                    ptr::copy_nonoverlapping(b.as_ptr(), a.as_mut_ptr(), 3);
                }
            });
        }
        for (block_idx, block) in exp.blocks.into_iter().enumerate() {
            self.paint_block(&mut img, &block, &config, block_idx)?;
        }
        Ok(img)
    }

    fn paint_block(
        &mut self,
        img: &mut RawImage,
        block: &TextBlock,
        config: &PngRenderConfig,
        block_idx: usize,
    ) -> anyhow::Result<()> {
        let Some(text) = block.translation() else {
            return Ok(());
        };
        if text.trim().is_empty() {
            return Ok(());
        }
        let Some(frame) = BubbleFrame::from_block(block, img.width as u32, img.height as u32)
        else {
            return Ok(());
        };
        let (frame_w, frame_h) = (frame.width_px as f32, frame.height_px as f32);
        let vertical = config.direction.resolve(block.axis()).is_vertical();
        let min_font = if config.font_size_minimum < 0.0 {
            ((img.width as f32 + img.height as f32) / 200.0).max(1.0)
        } else {
            config.font_size_minimum.max(1.0)
        };
        // 只缩小：死字段 max_fontsize 转正为上界（默认 200），短句不撑满框。
        let preferred = config
            .font_size
            .unwrap_or(block.font_size as f32 + config.font_size_offset)
            .clamp(min_font, config.max_fontsize.max(min_font));
        let (fg, bg) = fg_bg_compare(
            config.fg_color.or(block.fg_color).unwrap_or((0, 0, 0)),
            config.bg_color.or(block.bg_color).unwrap_or((255, 255, 255)),
        );
        let bg = if config.disable_font_border {
            None
        } else {
            Some(bg)
        };
        let line_height = config.line_height.unwrap_or(if vertical { 1.2 } else { 1.01 });
        let template = RenderTextBlock {
            align: match config.align {
                MyAlign::Left => Align::Left,
                MyAlign::Center => Align::Center,
                MyAlign::Right => Align::Right,
            },
            default_font_size: preferred,
            default_line_height: line_height,
            size: (frame.width_px as usize, frame.height_px as usize),
            texts: vec![Text {
                text: text.to_owned(),
                letter_spacing: config.letter_spacing,
                color: Some(fg),
                bg_color: bg,
                stretch: None,
                style: Style::Normal,
                weight: None,
                family: config.family.clone(),
                font_size: preferred,
                line_height,
            }],
        };
        let (font_size, line_height) = fit_font_px(
            self,
            &template,
            frame_w,
            frame_h,
            preferred,
            min_font,
            line_height,
        );
        let mut fitted = template.clone();
        fitted.set_font_size(font_size);
        fitted.set_line_height(line_height);
        let mut box_img = self.render_block(fitted);
        if box_img.width == 0 || box_img.height == 0 {
            return Ok(());
        }
        // L3 兜底：min_font + 行高 1.0 仍溢出，光栅化后各向同性缩小。
        if box_img.width as f32 > frame_w || box_img.height as f32 > frame_h {
            let scale = (frame_w / box_img.width as f32).min(frame_h / box_img.height as f32);
            box_img = shrink_isotropic(&box_img, scale)?;
            log::warn!("block={block_idx} 译文超框，min_font 行高1.0 后仍溢出，等比缩放 scale={scale:.2}");
        }
        let box_img = pad_to_aspect(box_img, frame_w / frame_h);
        warp_onto(img, &box_img, frame.dest)
    }
}

impl Default for PngRenderer {
    fn default() -> Self {
        Self {
            font_system: FontSystem::new(),
            cache: SwashCache::new(),
        }
    }
}

fn to_metrics(input: &RenderTextBlock) -> Metrics {
    Metrics::new(
        input.default_font_size,
        input.default_font_size * input.default_line_height,
    )
}

#[derive(Default)]
pub struct ColorMap {
    index: usize,
    map: HashMap<(u8, u8, u8), usize>,
    map2: HashMap<usize, (u8, u8, u8)>,
}

impl ColorMap {
    pub fn get_id(&mut self, color: (u8, u8, u8)) -> anyhow::Result<usize> {
        if let Some(i) = self.map.get(&color) {
            return Ok(*i);
        }
        self.index += 1;
        if self.index >= 255 {
            bail!("To many colors in text block")
        }
        self.map.insert(color, self.index);
        self.map2.insert(self.index, color);

        Ok(self.index)
    }

    pub fn to_image(&self, input: Mask) -> RawImage {
        let w = input.width;
        let h = input.height;
        let mut data = Vec::with_capacity(input.data.len());
        for id in input.data {
            let get = self.map2.get(&(id as usize));
            data.push(match get {
                Some(s) => [s.0, s.1, s.2, 255],
                None => [0, 0, 0, 0],
            });
        }
        let len = data.len() * 4;
        let cap = data.capacity() * 4;
        let ptr = data.as_ptr() as *mut u8;

        std::mem::forget(data);

        let flat: Vec<u8> = unsafe { Vec::from_raw_parts(ptr, len, cap) };
        RawImage {
            data: flat,
            width: w,
            height: h,
            channels: 4,
        }
    }
}

fn backdrop_kernel(font_size: i32) -> opencv::Result<opencv::core::Mat> {
    let k = (font_size as f32 / 12.0).ceil() as i32;
    let size = 2 * k + 1;

    imgproc::get_structuring_element(
        imgproc::MORPH_ELLIPSE,
        Size::new(size, size),
        Point::new(-1, -1),
    )
}

fn wh(layouts: &Vec<LayoutRun<'_>>) -> (usize, usize) {
    let (h, w): (Vec<_>, Vec<_>) = layouts
        .iter()
        .map(|v| (v.line_top + v.line_height, v.line_w))
        .unzip();
    let h = h
        .iter()
        .map(|v| OrderedFloat(*v))
        .max()
        .unwrap_or_default()
        .ceil() as usize;
    let w = w
        .iter()
        .map(|v| OrderedFloat(*v))
        .max()
        .unwrap_or_default()
        .ceil() as usize;
    (w, h)
}
impl PngRenderer {
    fn create_buffer(&mut self, text: &RenderTextBlock, color_map: &mut ColorMap) -> Buffer {
        let metrics = to_metrics(&text);
        let mut buffer_ = Buffer::new(&mut self.font_system, metrics);
        let mut buffer = buffer_.borrow_with(&mut self.font_system);
        // 宽度约束换行，高度永不做 Buffer 输入：
        // width None 会永不换行，height Some 会静默丢掉超高行。
        buffer.set_size(Some(text.size.0 as f32), None);
        let attrs = Attrs::new();
        let spans = text
            .texts
            .iter()
            .map(|v| (v.text.as_str(), v.to_attr(color_map)))
            .collect::<Vec<_>>();
        buffer.set_rich_text(
            spans.iter().map(|(text, attrs)| (*text, attrs.clone())),
            &attrs,
            Shaping::Advanced,
            Some(text.align),
        );
        buffer.shape_until_scroll(true);
        buffer_
    }

    pub fn render_block(&mut self, text: RenderTextBlock) -> RawImage {
        if text.texts.is_empty() {
            return empty_rgba();
        }
        let font_size =
            text.texts.iter().map(|v| v.font_size).sum::<f32>() / text.texts.len() as f32;
        let mut color_map = ColorMap::default();
        let buffer = self.create_buffer(&text, &mut color_map);
        let layouts = buffer.layout_runs().collect::<Vec<_>>();
        let (w, h) = wh(&layouts);
        if w == 0 || h == 0 {
            return empty_rgba();
        }

        let mut rgb = vec![[0_u8; 4]; h as usize * w as usize];
        let mut bg = vec![0_u8; h as usize * w as usize];
        // 宽度约束下对齐会产生行内偏移（居中/右对齐），画布按内容宽分配，
        // 写像素时必须减掉该偏移，否则内容整体落在画布外。
        let wrap_w = text.size.0 as f32;
        for run in layouts {
            let shift = match text.align {
                Align::Center => (wrap_w - run.line_w) / 2.0,
                Align::Right | Align::End => wrap_w - run.line_w,
                _ => 0.0,
            }
            .round() as i32;
            for glyph in run.glyphs.iter() {
                let physical_glyph = glyph.physical((0., 0.), 1.0);
                let glyph_color = glyph.color_opt.unwrap_or(Color::rgb(0, 0, 0));
                self.cache.with_pixels(
                    &mut self.font_system,
                    physical_glyph.cache_key,
                    glyph_color,
                    |x, y, color| {
                        let x = physical_glyph.x - shift + x;
                        let y = run.line_y as i32 + physical_glyph.y + y;
                        let a = color.a();
                        // 字形墨水可超出 wh() 行盒（行高 < 字形实际高时末行下溢、
                        // 右 bearing 外溢），超界像素丢弃而非 panic。
                        if a == 0 || x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                            return;
                        }
                        let x = x as usize;
                        let y = y as usize;
                        rgb[y * w + x] = [color.r(), color.g(), color.b(), a];
                        if a >= 127 {
                            bg[y * w + x] = glyph.metadata as u8;
                        }
                    },
                );
            }
        }

        let src = Mat::from_slice(&bg).unwrap();
        let src = src.reshape(1, h as i32).unwrap();
        let mut dst = Mat::default();
        dilate(
            &src,
            &mut dst,
            &backdrop_kernel(font_size as i32).unwrap(),
            Point::new(-1, -1),
            1,
            BORDER_CONSTANT,
            morphology_default_border_value().unwrap(),
        )
        .unwrap();
        let bg = color_map.to_image(Mask::from(dst));
        let len = rgb.len() * 4;
        let cap = rgb.capacity() * 4;
        let ptr = rgb.as_ptr() as *mut u8;

        std::mem::forget(rgb);

        let flat: Vec<u8> = unsafe { Vec::from_raw_parts(ptr, len, cap) };
        let text = RawImage {
            width: w as DimType,
            height: h as DimType,
            data: flat,
            channels: 4,
        };
        bg.apply(text)
    }
}

fn empty_rgba() -> RawImage {
    RawImage {
        data: vec![],
        width: 0,
        height: 0,
        channels: 4,
    }
}

fn color_difference(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
    let dr = a.0 as f32 - b.0 as f32;
    let dg = a.1 as f32 - b.1 as f32;
    let db = a.2 as f32 - b.2 as f32;
    (dr * dr + dg * dg + db * db).sqrt()
}

fn fg_bg_compare(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> ((u8, u8, u8), (u8, u8, u8)) {
    let fg_avg = (fg.0 as u16 + fg.1 as u16 + fg.2 as u16) / 3;
    if color_difference(fg, bg) < 30.0 {
        let bg = if fg_avg <= 127 {
            (255, 255, 255)
        } else {
            (0, 0, 0)
        };
        (fg, bg)
    } else {
        (fg, bg)
    }
}

/// 补白到目标宽高比：单一居中公式，内容中心与画布中心对齐。
fn pad_to_aspect(img: RawImage, r_orig: f32) -> RawImage {
    if img.width == 0 || img.height == 0 || r_orig <= 0.0 {
        return img;
    }
    let r_temp = img.width as f32 / img.height as f32;
    if (r_temp - r_orig).abs() < 0.01 {
        return img;
    }
    let (w, h, ox, oy) = if r_temp > r_orig {
        let h_ext = ((img.width as f32 / r_orig - img.height as f32) / 2.0).round() as i32;
        if h_ext <= 0 {
            return img;
        }
        (img.width, img.height + h_ext as u16 * 2, 0_u16, h_ext as u16)
    } else {
        let w_ext = ((img.height as f32 * r_orig - img.width as f32) / 2.0).round() as i32;
        if w_ext <= 0 {
            return img;
        }
        (
            img.width + w_ext as u16 * 2,
            img.height,
            w_ext as u16,
            0,
        )
    };
    let mut out = RawImage {
        data: vec![0; w as usize * h as usize * 4],
        width: w,
        height: h,
        channels: 4,
    };
    out.apply_patch(&img, ox, oy);
    out
}

fn warp_onto(dest: &mut RawImage, src: &RawImage, dst: [(i64, i64); 4]) -> anyhow::Result<()> {
    if src.width < 2 || src.height < 2 {
        return Ok(());
    }
    let src_pts = [
        Point2f::new(0.0, 0.0),
        Point2f::new(src.width as f32 - 1.0, 0.0),
        Point2f::new(src.width as f32 - 1.0, src.height as f32 - 1.0),
        Point2f::new(0.0, src.height as f32 - 1.0),
    ]
    .into_iter()
    .collect::<Vector<Point2f>>();
    let dst_pts = dst
        .into_iter()
        .map(|(x, y)| Point2f::new(x as f32, y as f32))
        .collect::<Vector<Point2f>>();
    let mut m = find_homography(&src_pts, &dst_pts, &mut no_array(), RANSAC, 5.0)?;
    let h = m.data_typed_mut::<f64>()?;
    let (x0, y0, x1, y1) = warp_roi(h, src, dest);
    if x1 <= x0 || y1 <= y0 {
        return Ok(());
    }
    // Shift the homography so the ROI's top-left lands on (0, 0) and warp only that region.
    for c in 0..3 {
        h[c] -= x0 as f64 * h[6 + c];
        h[3 + c] -= y0 as f64 * h[6 + c];
    }
    let src_mat = src.as_opencv_mat()?;
    let mut warped = Mat::default();
    warp_perspective(
        &src_mat,
        &mut warped,
        &m,
        Size::new((x1 - x0) as i32, (y1 - y0) as i32),
        INTER_LINEAR,
        BORDER_CONSTANT,
        Scalar::all(0.0),
    )?;
    let warped = RawImage::try_from(warped)?;
    blend_warped(dest, &warped, x0, y0);
    Ok(())
}

/// Destination pixels can only be non-transparent where their source sample falls inside
/// (-1, w) x (-1, h); everything outside the forward image of that rectangle is border (alpha 0).
/// Returns the half-open pixel box [x0, x1) x [y0, y1) covering it, clamped to `dest`.
fn warp_roi(h: &[f64], src: &RawImage, dest: &RawImage) -> (usize, usize, usize, usize) {
    let (w, hh) = (src.width as f64, src.height as f64);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (sx, sy) in [(-1.0, -1.0), (w, -1.0), (w, hh), (-1.0, hh)] {
        let d = h[6] * sx + h[7] * sy + h[8];
        let x = (h[0] * sx + h[1] * sy + h[2]) / d;
        let y = (h[3] * sx + h[4] * sy + h[5]) / d;
        if d <= 0.0 || !x.is_finite() || !y.is_finite() {
            return (0, 0, dest.width as usize, dest.height as usize);
        }
        (min_x, min_y) = (min_x.min(x), min_y.min(y));
        (max_x, max_y) = (max_x.max(x), max_y.max(y));
    }
    let clamp = |v: f64, hi: DimType| (v as i64).clamp(0, hi as i64) as usize;
    (
        clamp(min_x.floor() - 1.0, dest.width),
        clamp(min_y.floor() - 1.0, dest.height),
        clamp(max_x.ceil() + 2.0, dest.width),
        clamp(max_y.ceil() + 2.0, dest.height),
    )
}

fn blend_warped(dest: &mut RawImage, warped: &RawImage, x0: usize, y0: usize) {
    let (dc, wc) = (dest.channels as usize, warped.channels as usize);
    for y in 0..warped.height as usize {
        for x in 0..warped.width as usize {
            let wi = (y * warped.width as usize + x) * wc;
            let a = if wc >= 4 { warped.data[wi + 3] } else { 255 };
            if a == 0 {
                continue;
            }
            let di = ((y + y0) * dest.width as usize + x + x0) * dc;
            let t = a as f32 / 255.0;
            dest.data[di] =
                ((warped.data[wi] as f32 * t) + dest.data[di] as f32 * (1.0 - t)).round() as u8;
            dest.data[di + 1] = ((warped.data[wi + 1] as f32 * t)
                + dest.data[di + 1] as f32 * (1.0 - t))
                .round() as u8;
            dest.data[di + 2] = ((warped.data[wi + 2] as f32 * t)
                + dest.data[di + 2] as f32 * (1.0 - t))
                .round() as u8;
        }
    }
}

#[derive(Clone)]
pub struct RenderTextBlock {
    align: Align,
    default_font_size: f32,
    default_line_height: f32,
    size: (usize, usize),
    texts: Vec<Text>,
}

impl RenderTextBlock {
    fn set_font_size(&mut self, font_size: f32) {
        self.default_font_size = font_size;
        self.texts.iter_mut().for_each(|v| v.font_size = font_size);
    }

    fn set_line_height(&mut self, line_height: f32) {
        self.default_line_height = line_height;
        self.texts
            .iter_mut()
            .for_each(|v| v.line_height = line_height);
    }
}

#[derive(Clone)]
pub struct Text {
    text: String,
    letter_spacing: Option<f32>,
    color: Option<(u8, u8, u8)>,
    bg_color: Option<(u8, u8, u8)>,
    stretch: Option<Stretch>,
    style: Style,
    weight: Option<Weight>,
    family: Option<String>,
    font_size: f32,
    line_height: f32,
}

impl Text {
    pub fn to_attr<'a>(&'a self, color_map: &mut ColorMap) -> Attrs<'a> {
        let mut attrs = Attrs::new();
        let color = self.color.unwrap_or_default();
        attrs = attrs
            .color(Color::rgb(color.0, color.1, color.2))
            .style(self.style)
            .metrics(Metrics::new(
                self.font_size,
                self.font_size * self.line_height,
            ))
            .metadata(
                color_map
                    .get_id(self.bg_color.unwrap_or((255, 255, 255)))
                    .unwrap(),
            );
        if let Some(letter_spacing) = self.letter_spacing {
            attrs = attrs.letter_spacing(letter_spacing)
        }
        if let Some(stretch) = self.stretch {
            attrs = attrs.stretch(stretch);
        }
        if let Some(weight) = self.weight {
            attrs = attrs.weight(weight);
        }
        if let Some(family) = &self.family {
            attrs = attrs.family(cosmic_text::Family::Name(family));
        }

        attrs
    }
}

#[cfg(test)]
mod tests {
    use cosmic_text::Style;
    use env_logger::Env;

    use crate::{PngRenderConfig, PngRenderer, RenderTextBlock, Text};

    #[test]
    fn render_test() {
        env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();
        let mut renderer = PngRenderer::default();
        let block = RenderTextBlock {
            align: cosmic_text::Align::Center,
            default_font_size: 1.0,
            default_line_height: 1.2,
            size: (1000, 2000),
            texts: vec![Text {
                text: "Hello world, this is a test".to_owned(),
                letter_spacing: None,
                color: Some((255, 0, 0)),
                bg_color: None,
                stretch: None,
                style: Style::Normal,
                weight: None,
                family: Some("Arial".to_owned()),
                font_size: 24.0,
                line_height: 1.2,
            }],
        };
        let img = renderer.render_block(block);
        assert!(img.width > 0 && img.height > 0);
        assert!(img.data.chunks(4).any(|p| p[3] > 0));
    }

    #[test]
    fn render_composites_overlay_and_text() {
        use export::Export;
        use image::{DynamicImage, Rgb, RgbImage, Rgba, RgbaImage};
        use interface_detector::textlines::MyPoint;
        use interface_translator::LangIdDetector;
        use textline_merge::TextBlock;

        let mut bg = RgbImage::new(120, 60);
        bg.pixels_mut().for_each(|p| *p = Rgb([10, 10, 10]));
        let mut overlay = RgbaImage::new(120, 60);
        for y in 8..52 {
            for x in 8..112 {
                overlay.put_pixel(x, y, Rgba([240, 240, 240, 255]));
            }
        }
        let det = LangIdDetector::new().unwrap();
        let block = TextBlock::new(
            vec![[
                MyPoint { x: 12, y: 12 },
                MyPoint { x: 108, y: 12 },
                MyPoint { x: 108, y: 48 },
                MyPoint { x: 12, y: 48 },
            ]],
            vec!["src".into()],
            18,
            0.0,
            1.0,
            Some((20, 20, 20)),
            Some((240, 240, 240)),
            &det,
        )
        .with_translation("ENG", "Hi");
        let exp = Export::new(
            DynamicImage::ImageRgb8(bg),
            DynamicImage::ImageRgba8(overlay),
            vec![block],
            None,
        );
        let mut renderer = PngRenderer::default();
        let out = renderer.render(exp, PngRenderConfig::default()).unwrap();
        assert_eq!(out.width, 120);
        assert_eq!(out.height, 60);
        assert!(out.data.chunks(out.channels as usize).any(|p| p[0] > 20));
    }
}
