mod sort;

use std::{
    collections::{HashMap, HashSet},
    f64::consts::PI,
};

use anyhow::anyhow;
use geo::{ConvexHull, Distance as _, Euclidean, MinimumRotatedRect, MultiPoint, Point};
use interface_detector::textlines::{self, MyPoint};
use interface_ocr::QuadrilateralInfo;
use interface_translator::{is_valuable_text, Detector, Language, LanguageWrapper};
use itertools::Itertools as _;
use log::info;
use once_cell::sync::Lazy;
use ordered_float::OrderedFloat;
use petgraph::{
    algo::min_spanning_tree,
    data::Element,
    graph::{NodeIndex, UnGraph},
    Graph,
};
use serde::{Deserialize, Serialize};
use util::text_direction::{connected_components_sets, quadrilateral_can_merge_region};

pub use sort::{simple_sort, sort_panels_fill, sort_regions};

pub fn dispatch(
    textlines: Vec<&QuadrilateralInfo>,
    width: u16,
    height: u16,
    det: &dyn Detector,
) -> anyhow::Result<Vec<TextBlock>> {
    let len = textlines.len();
    let items: anyhow::Result<Vec<TextBlock>> =
        merge_bboxes_text_region(&textlines, width, height)?
            .into_iter()
            .map(|(txtlns, fg_color, bg_color)| {
                let mut total_logprobs = 0.0;
                for txtln in &txtlns {
                    let pos = txtln.pos.lock();
                    total_logprobs += pos.score().ln() * pos.area();
                }

                total_logprobs /= textlines.iter().map(|v| v.pos.lock().area()).sum::<f64>();
                let font_size = txtlns
                    .iter()
                    .map(|v| v.pos.lock().font_size() as u64)
                    .min()
                    .unwrap_or_default();
                let mut angle = mean(txtlns.iter().map(|v| v.pos.lock().angle()))
                    .ok_or(anyhow!("no inputs"))?
                    * (180.0 / std::f64::consts::PI)
                    - 90.0;
                if angle.abs() < 3.0 {
                    angle = 0.0;
                }
                let lines = txtlns
                    .iter()
                    .map(|v| v.pos.lock().pts().clone())
                    .collect::<Vec<_>>();
                let texts = txtlns
                    .into_iter()
                    .map(|v| v.text.clone())
                    .collect::<Vec<_>>();
                let fg_color = match fg_color {
                    (Some(r), Some(g), Some(b)) => Some((r as u8, g as u8, b as u8)),
                    _ => None,
                };
                let bg_color = match bg_color {
                    (Some(r), Some(g), Some(b)) => Some((r as u8, g as u8, b as u8)),
                    _ => None,
                };
                Ok::<_, anyhow::Error>(TextBlock::new(
                    lines,
                    texts,
                    font_size,
                    angle,
                    total_logprobs.exp(),
                    fg_color,
                    bg_color,
                    det,
                ))
            })
            .collect();
    let items: Vec<TextBlock> = items?;
    let new_len = items.iter().map(|v| v.lines.len()).sum::<usize>();
    assert_eq!(len, new_len, "Some areas were removed");
    Ok(items)
}

fn deg2rad(deg: f32) -> f32 {
    deg * std::f32::consts::PI / 180.0
}

fn rotate_polygons(
    center: (i64, i64),
    polygons: Vec<[i64; 8]>,
    rotation: f32,
    new_center: Option<(i64, i64)>,
) -> Vec<[i64; 8]> {
    if rotation == 0.0 {
        return polygons;
    }

    let new_center = match new_center {
        Some(center) => center,
        None => center,
    };
    let rotation = deg2rad(rotation);
    let (s, c) = (rotation.sin(), rotation.cos());
    let (cx, cy) = (center.0 as f32, center.1 as f32);
    let (ncx, ncy) = (new_center.0 as f32, new_center.1 as f32);

    polygons
        .iter()
        .map(|poly| {
            let mut rotated = [0.0f32; 8];

            for i in 0..4 {
                let x = poly[i * 2] as f32 - cx;
                let y = poly[i * 2 + 1] as f32 - cy;

                let new_x = x * c - y * s + ncx;
                let new_y = x * s + y * c + ncy;

                rotated[i * 2] = new_x;
                rotated[i * 2 + 1] = new_y;
            }
            let rotated_i64: [i64; 8] = rotated.map(|x| x.round() as i64);
            rotated_i64
        })
        .collect()
}

pub struct OBB {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// radians
    pub theta: f64,
}

impl OBB {
    pub fn top_left(&self) -> (f64, f64) {
        let hw = self.w / 2.0;
        let hh = self.h / 2.0;
        let cos_t = self.theta.cos();
        let sin_t = self.theta.sin();

        let dx = -hw;
        let dy = -hh;

        let x = self.x + dx * cos_t - dy * sin_t;
        let y = self.y + dx * sin_t + dy * cos_t;

        (x, y)
    }

    pub fn angle_deg(&self) -> f64 {
        self.theta * 180.0 / PI
    }
}

#[derive(Deserialize, Serialize)]
pub struct TextBlock {
    pub lines: Vec<[MyPoint; 4]>,
    pub text: String,
    pub font_size: u64,
    pub angle: f64,
    prob: f64,
    pub fg_color: Option<(u8, u8, u8)>,
    pub bg_color: Option<(u8, u8, u8)>,
    pub skip_translate: bool,
    pub language: Option<LanguageWrapper>,
    pub translations: HashMap<String, String>,
}

impl TextBlock {
    pub fn load(data: &[u8]) -> Option<(Self, usize)> {
        use std::convert::TryInto;
        let mut offset = 0;

        macro_rules! read {
            ($ty:ty) => {{
                if offset + std::mem::size_of::<$ty>() > data.len() {
                    return None;
                }
                let bytes = &data[offset..offset + std::mem::size_of::<$ty>()];
                offset += std::mem::size_of::<$ty>();
                <$ty>::from_le_bytes(bytes.try_into().ok()?)
            }};
        }

        let font_size = read!(u64);
        let angle = read!(f64);
        let prob = read!(f64);

        let skip_translate = {
            if offset >= data.len() {
                return None;
            }
            let v = data[offset];
            offset += 1;
            v == 1
        };

        let fg_color = {
            if offset >= data.len() {
                return None;
            }
            let has = data[offset];
            offset += 1;
            if has == 1 {
                if offset + 3 > data.len() {
                    return None;
                }
                let r = data[offset];
                let g = data[offset + 1];
                let b = data[offset + 2];
                offset += 3;
                Some((r, g, b))
            } else {
                None
            }
        };

        let bg_color = {
            if offset >= data.len() {
                return None;
            }
            let has = data[offset];
            offset += 1;
            if has == 1 {
                if offset + 3 > data.len() {
                    return None;
                }
                let r = data[offset];
                let g = data[offset + 1];
                let b = data[offset + 2];
                offset += 3;
                Some((r, g, b))
            } else {
                None
            }
        };

        if offset + 8 > data.len() {
            return None;
        }
        let text_len = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?) as usize;
        offset += 8;
        if offset + text_len > data.len() {
            return None;
        }
        let text = String::from_utf8_lossy(&data[offset..offset + text_len]).to_string();
        offset += text_len;

        if offset + 8 > data.len() {
            return None;
        }
        let line_count = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?) as usize;
        offset += 8;
        let mut lines = Vec::with_capacity(line_count);
        for _ in 0..line_count {
            let x1 = read!(i64);
            let y1 = read!(i64);
            let x2 = read!(i64);
            let y2 = read!(i64);
            let x3 = read!(i64);
            let y3 = read!(i64);
            let x4 = read!(i64);
            let y4 = read!(i64);
            lines.push([
                textlines::MyPoint { x: x1, y: y1 },
                textlines::MyPoint { x: x2, y: y2 },
                textlines::MyPoint { x: x3, y: y3 },
                textlines::MyPoint { x: x4, y: y4 },
            ]);
        }

        if offset + 8 > data.len() {
            return None;
        }
        let trans_count = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?) as usize;
        offset += 8;
        let mut translations = Vec::with_capacity(trans_count);
        for _ in 0..trans_count {
            if offset + 8 > data.len() {
                return None;
            }
            let key_len = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?) as usize;
            offset += 8;
            if offset + key_len > data.len() {
                return None;
            }
            let key = String::from_utf8_lossy(&data[offset..offset + key_len]).to_string();
            offset += key_len;

            if offset + 8 > data.len() {
                return None;
            }
            let value_len = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?) as usize;
            offset += 8;
            if offset + value_len > data.len() {
                return None;
            }
            let value = String::from_utf8_lossy(&data[offset..offset + value_len]).to_string();
            offset += value_len;

            translations.push((key, value));
        }

        Some((
            Self {
                font_size,
                angle,
                prob,
                skip_translate,
                fg_color,
                bg_color,
                text,
                lines,
                language: None,
                translations: translations.into_iter().collect(),
            },
            offset,
        ))
    }
    pub fn export(self) -> Vec<u8> {
        let mut buffer = vec![];
        buffer.extend(self.font_size.to_le_bytes());
        buffer.extend(self.angle.to_le_bytes());
        buffer.extend(self.prob.to_le_bytes());
        buffer.push(if self.skip_translate { 1 } else { 0 });
        buffer.push(if self.fg_color.is_some() { 1 } else { 0 });
        if let Some(fg_color) = self.fg_color {
            buffer.push(fg_color.0);
            buffer.push(fg_color.1);
            buffer.push(fg_color.2);
        }
        buffer.push(if self.bg_color.is_some() { 1 } else { 0 });
        if let Some(bg_color) = self.bg_color {
            buffer.push(bg_color.0);
            buffer.push(bg_color.1);
            buffer.push(bg_color.2);
        }
        let text = self.text.as_bytes();
        buffer.extend((text.len() as u64).to_le_bytes());
        buffer.extend(text);
        buffer.extend((self.lines.len() as u64).to_le_bytes());
        for line in self.lines {
            buffer.extend(
                line.iter()
                    .flat_map(|v| vec![v.x.to_le_bytes(), v.y.to_le_bytes()])
                    .flatten(),
            );
        }
        buffer.extend((self.translations.len() as u64).to_le_bytes());
        for (key, value) in self.translations {
            let key = key.as_bytes();
            let value = value.as_bytes();
            buffer.extend((key.len() as u64).to_le_bytes());
            buffer.extend(key);
            buffer.extend((value.len() as u64).to_le_bytes());
            buffer.extend(value);
        }
        buffer
    }

    pub fn obb(&self) -> Option<OBB> {
        let points = self
            .lines
            .iter()
            .flat_map(|line| line.iter().map(|p| Point::new(p.x as f64, p.y as f64)))
            .collect::<Vec<_>>();
        if points.is_empty() {
            return None;
        }
        let coords = MultiPoint::new(points)
            .convex_hull()
            .minimum_rotated_rect()?;

        let coords = &coords.exterior().0;
        let w = Euclidean.distance(coords[0], coords[1]);
        let h = Euclidean.distance(coords[1], coords[2]);
        let dx = coords[1].x - coords[0].x;
        let dy = coords[1].y - coords[0].y;
        let center_x = coords.iter().take(4).map(|c| c.x).sum::<f64>() / 4.0;
        let center_y = coords.iter().take(4).map(|c| c.y).sum::<f64>() / 4.0;
        let rotation = dy.atan2(dx);
        Some(OBB {
            x: center_x,
            y: center_y,
            w,
            h,
            theta: rotation,
        })
    }

    pub fn translation(&self) -> Option<&str> {
        let key = self.translations.get("last_trans")?;
        self.translations.get(key).map(String::as_str)
    }

    pub fn with_translation(mut self, lang: impl Into<String>, text: impl Into<String>) -> Self {
        let lang = lang.into();
        self.translations.insert(lang.clone(), text.into());
        self.translations.insert("last_trans".to_owned(), lang);
        self
    }

    pub fn vertical(&self) -> bool {
        let mut max_area = 0.0_f64;
        let mut aspect = 1.0;
        for line in &self.lines {
            let xs = line.map(|p| p.x);
            let ys = line.map(|p| p.y);
            let min_x = xs.iter().copied().min().unwrap_or_default();
            let max_x = xs.iter().copied().max().unwrap_or_default();
            let min_y = ys.iter().copied().min().unwrap_or_default();
            let max_y = ys.iter().copied().max().unwrap_or_default();
            let w = (max_x - min_x) as f64;
            let h = (max_y - min_y) as f64;
            let area = w * h;
            if area > max_area {
                max_area = area;
                aspect = if h > 0.0 { w / h } else { 1.0 };
            }
        }
        if max_area == 0.0 {
            let (x1, y1, x2, y2) = self.xyxy();
            return (x2 - x1) < (y2 - y1);
        }
        aspect < 1.0
    }
    pub fn new(
        lines: Vec<[MyPoint; 4]>,
        texts: Vec<String>,
        font_size: u64,
        angle: f64,
        prob: f64,
        fg_color: Option<(u8, u8, u8)>,
        bg_color: Option<(u8, u8, u8)>,
        det: &dyn Detector,
    ) -> Self {
        let mut iter = texts.iter();
        let mut result = match iter.next() {
            Some(first) => first.clone(),
            None => String::new(),
        };

        for txt in iter {
            let last_char_cjk = result
                .chars()
                .last()
                .is_some_and(|ch| matches!(ch, '\u{3000}'..='\u{9FFF}'));
            let first_char_cjk = txt
                .chars()
                .next()
                .is_some_and(|ch| matches!(ch, '\u{3000}'..='\u{9FFF}'));

            if last_char_cjk || first_char_cjk {
                result.push_str(txt);
            } else {
                result.push(' ');
                result.push_str(txt);
            }
        }
        Self {
            language: det.detect_language(&result).map(LanguageWrapper),
            lines,
            text: result,
            font_size,
            angle,
            prob,
            fg_color,
            bg_color,
            translations: Default::default(),
            skip_translate: false,
        }
    }
}

fn compute_bounds(polygons: &[[i64; 8]]) -> Option<(i64, i64, i64, i64)> {
    if polygons.is_empty() {
        return None;
    }

    let mut min_x = i64::MAX;
    let mut min_y = i64::MAX;
    let mut max_x = i64::MIN;
    let mut max_y = i64::MIN;

    for poly in polygons {
        for (i, &val) in poly.iter().enumerate() {
            if i % 2 == 0 {
                min_x = min_x.min(val);
                max_x = max_x.max(val);
            } else {
                min_y = min_y.min(val);
                max_y = max_y.max(val);
            }
        }
    }

    Some((min_x, min_y, max_x, max_y))
}
impl TextBlock {
    pub fn center(&self) -> (i64, i64) {
        let xyxy = self.xyxy();
        ((xyxy.0 + xyxy.2) / 2, (xyxy.1 + xyxy.3) / 2)
    }

    fn unrotated_polygons(&self) -> Vec<[i64; 8]> {
        let reshaped: Vec<[i64; 8]> = self
            .lines
            .iter()
            .map(|quad| {
                [
                    quad[0].x, quad[0].y, quad[1].x, quad[1].y, quad[2].x, quad[2].y, quad[3].x,
                    quad[3].y,
                ]
            })
            .collect();
        if self.angle != 0.0 {
            rotate_polygons(self.center(), reshaped, self.angle as f32, None)
        } else {
            reshaped
        }
    }

    pub fn min_rect(&self) -> anyhow::Result<[(i64, i64); 4]> {
        let polygons = self.unrotated_polygons();
        let (min_x, min_y, max_x, max_y) =
            compute_bounds(&polygons).ok_or(anyhow::anyhow!("Failed to compute bounds"))?;
        let mut min_bbox = vec![[min_x, min_y, max_x, min_y, max_x, max_y, min_x, max_y]];
        if self.angle != 0.0 {
            min_bbox = rotate_polygons(self.center(), min_bbox, (-self.angle) as f32, None);
        }
        let min_bbox = min_bbox.remove(0);
        let clipped: Vec<i64> = min_bbox.iter().map(|&x| x.max(0)).collect();
        Ok([
            (clipped[0], clipped[1]),
            (clipped[2], clipped[3]),
            (clipped[4], clipped[5]),
            (clipped[6], clipped[7]),
        ])
    }
    pub fn xyxy(&self) -> (i64, i64, i64, i64) {
        let x = self
            .lines
            .iter()
            .flat_map(|v| v.into_iter().map(|v| v.x))
            .collect::<Vec<_>>();
        let y = self
            .lines
            .iter()
            .flat_map(|v| v.into_iter().map(|v| v.y))
            .collect::<Vec<_>>();

        (
            x.iter().min().copied().unwrap_or_default(),
            y.iter().min().copied().unwrap_or_default(),
            x.iter().max().copied().unwrap_or_default(),
            y.iter().max().copied().unwrap_or_default(),
        )
    }
}

fn merge_bboxes_text_region<'a>(
    bboxes: &'a [&'a QuadrilateralInfo],
    width: u16,
    height: u16,
) -> anyhow::Result<
    Vec<(
        Vec<&'a QuadrilateralInfo>,
        (Option<f64>, Option<f64>, Option<f64>),
        (Option<f64>, Option<f64>, Option<f64>),
    )>,
> {
    let mut graph: Graph<usize, (), petgraph::Undirected> = Graph::new_undirected();

    // step 1: divide into multiple text region candidates
    for (i, _) in bboxes.iter().enumerate() {
        graph.add_node(i);
    }
    for ((u, ubox), (v, vbox)) in bboxes.iter().enumerate().tuple_combinations() {
        if quadrilateral_can_merge_region(
            &*ubox.pos.lock(),
            &*vbox.pos.lock(),
            1.9,
            2.0,
            1.0,
            3.0,
            2.0,
            1.3,
        ) {
            graph.add_edge((u as u32).into(), (v as u32).into(), ());
        }
    }

    // step 2: postprocess - further split each region
    let region_indices = connected_components_sets(&graph)
        .into_iter()
        .map(|v| split_text_region(&bboxes, v, width, height, 0.5, 2.0))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    // step 3: return regions
    let v = region_indices
        .into_iter()
        .map(|node_set| {
            //TODO: should vertical or assigned_vertical be used?
            let mut nodes = node_set.into_iter().collect::<Vec<_>>();
            let txtlns = nodes.iter().map(|v| &bboxes[v.index()]).collect::<Vec<_>>();
            let fg_r = mean(txtlns.iter().filter_map(|v| v.fg.map(|v| v[0] as f64)));
            let fg_g = mean(txtlns.iter().filter_map(|v| v.fg.map(|v| v[1] as f64)));
            let fg_b = mean(txtlns.iter().filter_map(|v| v.fg.map(|v| v[2] as f64)));
            let bg_r = mean(txtlns.iter().filter_map(|v| v.bg.map(|v| v[0] as f64)));
            let bg_g = mean(txtlns.iter().filter_map(|v| v.bg.map(|v| v[1] as f64)));
            let bg_b = mean(txtlns.iter().filter_map(|v| v.bg.map(|v| v[2] as f64)));
            let vert = txtlns
                .iter()
                .map(|v| v.pos.lock().vertical() as u64)
                .sum::<u64>();
            let count = txtlns.len() as u64;
            let vertical = if vert == count {
                true
            } else if vert * 2 == count {
                let mut max_aspect_ratio = -100.0;
                let mut lvert = true;
                for boxx in txtlns {
                    let baspect = boxx.pos.lock().aspect_ratio();
                    if baspect > max_aspect_ratio {
                        max_aspect_ratio = baspect;
                        lvert = boxx.pos.lock().vertical();
                    }
                    if 1.0 / baspect > max_aspect_ratio {
                        max_aspect_ratio = 1.0 / baspect;
                        lvert = boxx.pos.lock().vertical();
                    }
                }
                lvert
            } else if vert * 2 > count {
                true
            } else {
                false
            };
            if vertical {
                nodes.sort_by_key(|a| OrderedFloat(-bboxes[a.index()].pos.lock().centroid().x));
            } else {
                nodes.sort_by_key(|a| OrderedFloat(bboxes[a.index()].pos.lock().centroid().y));
            }
            let txtlns = nodes.iter().map(|v| bboxes[v.index()]).collect::<Vec<_>>();

            (txtlns, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b))
        })
        .collect::<Vec<_>>();
    Ok(v)
}

fn mean<I>(iter: I) -> Option<f64>
where
    I: Iterator<Item = f64>,
{
    let (sum, count) = iter.fold((0.0, 0), |(s, c), v| (s + v, c + 1));
    if count == 0 {
        return None;
    }
    Some(sum / count as f64)
}

fn stddev(values: &[f64]) -> Option<f64> {
    let len = values.len();
    if len == 0 {
        return None;
    }

    let mean = values.iter().sum::<f64>() / len as f64;
    let variance = values.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / len as f64;

    Some(variance.sqrt())
}

fn split_text_region(
    bboxes: &[&QuadrilateralInfo],
    connected_region_indices: Vec<NodeIndex>,
    width: u16,
    height: u16,
    gamma: f64,
    sigma: f64,
) -> anyhow::Result<Vec<HashSet<NodeIndex>>> {
    if connected_region_indices.len() == 1 {
        return Ok(vec![connected_region_indices.into_iter().collect()]);
    }

    if connected_region_indices.len() == 2 {
        let fb = &bboxes[connected_region_indices[0].index()];
        let sb = &bboxes[connected_region_indices[1].index()];
        let fs1 = fb.pos.lock().font_size();
        let fs2 = sb.pos.lock().font_size();
        let fs = fs1.max(fs2);

        if fb.pos.lock().distance(&sb.pos.lock(), 0.5) < (1.0 + gamma) * fs
            && (fb.pos.lock().angle() - sb.pos.lock().angle()).abs() < 0.2 * PI
        {
            return Ok(vec![connected_region_indices.into_iter().collect()]);
        } else {
            return Ok(vec![
                vec![connected_region_indices[0]].into_iter().collect(),
                vec![connected_region_indices[1]].into_iter().collect(),
            ]);
        }
    }

    let mut graph: Graph<usize, f64, petgraph::Undirected> = UnGraph::new_undirected();
    let mut map = HashMap::new();
    for bbox in connected_region_indices.iter() {
        let idx = graph.add_node(bbox.index());
        map.insert(bbox.index(), idx);
    }
    for (u, v) in connected_region_indices.iter().tuple_combinations() {
        let weight = bboxes[u.index()]
            .pos
            .lock()
            .distance(&*bboxes[v.index()].pos.lock(), 0.5);
        graph.add_edge(
            *map.get(&u.index()).unwrap(),
            *map.get(&v.index()).unwrap(),
            weight,
        );
    }

    let edges: Vec<_> = min_spanning_tree(&graph).collect();
    let idx_map = edges
        .iter()
        .filter_map(|v| match v {
            Element::Node { weight } => Some(*weight),
            Element::Edge { .. } => None,
        })
        .enumerate()
        .collect::<HashMap<_, _>>();

    let mut edges = edges
        .into_iter()
        .filter_map(|el| match el {
            Element::Edge {
                weight,
                source,
                target,
            } => Some((
                *idx_map.get(&source).unwrap(),
                *idx_map.get(&target).unwrap(),
                weight,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    edges.sort_by(|a, b| OrderedFloat(b.2).cmp(&OrderedFloat(a.2)));
    let distances_sorted = edges.iter().map(|v| v.2).collect::<Vec<_>>();

    let fontsize = mean(
        connected_region_indices
            .iter()
            .map(|idx| (bboxes[idx.index()]).pos.lock().font_size()),
    )
    .unwrap();

    let distances_mean =
        mean(distances_sorted.iter().cloned()).ok_or(anyhow!("distances_sorted is empty"))?;
    let distances_std = stddev(&distances_sorted).ok_or(anyhow!("distances_sorted is empty"))?;
    let std_threshold = f64::max(0.3 * fontsize + 5.0, 5.0);
    let (b1, b2) = (&bboxes[edges[0].0], &bboxes[edges[0].1]);
    let max_poly_distance = b1.pos.lock().poly_distance(&b2.pos.lock());
    let b1_centroid = b1.pos.lock().centroid();
    let b2_centroid = b2.pos.lock().centroid();
    let max_centroid_alignment = f64::max(
        (b1_centroid.x - b2_centroid.x).abs(),
        (b1_centroid.y - b2_centroid.y).abs(),
    );
    if (distances_sorted[0] <= distances_mean + distances_std * sigma
        || distances_sorted[0] <= fontsize * (1.0 + gamma))
        && (distances_std < std_threshold
            || max_poly_distance == 0.0 && max_centroid_alignment < 5.0)
    {
        Ok(vec![connected_region_indices.into_iter().collect()])
    } else {
        let mut graph: Graph<usize, (), petgraph::Undirected> = Graph::new_undirected();
        let mut map = HashMap::new();

        for idx in connected_region_indices.clone() {
            map.insert(idx.index(), graph.add_node(idx.index()));
        }

        // Split out the most deviating bbox
        for edge in &edges[1..] {
            graph.add_edge(*map.get(&edge.0).unwrap(), *map.get(&edge.1).unwrap(), ());
        }
        let map = map
            .into_iter()
            .map(|(k, v)| (v, k))
            .collect::<HashMap<NodeIndex, usize>>();

        Ok(connected_components_sets(&graph)
            .into_iter()
            .map(|node_set| {
                split_text_region(
                    bboxes,
                    node_set
                        .into_iter()
                        .map(|v| (*&map[&v] as u32).into())
                        .collect::<Vec<NodeIndex>>(),
                    width,
                    height,
                    gamma,
                    sigma,
                )
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect())
    }
}

pub static BRACKET_PAIRS: Lazy<HashMap<char, char>> = Lazy::new(|| {
    HashMap::from([
        ('(', ')'),
        ('（', '）'),
        ('[', ']'),
        ('【', '】'),
        ('{', '}'),
        ('〔', '〕'),
        ('〈', '〉'),
        ('「', '」'),
        ('"', '"'),
        ('＂', '＂'),
        ('\'', '\''),
        ('“', '”'),
        ('《', '》'),
        ('『', '』'),
        ('〝', '〞'),
        ('﹁', '﹂'),
        ('﹃', '﹄'),
        ('⸂', '⸃'),
        ('⸄', '⸅'),
        ('⸉', '⸊'),
        ('⸌', '⸍'),
        ('⸜', '⸝'),
        ('⸠', '⸡'),
        ('‹', '›'),
        ('«', '»'),
        ('＜', '＞'),
        ('<', '>'),
    ])
});

pub static LEFT_SYMBOLS: Lazy<HashSet<char>> =
    Lazy::new(|| BRACKET_PAIRS.keys().cloned().collect());

pub static RIGHT_SYMBOLS: Lazy<HashSet<char>> =
    Lazy::new(|| BRACKET_PAIRS.values().cloned().collect());

fn has_brackets(stripped_text: &str) -> bool {
    stripped_text
        .chars()
        .any(|c| LEFT_SYMBOLS.contains(&c) || RIGHT_SYMBOLS.contains(&c))
}
pub fn dispatch_main(
    textlines: &[QuadrilateralInfo],
    width: u16,
    height: u16,
    min_text_length: usize,
    prob_thesh: f64,
    skip_languages: Vec<Language>,
    remove_text: &Vec<String>,
    det: &dyn Detector,
) -> anyhow::Result<Vec<TextBlock>> {
    let textlines = textlines
        .iter()
        .filter(|v| v.prob >= prob_thesh)
        .collect::<Vec<_>>();
    let text_regions = dispatch(textlines, width, height, det)?;
    Ok(text_regions
        .into_iter()
        .filter_map(|mut region| {
            let original_text = region.text;
            let stripped_text = original_text.trim();
            if original_text.len() != stripped_text.len() {
                info!("Removed leading characters from \"{original_text}\"");
            }
            let has_brackets = has_brackets(stripped_text);
            let stripped_text = if has_brackets {
                remove_leading_spaces_after_predict(stripped_text)
            } else {
                stripped_text.to_owned()
            };
            if stripped_text.len() < min_text_length {
                info!("Filtered out: {}", stripped_text);
                info!("Reason: Text length is less than the minimum required length.");
                None
            } else if !is_valuable_text(&stripped_text) {
                info!("Filtered out: {}", stripped_text);
                info!("Reason: Text is not considered valuable.");
                None
            } else if remove_text.contains(&stripped_text) {
                info!("Filtered out: {}", stripped_text);
                info!("Reason: Text contains unwanted content.");
                None
            } else {
                if let Some(language) = region.language {
                    if skip_languages.contains(&language.0) {
                        info!("Filtered out for translation: {}", stripped_text);
                        info!("Reason: Language should not be translated.");
                        region.skip_translate = true;
                    }
                }
                region.text = stripped_text;
                Some(region)
            }
        })
        .collect())
}

fn remove_leading_spaces_after_predict(stripped_text: &str) -> String {
    let mut stack = vec![];
    let mut result_chars = vec![];
    let mut to_skip = vec![];
    // First traversal: mark matching brackets
    for (i, char) in stripped_text.chars().enumerate() {
        if LEFT_SYMBOLS.contains(&char) {
            stack.push((i, char));
        } else if RIGHT_SYMBOLS.contains(&char) {
            if stack.is_empty() {
                // No corresponding left parenthesis, marked for deletion
                to_skip.push(i);
            } else {
                // There is a corresponding left bracket, pop the stack
                stack.pop();
            }
        }
    }

    // Mark unmatched left brackets as delete
    for (pos, _) in stack {
        to_skip.push(pos);
    }
    let mut stack = vec![];

    let has_removed_symbols = to_skip.len() > 0;
    for (i, char) in stripped_text.chars().enumerate() {
        if to_skip.contains(&i) {
            // Skip isolated parentheses
            continue;
        }
        if LEFT_SYMBOLS.contains(&char) {
            stack.push(char);
            result_chars.push(char);
        } else if RIGHT_SYMBOLS.contains(&char) {
            if let Some(left_bracket) = stack.pop() {
                let expected_right = *BRACKET_PAIRS.get(&left_bracket).unwrap();
                if char != expected_right {
                    //Replace mismatched right brackets with the correct right brackets corresponding to the left brackets
                    result_chars.push(expected_right);
                    info!(
                        "Fixed mismatched bracket: replaced \"{}\" with \"{}\"",
                        char, expected_right
                    );
                } else {
                    result_chars.push(char);
                }
            }
        } else {
            result_chars.push(char);
        }
    }
    let new_stripped_text = String::from_iter(result_chars);
    if has_removed_symbols {
        info!("Removed unpaired bracket from \"{}\"", stripped_text);
    }
    if new_stripped_text != stripped_text && !has_removed_symbols {
        info!(
            "Fixed brackets: \"{}\" → \"{}\"",
            stripped_text, new_stripped_text
        );
    }
    new_stripped_text
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use interface_detector::textlines::Quadrilateral;
    use interface_ocr::QuadrilateralInfo;
    use interface_translator::LangIdDetector;

    use crate::dispatch;

    #[test]
    fn testssss() {
        let v: Vec<_> = [
            [[412_i64, 4634], [591, 4634], [591, 4707], [412, 4707]],
            [[585, 1987], [795, 1991], [794, 2067], [584, 2063]],
            [[219, 5258], [445, 5262], [444, 5338], [218, 5334]],
            [[204, 4492], [437, 4492], [437, 4568], [204, 4568]],
            [[236, 2976], [542, 2976], [542, 3050], [236, 3050]],
            [[670, 2263], [970, 2262], [971, 2333], [670, 2335]],
            [[670, 2121], [968, 2121], [968, 2186], [670, 2186]],
            [[434, 5499], [806, 5499], [806, 5564], [434, 5564]],
            [[173, 2908], [603, 2908], [603, 2973], [173, 2973]],
            [[277, 4708], [722, 4712], [722, 4777], [277, 4773]],
            [[583, 2194], [1063, 2194], [1063, 2258], [583, 2258]],
            [[136, 3052], [640, 3052], [640, 3117], [136, 3117]],
            [[201, 440], [728, 440], [728, 500], [201, 500]],
            [[334, 5424], [912, 5428], [912, 5493], [334, 5489]],
        ]
        .into_iter()
        .map(|v| Quadrilateral::new(v.into_iter().map(|v| (v[0], v[1])).collect(), 1.0))
        .map(|v| QuadrilateralInfo {
            text: "".to_string(),
            fg: None,
            bg: None,
            pos: Arc::new(v.into()),
            prob: 1.0,
        })
        .collect();
        let d = LangIdDetector::new().unwrap();
        let out = dispatch(v.iter().collect::<Vec<_>>(), 1080, 6587, &d).unwrap();
        assert_eq!(out.len(), 9)
    }

    #[test]
    fn obb_uses_line_points() {
        let block = crate::TextBlock {
            lines: vec![[
                interface_detector::textlines::MyPoint { x: 100, y: 40 },
                interface_detector::textlines::MyPoint { x: 180, y: 40 },
                interface_detector::textlines::MyPoint { x: 180, y: 70 },
                interface_detector::textlines::MyPoint { x: 100, y: 70 },
            ]],
            text: "hi".into(),
            font_size: 16,
            angle: 0.0,
            prob: 1.0,
            fg_color: None,
            bg_color: None,
            skip_translate: false,
            language: None,
            translations: Default::default(),
        };
        let obb = block.obb().unwrap();
        assert!((obb.x - 140.0).abs() < 1.0, "x={}", obb.x);
        assert!((obb.y - 55.0).abs() < 1.0, "y={}", obb.y);
        assert!(obb.w > 70.0 && obb.h > 20.0);
        assert!(!block.vertical());
    }

    #[test]
    fn translation_reads_last_trans() {
        let block = crate::TextBlock {
            lines: vec![],
            text: "源".into(),
            font_size: 12,
            angle: 0.0,
            prob: 1.0,
            fg_color: None,
            bg_color: None,
            skip_translate: false,
            language: None,
            translations: Default::default(),
        }
        .with_translation("ENG", "hello");
        assert_eq!(block.translation(), Some("hello"));
    }
}
