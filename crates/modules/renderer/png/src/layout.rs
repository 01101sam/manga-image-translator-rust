//! 目标框与排版适配（横竖共用）。
//!
//! 渲染只读 `TextBlock::axis`（存储轴），方向不在此重算。

use interface_image::RawImage;
use opencv::{
    core::{Mat, MatTraitConst as _, Size},
    imgproc::{INTER_AREA, resize},
};
use textline_merge::{ScriptAxis, TextBlock};

use crate::{ColorMap, PngRenderer, RenderTextBlock, backdrop_kernel, empty_rgba, wh};

/// 渲染目标框：行集合转正 AABB 外扩半字号，再入界到图像内。
///
/// 不选块级 OBB：近方形块主次轴不定，方向会翻。
pub struct BubbleFrame {
    /// 与 `min_rect` 同绕序，供 `warp_onto`。四角保证在图像内。
    pub dest: [(i64, i64); 4],
    /// `dest` 对边中点距。横排 wrap 宽 / 竖排列总宽。
    pub width_px: u32,
    /// 横排行堆叠预算 / 竖排单列高。
    pub height_px: u32,
}

impl BubbleFrame {
    /// 派生：转正取行点 AABB → 外扩 `0.5 * font_size` → 旋回 → 入界。
    ///
    /// 退化（无行点、图尺寸为零、边 <2px）返回 `None`，调用方跳过该块。
    pub fn from_block(block: &TextBlock, img_w: u32, img_h: u32) -> Option<Self> {
        if img_w == 0 || img_h == 0 {
            return None;
        }
        // 与 `min_rect` 同几何：按 +angle 转正（textline-merge 的旋向约定），
        // 行间隙自然包含在全部行点的包络里。
        let (cx, cy) = block.center();
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        for quad in &block.lines {
            for p in quad {
                let (x, y) = rotate(p.x as f64, p.y as f64, cx as f64, cy as f64, block.angle);
                bounds = Some(match bounds {
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                    None => (x, y, x, y),
                });
            }
        }
        let (mut x0, mut y0, mut x1, mut y1) = bounds?;
        // 半字号外扩逼近气泡留白；不是轮廓检测，圆泡四角仍可能出框。
        let pad = block.font_size as f64 * 0.5;
        x0 -= pad;
        y0 -= pad;
        x1 += pad;
        y1 += pad;
        let mut dest = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| {
            let (rx, ry) = rotate(x, y, cx as f64, cy as f64, -block.angle);
            (rx.round() as i64, ry.round() as i64)
        });
        fit_to_image(&mut dest, img_w, img_h);
        let (w, h) = dest_axes(dest);
        if w < 2.0 || h < 2.0 {
            return None;
        }
        Some(Self {
            dest,
            width_px: w.round() as u32,
            height_px: h.round() as u32,
        })
    }
}

/// 入界：先整体平移（保形），框大于图才按外接盒等比收缩（保长宽比）。
///
/// 逐坐标钳制会把旋转框削成多边形，不用。
fn fit_to_image(dest: &mut [(i64, i64); 4], img_w: u32, img_h: u32) {
    let (w, h) = (img_w as i64, img_h as i64);
    let (mut x0, mut y0, mut x1, mut y1) = bbox(dest);
    if x1 - x0 > w - 1 || y1 - y0 > h - 1 {
        let span_x = (x1 - x0).max(1) as f64;
        let span_y = (y1 - y0).max(1) as f64;
        let s = ((w - 1) as f64 / span_x).min((h - 1) as f64 / span_y);
        let (qx, qy) = centroid(dest);
        for (x, y) in dest.iter_mut() {
            *x = (qx + (*x as f64 - qx) * s).round() as i64;
            *y = (qy + (*y as f64 - qy) * s).round() as i64;
        }
        (x0, y0, x1, y1) = bbox(dest);
    }
    // 收缩后跨度必小于图，平移一次即入界。
    let dx = if x0 < 0 {
        -x0
    } else if x1 > w - 1 {
        (w - 1) - x1
    } else {
        0
    };
    let dy = if y0 < 0 {
        -y0
    } else if y1 > h - 1 {
        (h - 1) - y1
    } else {
        0
    };
    for (x, y) in dest.iter_mut() {
        *x += dx;
        *y += dy;
    }
}

/// 与 textline-merge `rotate_polygons` 同旋向（角度制）。
fn rotate(x: f64, y: f64, cx: f64, cy: f64, deg: f64) -> (f64, f64) {
    if deg == 0.0 {
        return (x, y);
    }
    let rad = deg * std::f64::consts::PI / 180.0;
    let (s, c) = (rad.sin(), rad.cos());
    ((x - cx) * c - (y - cy) * s + cx, (x - cx) * s + (y - cy) * c + cy)
}

fn bbox(dest: &[(i64, i64); 4]) -> (i64, i64, i64, i64) {
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for &(x, y) in dest {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    (x0, y0, x1, y1)
}

fn centroid(dest: &[(i64, i64); 4]) -> (f64, f64) {
    (
        dest.iter().map(|p| p.0).sum::<i64>() as f64 / 4.0,
        dest.iter().map(|p| p.1).sum::<i64>() as f64 / 4.0,
    )
}

/// 对边中点距：旋转框的等价位宽/高。
pub fn dest_axes(dst: [(i64, i64); 4]) -> (f32, f32) {    let mid = |a: (i64, i64), b: (i64, i64)| {
        (
            (a.0 + b.0) as f32 / 2.0,
            (a.1 + b.1) as f32 / 2.0,
        )
    };
    let m01 = mid(dst[0], dst[1]);
    let m12 = mid(dst[1], dst[2]);
    let m23 = mid(dst[2], dst[3]);
    let m30 = mid(dst[3], dst[0]);
    let norm_h = ((m12.0 - m30.0).hypot(m12.1 - m30.1)).max(1.0);
    let norm_v = ((m23.0 - m01.0).hypot(m23.1 - m01.1)).max(1.0);
    (norm_h, norm_v)
}

/// 只缩小适配（L1/L2）：L1 在 `[min_font, preferred]` 二分；到底仍溢出则
/// L2 把行高收到 1.0 再二分一次。返回 `(字号, 行高)`。
///
/// 短句不放大：`preferred` 装得下就直接用。L2 仍溢出由调用方 L3 等比缩放兜底。
pub fn fit_font_px(
    renderer: &mut PngRenderer,
    axis: ScriptAxis,
    template: &RenderTextBlock,
    frame: &BubbleFrame,
    preferred: f32,
    min_font: f32,
    line_height: f32,
) -> (f32, f32) {
    let (frame_w, frame_h) = (frame.width_px as f32, frame.height_px as f32);
    let fits = |renderer: &mut PngRenderer, font: f32, lh: f32| {
        let (w, h) = measure(renderer, axis, template, frame, font, lh);
        w as f32 <= frame_w && h as f32 <= frame_h
    };
    if fits(renderer, preferred, line_height) {
        return (preferred, line_height);
    }
    let lo = bisect(
        |r, font| fits(r, font, line_height),
        renderer,
        min_font,
        preferred,
    );
    if fits(renderer, lo, line_height) {
        return (lo, line_height);
    }
    if line_height > 1.0 {
        if fits(renderer, preferred, 1.0) {
            return (preferred, 1.0);
        }
        let lo = bisect(|r, font| fits(r, font, 1.0), renderer, min_font, preferred);
        return (lo, 1.0);
    }
    (lo, line_height)
}

fn bisect(
    mut fits_at: impl FnMut(&mut PngRenderer, f32) -> bool,
    renderer: &mut PngRenderer,
    min_font: f32,
    preferred: f32,
) -> f32 {
    let mut lo = min_font;
    let mut hi = preferred;
    while hi - lo > 0.5 {
        let mid = (lo + hi) / 2.0;
        if fits_at(renderer, mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// 测量走新排版（横排折行 / 竖排打包），不走旧截断路径。
fn measure(
    renderer: &mut PngRenderer,
    axis: ScriptAxis,
    template: &RenderTextBlock,
    frame: &BubbleFrame,
    font: f32,
    line_height: f32,
) -> (usize, usize) {
    let mut text = template.clone();
    text.set_font_size(font);
    text.set_line_height(line_height);
    let mut color_map = ColorMap::default();
    match layout(renderer, axis, &text, frame, &mut color_map) {
        Some(placed) => (placed.width as usize, placed.height as usize),
        None => (0, 0),
    }
}

/// L3 兜底：各向同性缩小。`INTER_AREA` 高降采样保笔画，`INTER_LINEAR` 会抹掉。
pub fn shrink_isotropic(img: &RawImage, scale: f32) -> anyhow::Result<RawImage> {
    let w = ((img.width as f32 * scale).round() as i32).max(1);
    let h = ((img.height as f32 * scale).round() as i32).max(1);
    let src = img.as_opencv_mat()?;
    let mut dst = Mat::default();
    resize(&src, &mut dst, Size::new(w, h), 0.0, 0.0, INTER_AREA)?;
    Ok(RawImage::try_from(dst)?)
}

/// 一个不可拆开的竖排原子。分段即竖排的全部混排策略。
pub enum LayoutAtom {
    /// 直立，占约 1em 格。
    Upright(char),
    /// 整形后位图顺时针转 90°。
    Rotated(char),
    /// 纵中横：1–2 位 ASCII 数字横排挤进约 1em 格。
    TateChuYoko(String),
    /// 显式换列（`\n`）。
    Break,
}

impl LayoutAtom {
    fn ch(&self) -> Option<char> {
        match self {
            Self::Upright(c) | Self::Rotated(c) => Some(*c),
            _ => None,
        }
    }

    /// 原子整形文本。
    fn shape_text(&self) -> String {
        match self {
            Self::Upright(c) | Self::Rotated(c) => c.to_string(),
            Self::TateChuYoko(s) => s.clone(),
            Self::Break => String::new(),
        }
    }
}

/// 顺时针 90°：括号引号、长音符、浪线、破折、省略号。
const ROTATED_EXTRA: &[char] = &[
    '「', '」', '『', '』', '（', '）', '〔', '〕', '〈', '〉', '《', '》', '【', '】', '〖', '〗',
    '“', '”', '‘', '’', '‹', '›', '«', '»', '〝', '〞', '﹁', '﹂', '﹃', '﹄', '[', ']', '(',
    ')', '{', '}', '—', '–', 'ー', 'ｰ', '〜', '～', '…', '‥',
];

/// 直立字符：CJK、假名、注音、谚文、全角字母数字、问叹、部分全角标点。
/// ASCII `!?` 例外直立（漫画气泡常见）；长音符已在旋转表先行匹配。
fn is_upright(c: char) -> bool {
    matches!(c,
        '!' | '?' | '\u{FF01}' | '\u{FF1F}' | '\u{3001}' | '\u{3002}' | '\u{FF0C}' | '\u{FF0E}'
        | '\u{FF1A}' | '\u{FF1B}' | '\u{00B7}' | '\u{3005}' | '\u{3000}'
        | '\u{3040}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' | '\u{3100}'..='\u{312F}'
        | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2FA1D}' | '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}'
        | '\u{FF10}'..='\u{FF5A}')
}

/// 句读位移：直立但放到 em 格右上（content 偏移 (+0.5,−0.5)em）。
fn is_kuten(c: char) -> bool {
    matches!(c, '\u{3001}' | '\u{3002}' | '\u{FF0C}' | '\u{FF0E}')
}

/// 列首禁则 / 列尾禁则（任务集 + 全角对应形）。
const KINSOKU_HEAD: &[char] = &[
    '\u{3001}', '\u{3002}', ')', '\u{FF09}', '\u{300D}', '\u{300F}', '\u{3011}', '!', '?',
    '\u{FF01}', '\u{FF1F}',
];
const KINSOKU_TAIL: &[char] = &['(', '\u{FF08}', '\u{300C}', '\u{300E}', '\u{3010}'];

/// 一次扫描分段：数字贪心 1–2 位成纵中横，其余查表。
pub fn segment_atoms(text: &str) -> Vec<LayoutAtom> {
    fn flush_digits(digits: &mut String, atoms: &mut Vec<LayoutAtom>) {
        let mut rest = digits.as_str();
        while rest.len() > 2 {
            atoms.push(LayoutAtom::TateChuYoko(rest[..2].to_owned()));
            rest = &rest[2..];
        }
        if !rest.is_empty() {
            atoms.push(LayoutAtom::TateChuYoko(rest.to_owned()));
        }
        digits.clear();
    }

    let mut atoms = Vec::new();
    let mut digits = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        flush_digits(&mut digits, &mut atoms);
        match c {
            '\n' => atoms.push(LayoutAtom::Break),
            '\r' => {}
            ' ' | '\t' => atoms.push(LayoutAtom::Upright('\u{3000}')),
            c if ROTATED_EXTRA.contains(&c) => atoms.push(LayoutAtom::Rotated(c)),
            c if is_upright(c) => atoms.push(LayoutAtom::Upright(c)),
            c if c.is_ascii_graphic() => atoms.push(LayoutAtom::Rotated(c)),
            c if c.is_alphabetic() => atoms.push(LayoutAtom::Rotated(c)),
            c if c.is_control() => {}
            _ => atoms.push(LayoutAtom::Upright(c)),
        }
    }
    flush_digits(&mut digits, &mut atoms);
    atoms
}

/// 已定位字形：横竖共用的光栅化输入。
pub struct PlacedGlyph {
    pub cache_key: cosmic_text::CacheKey,
    /// 直立：位图左上 canvas 坐标；旋转：右缘锚点（光栅化 `x − px`，推导见打包注释）。
    pub x: i32,
    pub y: i32,
    pub rotate_90_cw: bool,
    pub color: cosmic_text::Color,
    pub metadata: usize,
    pub is_whitespace: bool,
}

pub struct PlacedLayout {
    pub width: u32,
    pub height: u32,
    pub glyphs: Vec<PlacedGlyph>,
    /// 竖排列数 / 横排行数（渲染报告消费）。
    #[allow(dead_code)]
    pub cols: usize,
}

/// 按轴排版，返回扁平已定位字形列表。空文本返回 `None`。
pub fn layout(
    renderer: &mut PngRenderer,
    axis: ScriptAxis,
    template: &RenderTextBlock,
    frame: &BubbleFrame,
    color_map: &mut ColorMap,
) -> Option<PlacedLayout> {
    match axis {
        ScriptAxis::Horizontal => layout_horizontal(renderer, template),
        ScriptAxis::VerticalLtr => {
            let t = template.texts.first()?;
            let bg_id = color_map
                .get_id(t.bg_color.unwrap_or((255, 255, 255)))
                .unwrap();
            pack_vertical(
                renderer,
                &t.text,
                frame,
                t.font_size,
                t.line_height,
                t.color.unwrap_or((0, 0, 0)),
                bg_id,
                &t.family,
                color_map,
            )
        }
    }
}

/// 横排：cosmic 折行，收齐字形并减掉对齐行偏移。
pub fn layout_horizontal(
    renderer: &mut PngRenderer,
    template: &RenderTextBlock,
) -> Option<PlacedLayout> {
    let mut color_map = ColorMap::default();
    let buffer = renderer.create_buffer(template, &mut color_map);
    let runs = buffer.layout_runs().collect::<Vec<_>>();
    let (w, h) = wh(&runs);
    let wrap_w = template.size.0 as f32;
    let mut glyphs = Vec::new();
    for run in &runs {
        let shift = match template.align {
            cosmic_text::Align::Center => (wrap_w - run.line_w) / 2.0,
            cosmic_text::Align::Right | cosmic_text::Align::End => wrap_w - run.line_w,
            _ => 0.0,
        }
        .round() as i32;
        for g in run.glyphs.iter() {
            let physical = g.physical((0., 0.), 1.0);
            glyphs.push(PlacedGlyph {
                cache_key: physical.cache_key,
                x: physical.x - shift,
                y: run.line_y as i32 + physical.y,
                rotate_90_cw: false,
                color: g.color_opt.unwrap_or(cosmic_text::Color::rgb(0, 0, 0)),
                metadata: g.metadata,
                is_whitespace: run
                    .text
                    .get(g.start..g.end)
                    .is_some_and(|s| s.chars().all(|c| c.is_whitespace())),
            });
        }
    }
    if glyphs.is_empty() {
        return None;
    }
    Some(PlacedLayout {
        width: w as u32,
        height: h as u32,
        cols: runs.len(),
        glyphs,
    })
}

struct ShapedGlyph {
    cache_key: cosmic_text::CacheKey,
    gx: i32,
    gy: i32,
    color: cosmic_text::Color,
    metadata: usize,
    is_whitespace: bool,
}

struct ShapedAtom {
    glyphs: Vec<ShapedGlyph>,
    w: f32,
    h: f32,
}

/// 单原子整形：cosmic 只当字形器，几何由打包控制。
fn shape_atom(
    renderer: &mut PngRenderer,
    text: &str,
    font_px: f32,
    fg: (u8, u8, u8),
    bg_id: usize,
    family: &Option<String>,
) -> ShapedAtom {
    use cosmic_text::{Attrs, Buffer, Metrics, Shaping};
    let mut attrs = Attrs::new()
        .color(cosmic_text::Color::rgb(fg.0, fg.1, fg.2))
        .metrics(Metrics::new(font_px, font_px))
        .metadata(bg_id);
    if let Some(family) = family {
        attrs = attrs.family(cosmic_text::Family::Name(family));
    }
    let metrics = Metrics::new(font_px, font_px);
    let mut buffer = Buffer::new(&mut renderer.font_system, metrics);
    {
        let mut b = buffer.borrow_with(&mut renderer.font_system);
        b.set_size(None, None);
        b.set_text(text, &attrs, Shaping::Advanced);
        b.shape_until_scroll(true);
    }
    let mut glyphs = Vec::new();
    let (mut aw, mut ah) = (0.0f32, 0.0f32);
    struct Raw {
        cache_key: cosmic_text::CacheKey,
        ax: i32,
        ay: i32,
        color: cosmic_text::Color,
        metadata: usize,
        is_whitespace: bool,
    }
    let mut raws = Vec::new();
    for run in buffer.layout_runs() {
        aw = aw.max(run.line_w);
        ah = ah.max(run.line_top + run.line_height);
        for g in run.glyphs.iter() {
            let physical = g.physical((0., 0.), 1.0);
            raws.push(Raw {
                cache_key: physical.cache_key,
                // 与横排同一锚定：基线 line_y + 物理偏移。with_pixels 的像素坐标
                // 含位图自身偏移（常为负），缺了基线字形会整体落在画布上方。
                ax: physical.x,
                ay: run.line_y as i32 + physical.y,
                color: g.color_opt.unwrap_or(cosmic_text::Color::rgb(0, 0, 0)),
                metadata: g.metadata,
                is_whitespace: text
                    .get(g.start..g.end)
                    .is_some_and(|s| s.chars().all(|c| c.is_whitespace())),
            });
        }
    }
    // 内容盒取墨水包络而非行盒：字形在格内按墨水居中（一横居中而非沉底），
    // 且与字体 ascent 无关。空白原子无墨水，按 advance 占位。
    let mut ink: Option<(i32, i32, i32, i32)> = None;
    for r in &raws {
        if let Some(image) = renderer.cache.get_image(&mut renderer.font_system, r.cache_key) {
            let (x0, y0) = (
                r.ax + image.placement.left,
                r.ay - image.placement.top,
            );
            let (x1, y1) = (
                x0 + image.placement.width as i32,
                y0 + image.placement.height as i32,
            );
            ink = Some(match ink {
                Some((ix0, iy0, ix1, iy1)) => {
                    (ix0.min(x0), iy0.min(y0), ix1.max(x1), iy1.max(y1))
                }
                None => (x0, y0, x1, y1),
            });
        }
    }
    match ink {
        Some((x0, y0, x1, y1)) if x1 > x0 && y1 > y0 => {
            for r in &raws {
                glyphs.push(ShapedGlyph {
                    cache_key: r.cache_key,
                    gx: r.ax - x0,
                    gy: r.ay - y0,
                    color: r.color,
                    metadata: r.metadata,
                    is_whitespace: r.is_whitespace,
                });
            }
            ShapedAtom {
                glyphs,
                w: (x1 - x0) as f32,
                h: (y1 - y0) as f32,
            }
        }
        _ => {
            for r in &raws {
                glyphs.push(ShapedGlyph {
                    cache_key: r.cache_key,
                    gx: r.ax,
                    gy: r.ay,
                    color: r.color,
                    metadata: r.metadata,
                    is_whitespace: r.is_whitespace,
                });
            }
            ShapedAtom {
                glyphs,
                w: aw.max(1.0),
                h: ah.max(1.0),
            }
        }
    }
}

/// 竖排打包：列内沿 y 堆叠，满列换列，列沿 +x（左到右）。
#[allow(clippy::too_many_arguments)]
pub fn pack_vertical(
    renderer: &mut PngRenderer,
    text: &str,
    frame: &BubbleFrame,
    font_px: f32,
    line_height: f32,
    fg: (u8, u8, u8),
    bg_id: usize,
    family: &Option<String>,
    _color_map: &mut ColorMap,
) -> Option<PlacedLayout> {
    let atoms = segment_atoms(text);
    // 分列：Break 强制换列；列内 y 累加超预算换列。
    let em = font_px;
    let col_h = frame.height_px as f32;
    let mut cols: Vec<Vec<usize>> = vec![Vec::new()];
    let mut y = 0.0;
    for (i, atom) in atoms.iter().enumerate() {
        if matches!(atom, LayoutAtom::Break) {
            if !cols.last().is_some_and(|c| c.is_empty()) {
                cols.push(Vec::new());
            }
            y = 0.0;
            continue;
        }
        if y + em > col_h && y > 0.0 {
            cols.push(Vec::new());
            y = 0.0;
        }
        cols.last_mut().unwrap().push(i);
        y += em;
    }
    while cols.first().is_some_and(|c| c.is_empty()) {
        cols.remove(0);
    }
    while cols.last().is_some_and(|c| c.is_empty()) {
        cols.pop();
    }
    if cols.is_empty() {
        return None;
    }
    apply_kinsoku(&mut cols, &atoms);

    // 同字复用整形结果（fit 二分会反复打包）。
    let mut shaped_cache: std::collections::HashMap<String, ShapedAtom> =
        std::collections::HashMap::new();
    let mut shaped: Vec<Option<ShapedAtom>> = Vec::with_capacity(atoms.len());
    for atom in &atoms {
        if matches!(atom, LayoutAtom::Break) {
            shaped.push(None);
            continue;
        }
        let key = atom.shape_text();
        if !shaped_cache.contains_key(&key) {
            let s = shape_atom(renderer, &key, font_px, fg, bg_id, family);
            shaped_cache.insert(key.clone(), s);
        }
        let s = &shaped_cache[&key];
        shaped.push(Some(ShapedAtom {
            glyphs: s
                .glyphs
                .iter()
                .map(|g| ShapedGlyph {
                    cache_key: g.cache_key,
                    gx: g.gx,
                    gy: g.gy,
                    color: g.color,
                    metadata: g.metadata,
                    is_whitespace: g.is_whitespace,
                })
                .collect(),
            w: s.w,
            h: s.h,
        }));
    }

    let col_pitch = em * line_height;
    let rows = cols.iter().map(|c| c.len()).max().unwrap_or(0);
    let mut glyphs = Vec::new();
    for (k, col) in cols.iter().enumerate() {
        for (j, &i) in col.iter().enumerate() {
            let atom = &atoms[i];
            let s = shaped[i].as_ref().unwrap();
            let (cx, cy) = (k as f32 * col_pitch, j as f32 * em);
            let rotated = matches!(atom, LayoutAtom::Rotated(_));
            // 内容在格内居中；旋转后内容尺寸为 (h, w)。
            let (cw, ch) = if rotated { (s.h, s.w) } else { (s.w, s.h) };
            let mut ox = (col_pitch - cw) / 2.0;
            let mut oy = (em - ch) / 2.0;
            if matches!(atom.ch(), Some(c) if is_kuten(c)) {
                ox += em * 0.5;
                oy -= em * 0.5;
            }
            for g in &s.glyphs {
                // 顺时针 90°：content(u,v) → (h−1−v, u)。
                // 存右缘锚点 X = ox + h − 1 − gy，光栅化时 dest = (X − px, Y + px)，
                // 不需要字形位图尺寸即可旋转。
                let (x, y) = if rotated {
                    (
                        (cx + ox + s.h - 1.0 - g.gy as f32).round() as i32,
                        (cy + oy + g.gx as f32).round() as i32,
                    )
                } else {
                    (
                        (cx + ox + g.gx as f32).round() as i32,
                        (cy + oy + g.gy as f32).round() as i32,
                    )
                };
                glyphs.push(PlacedGlyph {
                    cache_key: g.cache_key,
                    x,
                    y,
                    rotate_90_cw: rotated,
                    color: g.color,
                    metadata: g.metadata,
                    is_whitespace: g.is_whitespace,
                });
            }
        }
    }
    Some(PlacedLayout {
        width: (cols.len() as f32 * col_pitch).ceil() as u32,
        height: (rows as f32 * em).ceil() as u32,
        cols: cols.len(),
        glyphs,
    })
}

/// 最小禁则：只在相邻列间移动一个单元，不连锁。
fn apply_kinsoku(cols: &mut [Vec<usize>], atoms: &[LayoutAtom]) {
    for k in 0..cols.len().saturating_sub(1) {
        let head_forbidden = cols[k + 1]
            .first()
            .and_then(|&i| atoms[i].ch())
            .is_some_and(|c| KINSOKU_HEAD.contains(&c));
        if head_forbidden && !cols[k].is_empty() {
            // 列首禁则字拉回上一列末（允许该列超高 1em，由画布如实量出）。
            let i = cols[k + 1].remove(0);
            cols[k].push(i);
            continue;
        }
        let tail_forbidden = cols[k]
            .last()
            .and_then(|&i| atoms[i].ch())
            .is_some_and(|c| KINSOKU_TAIL.contains(&c));
        if tail_forbidden && !cols[k + 1].is_empty() {
            // 列尾禁则字推到下一列首。
            let i = cols[k].pop().unwrap();
            cols[k + 1].insert(0, i);
        }
    }
}

/// 光栅化已定位字形：返回（图像，落笔的非空字形数）。
///
/// 旋转在写像素时做坐标变换；描边仍是 ColorMap + 形态学膨胀，与横排同一路径。
pub fn rasterize_placed(
    renderer: &mut PngRenderer,
    placed: &PlacedLayout,
    font_size: f32,
    color_map: &mut ColorMap,
) -> (RawImage, usize) {
    let (w, h) = (placed.width as usize, placed.height as usize);
    if w == 0 || h == 0 || placed.glyphs.is_empty() {
        return (empty_rgba(), 0);
    }
    let mut rgb = vec![[0_u8; 4]; w * h];
    let mut bg = vec![0_u8; w * h];
    let mut written = 0;
    for g in &placed.glyphs {
        let mut touched = false;
        renderer.cache.with_pixels(
            &mut renderer.font_system,
            g.cache_key,
            g.color,
            |px, py, color| {
                let (dx, dy) = if g.rotate_90_cw {
                    (g.x - px, g.y + py)
                } else {
                    (g.x + px, g.y + py)
                };
                let a = color.a();
                if a == 0 || dx < 0 || dy < 0 || dx >= w as i32 || dy >= h as i32 {
                    return;
                }
                let (dx, dy) = (dx as usize, dy as usize);
                rgb[dy * w + dx] = [color.r(), color.g(), color.b(), a];
                touched = true;
                if a >= 127 {
                    bg[dy * w + dx] = g.metadata as u8;
                }
            },
        );
        if touched && !g.is_whitespace {
            written += 1;
        }
    }
    (stroke_and_composite(rgb, bg, w, h, font_size, color_map), written)
}

/// 描边合成：bg 掩膜膨胀为描边，与前景叠合（横竖共用）。
fn stroke_and_composite(
    rgb: Vec<[u8; 4]>,
    bg: Vec<u8>,
    w: usize,
    h: usize,
    font_size: f32,
    color_map: &ColorMap,
) -> RawImage {
    use interface_image::Mask;
    let src = Mat::from_slice(&bg).unwrap();
    let src = src.reshape(1, h as i32).unwrap();
    let mut dst = Mat::default();
    opencv::imgproc::dilate(
        &src,
        &mut dst,
        &backdrop_kernel(font_size as i32).unwrap(),
        opencv::core::Point::new(-1, -1),
        1,
        opencv::core::BORDER_CONSTANT,
        opencv::imgproc::morphology_default_border_value().unwrap(),
    )
    .unwrap();
    let stroked = color_map.to_image(Mask::from(dst));
    let len = rgb.len() * 4;
    let cap = rgb.capacity() * 4;
    let ptr = rgb.as_ptr() as *mut u8;
    std::mem::forget(rgb);
    let flat: Vec<u8> = unsafe { Vec::from_raw_parts(ptr, len, cap) };
    let text = RawImage {
        width: w as u16,
        height: h as u16,
        data: flat,
        channels: 4,
    };
    stroked.apply(text)
}

#[cfg(test)]
mod tests {
    use cosmic_text::{Align, Style};
    use interface_detector::textlines::MyPoint;
    use interface_translator::LangIdDetector;

    use super::*;
    use crate::{RenderDirection, Text, pad_to_aspect};

    fn block(lines: Vec<[MyPoint; 4]>, font_size: u64, angle: f64) -> TextBlock {
        let n = lines.len().max(1);
        let det = LangIdDetector::new().unwrap();
        TextBlock::new(
            lines,
            vec!["src".to_owned(); n],
            font_size,
            angle,
            1.0,
            None,
            None,
            &det,
        )
    }

    fn rect(x: i64, y: i64, w: i64, h: i64) -> [MyPoint; 4] {
        [
            MyPoint { x, y },
            MyPoint { x: x + w, y },
            MyPoint { x: x + w, y: y + h },
            MyPoint { x, y: y + h },
        ]
    }

    fn assert_in_image(dest: &[(i64, i64); 4], w: u32, h: u32) {
        for &(x, y) in dest {
            assert!((0..w as i64).contains(&x), "x={x} 越界 w={w}");
            assert!((0..h as i64).contains(&y), "y={y} 越界 h={h}");
        }
    }

    #[test]
    fn frame_matches_min_rect_expanded() {
        // 无倾角内陆块：框包络 == min_rect 包络外扩半字号（±1 舍入）。
        let b = block(vec![rect(100, 100, 200, 60)], 40, 0.0);
        let frame = BubbleFrame::from_block(&b, 1000, 1000).unwrap();
        let mr = b.min_rect().unwrap();
        let (mx0, my0, mx1, my1) = bbox(&mr);
        let (fx0, fy0, fx1, fy1) = bbox(&frame.dest);
        let pad = 20;
        assert!((fx0 - (mx0 - pad)).abs() <= 1, "{fx0} vs {mx0}");
        assert!((fy0 - (my0 - pad)).abs() <= 1, "{fy0} vs {my0}");
        assert!((fx1 - (mx1 + pad)).abs() <= 1, "{fx1} vs {mx1}");
        assert!((fy1 - (my1 + pad)).abs() <= 1, "{fy1} vs {my1}");
    }

    #[test]
    fn edge_block_corners_stay_in_image() {
        // 贴右/底边：平移入界，四角不出图且不退化。
        let b = block(vec![rect(900, 900, 200, 150)], 40, 0.0);
        let frame = BubbleFrame::from_block(&b, 1000, 1000).unwrap();
        assert_in_image(&frame.dest, 1000, 1000);
        assert!(frame.width_px >= 2 && frame.height_px >= 2);
        // 贴左/上边同理。
        let b = block(vec![rect(-30, -20, 200, 150)], 40, 0.0);
        let frame = BubbleFrame::from_block(&b, 1000, 1000).unwrap();
        assert_in_image(&frame.dest, 1000, 1000);
    }

    #[test]
    fn tilted_block_stays_in_image_and_expands() {
        let b = block(vec![rect(400, 400, 200, 60)], 40, 15.0);
        let frame = BubbleFrame::from_block(&b, 1000, 1000).unwrap();
        assert_in_image(&frame.dest, 1000, 1000);
        // 外扩后面积大于 min_rect。
        let (mw, mh) = dest_axes(b.min_rect().unwrap());
        assert!(frame.width_px as f32 * frame.height_px as f32 > mw * mh);
    }

    #[test]
    fn oversized_block_shrinks_preserving_aspect() {
        // 框大于图：等比收缩，长宽比与外扩前一致（舍入内）。
        let b = block(vec![rect(0, 0, 900, 300)], 40, 0.0);
        let frame = BubbleFrame::from_block(&b, 400, 400).unwrap();
        assert_in_image(&frame.dest, 400, 400);
        let expect = (900.0 + 40.0) / (300.0 + 40.0);
        let got = frame.width_px as f32 / frame.height_px as f32;
        assert!((got - expect).abs() < 0.05, "got={got} expect={expect}");
    }

    #[test]
    fn degenerate_block_returns_none() {
        let det = LangIdDetector::new().unwrap();
        let b = TextBlock::new(vec![], vec![], 30, 0.0, 1.0, None, None, &det);
        assert!(BubbleFrame::from_block(&b, 1000, 1000).is_none());
        assert!(BubbleFrame::from_block(&b, 0, 1000).is_none());
        let b = block(vec![rect(10, 10, 50, 20)], 30, 0.0);
        assert!(BubbleFrame::from_block(&b, 0, 0).is_none());
    }

    fn h_template(text: &str, w: usize, h: usize, font: f32, lh: f32) -> RenderTextBlock {
        RenderTextBlock {
            align: Align::Center,
            default_font_size: font,
            default_line_height: lh,
            size: (w, h),
            texts: vec![Text {
                text: text.to_owned(),
                letter_spacing: None,
                color: Some((0, 0, 0)),
                bg_color: Some((255, 255, 255)),
                stretch: None,
                style: Style::Normal,
                weight: None,
                family: None,
                font_size: font,
                line_height: lh,
            }],
        }
    }

    #[test]
    fn narrow_frame_wraps_without_dropping_glyphs() {
        // 窄框长句（无空格 CJK，一字一 glyph）：产出多行，glyph 总数 == 字数。
        let text = "才没有那种事呢".repeat(6);
        let template = h_template(&text, 120, 300, 30.0, 1.2);
        let mut renderer = PngRenderer::default();
        let mut color_map = ColorMap::default();
        let buffer = renderer.create_buffer(&template, &mut color_map);
        let runs = buffer.layout_runs().collect::<Vec<_>>();
        assert!(runs.len() > 1, "runs={}", runs.len());
        let glyphs: usize = runs.iter().map(|r| r.glyphs.len()).sum();
        assert_eq!(glyphs, text.chars().count());
    }

    #[test]
    fn fit_shrinks_only() {
        use textline_merge::ScriptAxis;
        let mut renderer = PngRenderer::default();
        // 装不下：小于 preferred，不小于 min。
        let text = "才没有那种事呢".repeat(6);
        let template = h_template(&text, 120, 90, 30.0, 1.2);
        let frame = test_frame(120, 90);
        let (font, _) = fit_font_px(
            &mut renderer,
            ScriptAxis::Horizontal,
            &template,
            &frame,
            30.0,
            10.0,
            1.2,
        );
        assert!(font < 30.0, "font={font}");
        assert!(font >= 10.0, "font={font}");
        // 装得下：短句不放大。
        let template = h_template("Hi", 500, 200, 30.0, 1.01);
        let frame = test_frame(500, 200);
        let (font, lh) = fit_font_px(
            &mut renderer,
            ScriptAxis::Horizontal,
            &template,
            &frame,
            30.0,
            10.0,
            1.01,
        );
        assert_eq!(font, 30.0);
        assert_eq!(lh, 1.01);
    }

    #[test]
    fn fit_falls_back_to_unit_line_height() {
        use textline_merge::ScriptAxis;
        let mut renderer = PngRenderer::default();
        // 极小框长句：L1/L2 都装不下 → (min, 1.0)，由 L3 兜底。
        let text = "才没有那种事呢".repeat(6);
        let template = h_template(&text, 30, 30, 30.0, 1.2);
        let frame = test_frame(30, 30);
        let (font, lh) = fit_font_px(
            &mut renderer,
            ScriptAxis::Horizontal,
            &template,
            &frame,
            30.0,
            10.0,
            1.2,
        );
        assert_eq!(font, 10.0);
        assert_eq!(lh, 1.0);
        // 行高 1.2 装不下、1.0 装得下 → L2 生效。
        let template = h_template(&"啊".repeat(9), 35, 32, 30.0, 1.2);
        let frame = test_frame(35, 32);
        let (font, lh) = fit_font_px(
            &mut renderer,
            ScriptAxis::Horizontal,
            &template,
            &frame,
            30.0,
            10.0,
            1.2,
        );
        assert_eq!(lh, 1.0);
        assert!(font >= 10.0 && font < 12.0, "font={font}");
    }

    #[test]
    fn pad_centers_content_for_all_aspect_relations() {
        // 内容横/竖 × 目标横/竖：内容中心与画布中心对齐（±1px）。
        for (w, h, aspect) in [(100, 50, 1.0), (100, 50, 4.0), (50, 100, 1.0), (50, 100, 0.25)] {
            let img = RawImage {
                data: vec![0; w * h * 4],
                width: w as u16,
                height: h as u16,
                channels: 4,
            };
            let out = pad_to_aspect(img, aspect);
            let (ow, oh) = (out.width as f32, out.height as f32);
            assert!((ow / oh - aspect).abs() < 0.05, "{ow}x{oh} aspect {aspect}");
            let (cw, ch) = (w as f32, h as f32);
            let (ccx, ccy) = ((ow - cw) / 2.0 + cw / 2.0, (oh - ch) / 2.0 + ch / 2.0);
            assert!((ccx - ow / 2.0).abs() <= 1.0, "{ow}x{oh}");
            assert!((ccy - oh / 2.0).abs() <= 1.0, "{ow}x{oh}");
            // 旧贴边 bug：补白必须对称。
            assert_eq!((ow - cw) as i32 % 2, 0);
            assert_eq!((oh - ch) as i32 % 2, 0);
        }
    }

    #[test]
    fn shrink_isotropic_scales_dims() {
        let img = RawImage {
            data: vec![255; 100 * 50 * 4],
            width: 100,
            height: 50,
            channels: 4,
        };
        let out = shrink_isotropic(&img, 0.5).unwrap();
        assert_eq!((out.width, out.height), (50, 25));
    }

    #[test]
    fn direction_resolve_prefers_override() {
        use textline_merge::ScriptAxis;
        assert_eq!(
            RenderDirection::Auto.resolve(ScriptAxis::VerticalLtr),
            ScriptAxis::VerticalLtr
        );
        assert_eq!(
            RenderDirection::Horizontal.resolve(ScriptAxis::VerticalLtr),
            ScriptAxis::Horizontal
        );
        assert_eq!(
            RenderDirection::Vertical.resolve(ScriptAxis::Horizontal),
            ScriptAxis::VerticalLtr
        );
    }

    fn test_frame(w: u32, h: u32) -> BubbleFrame {
        BubbleFrame {
            dest: [
                (0, 0),
                (w as i64, 0),
                (w as i64, h as i64),
                (0, h as i64),
            ],
            width_px: w,
            height_px: h,
        }
    }

    fn pack(text: &str, frame_w: u32, frame_h: u32, font: f32) -> PlacedLayout {
        let mut renderer = PngRenderer::default();
        let mut color_map = ColorMap::default();
        let frame = test_frame(frame_w, frame_h);
        let bg_id = color_map.get_id((255, 255, 255)).unwrap();
        pack_vertical(
            &mut renderer,
            text,
            &frame,
            font,
            1.2,
            (0, 0, 0),
            bg_id,
            &None,
            &mut color_map,
        )
        .unwrap()
    }

    #[test]
    fn segment_classifies_mixed_text() {
        let atoms = segment_atoms("あA\u{30FC}12\n\u{3001}\u{3002}");
        assert_eq!(atoms.len(), 7);
        assert!(matches!(atoms[0], LayoutAtom::Upright('あ')));
        assert!(matches!(atoms[1], LayoutAtom::Rotated('A')));
        assert!(matches!(atoms[2], LayoutAtom::Rotated('\u{30FC}')));
        assert!(matches!(atoms[3], LayoutAtom::TateChuYoko(ref s) if s == "12"));
        assert!(matches!(atoms[4], LayoutAtom::Break));
        assert!(matches!(atoms[5], LayoutAtom::Upright('\u{3001}')));
        assert!(matches!(atoms[6], LayoutAtom::Upright('\u{3002}')));
        // 长数字串按两位切开。
        let atoms = segment_atoms("2026");
        assert_eq!(atoms.len(), 2);
        assert!(matches!(atoms[0], LayoutAtom::TateChuYoko(ref s) if s == "20"));
        assert!(matches!(atoms[1], LayoutAtom::TateChuYoko(ref s) if s == "26"));
    }

    #[test]
    fn vertical_column_flows_down_then_right() {
        // 单列：同 x（墨水居中 ±1px），y 递增。
        let placed = pack("あいうえお", 200, 500, 30.0);
        assert_eq!(placed.cols, 1);
        let xs: Vec<i32> = placed.glyphs.iter().map(|g| g.x).collect();
        assert!(xs.iter().all(|&x| (x - xs[0]).abs() <= 1), "{xs:?}");
        let ys: Vec<i32> = placed.glyphs.iter().map(|g| g.y).collect();
        assert!(ys.windows(2).all(|w| w[1] > w[0]), "{ys:?}");
        // 超一列高：第二列 x 更大（+x，左到右）。
        let placed = pack("あいうえお", 200, 90, 30.0);
        assert_eq!(placed.cols, 2);
        let col1_x = placed.glyphs[0].x;
        let col2_x = placed.glyphs[3].x;
        assert!(col2_x > col1_x, "{col1_x} vs {col2_x}");
    }

    #[test]
    fn vertical_marks_rotation_and_tatechuyoko() {
        let placed = pack("A", 200, 200, 30.0);
        assert_eq!(placed.glyphs.len(), 1);
        assert!(placed.glyphs[0].rotate_90_cw);
        let placed = pack("あ", 200, 200, 30.0);
        assert!(!placed.glyphs[0].rotate_90_cw);
        // 纵中横：两个数字挤进一格高度。
        let placed = pack("12", 200, 200, 30.0);
        assert_eq!(placed.glyphs.len(), 2);
        assert!(placed.glyphs.iter().all(|g| !g.rotate_90_cw));
        let ys: Vec<i32> = placed.glyphs.iter().map(|g| g.y).collect();
        assert!((ys[1] - ys[0]).abs() < 30, "{ys:?}");
    }

    #[test]
    fn kinsoku_moves_single_unit_between_columns() {
        // 列首禁则字拉回上一列末。
        let atoms = segment_atoms("あい\u{3001}う");
        let mut cols = vec![vec![0, 1], vec![2, 3]];
        apply_kinsoku(&mut cols, &atoms);
        assert_eq!(cols, vec![vec![0, 1, 2], vec![3]]);
        // 列尾禁则字推到下一列首。
        let atoms = segment_atoms("あ\u{300C}い");
        let mut cols = vec![vec![0, 1], vec![2]];
        apply_kinsoku(&mut cols, &atoms);
        assert_eq!(cols, vec![vec![0], vec![1, 2]]);
        // 首列列首 / 末列列尾无处可移，保持不动。
        let atoms = segment_atoms("\u{3001}あ");
        let mut cols = vec![vec![0], vec![1]];
        apply_kinsoku(&mut cols, &atoms);
        assert_eq!(cols, vec![vec![0], vec![1]]);
    }

    #[test]
    fn vertical_raster_has_stroke_halo() {
        // CacheKey 与 FontSystem 绑定：整形和光栅化必须用同一 renderer。
        let mut renderer = PngRenderer::default();
        let mut color_map = ColorMap::default();
        let frame = test_frame(200, 200);
        let bg_id = color_map.get_id((255, 255, 255)).unwrap();
        let placed = pack_vertical(
            &mut renderer,
            "あ",
            &frame,
            30.0,
            1.2,
            (0, 0, 0),
            bg_id,
            &None,
            &mut color_map,
        )
        .unwrap();
        let (img, written) = rasterize_placed(&mut renderer, &placed, 30.0, &mut color_map);
        assert_eq!(written, 1);
        // 前景墨水与膨胀后的描边（纯白不透明像素）同时存在。
        assert!(img.data.chunks(4).any(|p| p[3] > 0 && p[0] < 128));
        assert!(
            img.data
                .chunks(4)
                .any(|p| p == [255, 255, 255, 255]),
            "描边掩膜为空"
        );
    }
}
