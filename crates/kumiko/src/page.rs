use image::{DynamicImage, RgbImage};
use itertools::Itertools;
use opencv::{
    boxed_ref::BoxedRef,
    core::{
        no_array, Mat, MatTraitConst, Point, ToInputArray, Vec4f, Vector, BORDER_DEFAULT, CV_16S,
    },
    imgproc::{
        self, approx_poly_dp, arc_length, convex_hull, create_line_segment_detector,
        CHAIN_APPROX_SIMPLE, LSD_REFINE_NONE, RETR_EXTERNAL, THRESH_BINARY,
    },
    prelude::LineSegmentDetectorTrait,
};
use std::collections::{BTreeSet, HashMap};

use crate::{
    panel::{self, Panel},
    segment::Segment,
};

pub fn detect_panels(
    buffer: Vec<u8>,
    width: u32,
    height: u32,
    ltr: bool,
    min_panel_size_ratio: Option<f64>,
    panel_expansion: bool,
) -> Vec<Panel> {
    let small_panel_ratio = min_panel_size_ratio.unwrap_or(1.0 / 10.0);
    let gray =
        DynamicImage::from(RgbImage::from_raw(width, height, buffer.clone()).unwrap()).to_luma8();
    let gray = Mat::from_slice(gray.as_raw()).unwrap();
    let gray = gray.reshape(1, height as i32).unwrap();
    // https://docs.opencv.org/3.4/d2/d2c/tutorial_sobel_derivatives.html
    let ddepth = CV_16S;

    let grad_x = sobel(&gray, ddepth, 1, 0, 3, 1.0, 0.0, BORDER_DEFAULT).unwrap();
    // Gradient-Y
    // grad_y = cv.Scharr(self.gray,ddepth,0,1)
    let grad_y = sobel(&gray, ddepth, 0, 1, 3, 1.0, 0.0, BORDER_DEFAULT).unwrap();
    let abs_grad_x = convert_scale_abs(&grad_x).unwrap();
    let abs_grad_y = convert_scale_abs(&grad_y).unwrap();
    let sobel = add_weighted(&abs_grad_x, 0.5, &abs_grad_y, 0.5, 0.0).unwrap();
    let contours = get_contours(&sobel).unwrap();
    let segments = get_segments(&gray, width, height, small_panel_ratio).unwrap();
    let mut panels = get_initial_panels(contours, width, height, small_panel_ratio, ltr).unwrap();
    group_small_panels(&mut panels, width, height, small_panel_ratio, ltr).unwrap();
    split_panels(
        &mut panels,
        width,
        height,
        small_panel_ratio,
        ltr,
        &segments,
    );
    let mut panels = exclude_small_panels(panels, width, height, small_panel_ratio);
    merge_panels(&mut panels);
    deoverlap_panels(&mut panels);
    let mut panels = exclude_small_panels(panels, width, height, small_panel_ratio);

    if panel_expansion {
        panels.sort(); // TODO: move this below before panels sort-fix, when panels expansion is smarter
        expand_panels(&mut panels);
    }

    if panels.len() == 0 {
        panels.push(Panel::from_xywh(0, 0, width, height, true, ltr));
    }

    group_big_panels(&mut panels, &segments);

    fix_panels_numbering(&mut panels, ltr);
    panels
}

fn group_big_panels(panels: &mut Vec<Panel>, segments: &[Segment]) {
    let mut grouped = true;
    while grouped {
        grouped = false;
        let mut to_remove = Vec::new();
        let mut to_extend = Vec::new();
        for (i, p1) in panels.iter().enumerate() {
            for j in i + 1..panels.len() {
                let p2 = &panels[j];
                let p3 = p1.group_with(&p2);

                let other_panels = panels
                    .iter()
                    .filter(|&v| v != p1 && v != p2)
                    .collect::<Vec<_>>();
                if p3.bumps_into(&other_panels) {
                    continue;
                }
                let mut seg = vec![];
                for s in segments {
                    if p3.contains_segment(s) && s.dist() > p3.diagonal().dist() / 5.0 {
                        if !seg.contains(s) {
                            seg.push(*s);
                        }
                    }
                }

                if seg.len() > 0 {
                    //maybe allow a small number of big segments here?
                    continue;
                }
                to_extend.push(p3);
                to_remove.push(i);
                to_remove.push(j);
                grouped = true;
                break;
            }
            if grouped {
                break;
            }
        }

        to_remove.sort_by(|a, &b| b.cmp(a));
        for item in to_remove {
            panels.remove(item);
        }
        panels.extend(to_extend);
    }
}

fn fix_panels_numbering(panels: &mut Vec<Panel>, ltr: bool) {
    let mut changes = 1;
    while changes != 0 {
        changes = 0;
        let mut ch = vec![];
        for (i, p) in panels.iter().enumerate() {
            let mut neighbours_before = vec![p.find_top_panel(panels)];
            if ltr {
                neighbours_before.extend(p.find_all_left_panels(panels).into_iter().map(Some));
            } else {
                neighbours_before.extend(p.find_all_right_panels(panels).into_iter().map(Some));
            }
            for neighbour in neighbours_before.into_iter().flatten() {
                let neighbour_pos = panels.iter().position(|v| v == neighbour);
                if let Some(neighbour_pos) = neighbour_pos {
                    if i < neighbour_pos {
                        changes += 1;
                        ch.push((neighbour_pos, i));
                        break;
                    }
                }
            }
            if changes > 0 {
                break; //start a new whole loop with reordered panels
            }
        }
        for ch in ch {
            let temp = panels.remove(ch.1);
            panels.insert(ch.0, temp);
        }
    }
}

fn deoverlap_panels(panels: &mut Vec<Panel>) {
    for i in 0..panels.len() {
        for j in 0..panels.len() {
            let (p1, p2) = (&panels[i], &panels[j]);
            if p1 == p2 {
                continue;
            }
            let opanel = match p1.overlap_panel(p2) {
                Some(panel) => panel,
                None => continue,
            };
            if opanel.w() < opanel.h() && p1.r == opanel.r {
                panels[i].r = opanel.x;
                panels[j].x = opanel.r;
                continue;
            }
            if opanel.w() > opanel.h() && p1.b == opanel.b {
                panels[i].b = opanel.y;
                panels[j].y = opanel.b;
                continue;
            }
        }
    }
}

fn exclude_small_panels(
    panels: Vec<Panel>,
    width: u32,
    height: u32,
    small_panel_ratio: f64,
) -> Vec<Panel> {
    panels
        .into_iter()
        .filter(|v| v.is_small(1.0, width, height, small_panel_ratio))
        .collect::<Vec<_>>()
}

fn expand_panels(panels: &mut Vec<Panel>) {
    let gutters = actual_gutters(panels, |gutters| *gutters.iter().min().unwrap());
    let mut i: usize = 0;
    let get_next = |panels: &Vec<Panel>, i| panels.get(i).is_some();
    while get_next(panels, i) {
        for d in [D::X, D::Y, D::R, D::B] {
            let p = &panels[i];
            let newcoord;
            let neighbour = p.find_neighbour_panel(d, panels);

            if let Some(neighbour) = neighbour {
                newcoord = neighbour.get(match d {
                    D::X => D::R,
                    D::Y => D::B,
                    D::R => D::X,
                    D::B => D::Y,
                }) + gutters.get(d);
            } else {
                //expand to the furthest known edge (frame around all panels)
                let min_panel = match d {
                    D::X | D::Y => panels.iter().min_by_key(|p| p.get(d)),
                    D::R | D::B => panels.iter().max_by_key(|p| p.get(d)),
                }
                .unwrap();
                newcoord = min_panel.get(d);
            }
            if matches!(d, D::R | D::B) && newcoord > p.get(d)
                || matches!(d, D::X | D::Y) && newcoord < p.get(d)
            {
                panels[i].set(d, newcoord);
            }
        }
        i += 1;
    }
}

#[derive(Clone, Copy)]
pub enum D {
    X,
    Y,
    R,
    B,
}

fn actual_gutters(panels: &[Panel], func: fn(&[i32]) -> i32) -> Box {
    let mut gutters_x = vec![];
    let mut gutters_y = vec![];
    for p in panels {
        let left_panel = p.find_left_panel(panels);
        if let Some(left_panel) = left_panel {
            gutters_x.push(p.x - left_panel.r);
        }
        let top_panel = p.find_top_panel(panels);
        if let Some(top_panel) = top_panel {
            gutters_y.push(p.y - top_panel.b);
        }
    }
    if gutters_x.is_empty() {
        gutters_x.push(1);
    }
    if gutters_y.is_empty() {
        gutters_y.push(1);
    }
    Box {
        x: func(&gutters_x),
        y: func(&gutters_y),
        r: -func(&gutters_x),
        b: -func(&gutters_y),
    }
}

pub struct Box {
    pub x: i32,
    pub y: i32,
    pub r: i32,
    pub b: i32,
}

impl Box {
    pub fn get(&self, d: D) -> i32 {
        match d {
            D::X => self.x,
            D::Y => self.y,
            D::R => self.r,
            D::B => self.b,
        }
    }
}

fn split_panels(
    panels: &mut Vec<Panel>,
    width: u32,
    height: u32,
    small_panel_ratio: f64,
    ltr: bool,
    segments: &[Segment],
) {
    let mut did_split = true;
    while did_split {
        did_split = false;
        let mut split_ = None;
        let mut temp = panels.iter_mut().collect::<Vec<_>>();
        temp.sort_by_key(|v| v.area());
        for (i, p) in temp.into_iter().rev().enumerate() {
            let split = p.split(width, height, small_panel_ratio, ltr, segments);
            if let Some(split) = split {
                did_split = true;
                split_ = Some((split, i));
                break;
            }
        }
        if let Some((split, i)) = split_ {
            panels.remove(i);
            panels.extend(split.subpanels);
        }
    }
}

fn group_small_panels(
    panels: &mut Vec<Panel>,
    width: u32,
    height: u32,
    small_panel_ratio: f64,
    ltr: bool,
) -> Result<(), opencv::Error> {
    let small_panels = panels
        .iter()
        .filter(|v| v.is_small(1.0, width, height, small_panel_ratio));
    let mut groups: HashMap<&Panel, i32> = HashMap::new();

    let mut group_id = 0;
    for (p1, p2) in small_panels.tuple_combinations() {
        if p1 == p2 {
            continue;
        }
        if !p1.is_close(p2) {
            continue;
        }
        match (groups.get(&p1).copied(), groups.get(&p2).copied()) {
            (None, None) => {
                group_id += 1;
                groups.insert(p1, group_id);
                groups.insert(p2, group_id);
            }
            (None, Some(id)) => {
                groups.insert(p1, id);
            }
            (Some(id), None) => {
                groups.insert(p2, id);
            }
            (Some(id1), Some(id2)) => {
                if id1 != id2 {
                    for (_, id) in groups.iter_mut() {
                        if *id == id2 {
                            *id = id1;
                        }
                    }
                }
            }
        }
    }
    let mut grouped: HashMap<i32, Vec<&Panel>> = HashMap::new();
    for (k, v) in groups.into_iter() {
        grouped.entry(v).or_default().push(k);
    }
    for small_panels in grouped
        .into_iter()
        .map(|v| v.1)
        .map(|v| v.into_iter().map(|v| v.clone()).collect::<Vec<_>>())
        .collect::<Vec<_>>()
    {
        let points = small_panels
            .iter()
            .filter_map(|v| v.polygon.as_ref())
            .flat_map(|v| v.iter().map(|v| Point::new(v.x, v.y)))
            .collect::<Vector<Point>>();
        let mut big_hull = Vector::<Point>::new();
        convex_hull(&points, &mut big_hull, false, true)?;
        let big_hull = big_hull
            .iter()
            .map(|v| panel::Point::new(v.x, v.y))
            .collect();
        let big_panel = Panel::new(big_hull, false, ltr);
        panels.push(big_panel);
        //TODO: optimize
        for p in small_panels {
            if let Some(i) = panels.iter().position(|v| v == &p) {
                panels.remove(i);
            }
        }
    }
    Ok(())
}

fn merge_panels(panels: &mut Vec<Panel>) {
    let mut panels_to_remove = vec![];
    for i in 0..panels.len() {
        for j in (i + 1)..panels.len() {
            let p1 = &panels[i];
            let p2 = &panels[j];
            if p1.contains(p2) {
                panels_to_remove.push(j);
                let p1_temp = p1.merge(p2, panels);
                panels[i] = p1_temp;
            } else if p2.contains(p1) {
                panels_to_remove.push(i);
                let p2_temp = p2.merge(p1, panels);
                panels[j] = p2_temp;
            }
        }
    }
    for p in panels_to_remove
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .rev()
    {
        panels.remove(p);
    }
}

fn get_segments(
    gray: &BoxedRef<'_, Mat>,
    width: u32,
    height: u32,
    small_panel_ratio: f64,
) -> Result<Vec<Segment>, opencv::Error> {
    let mut segments = None;
    let mut lsd =
        create_line_segment_detector(LSD_REFINE_NONE, 0.8, 0.6, 2.0, 22.5, 0.0, 0.7, 1024)?;
    let mut dlines = Vector::<Vec4f>::new();
    lsd.detect(
        gray,
        &mut dlines,
        &mut no_array(),
        &mut no_array(),
        &mut no_array(),
    )?;
    let mut min_dist = width.min(height) as f64 * small_panel_ratio;
    let helper = |seg: &Option<Vec<_>>| match seg {
        Some(seg) => seg.len() > 500,
        None => true,
    };
    while helper(&segments) {
        segments = Some(vec![]);

        if dlines.is_empty() {
            break;
        }
        for dline in &dlines {
            let [x0, y0, x1, y1] = dline.0;
            let (x0, y0, x1, y1) = (x0 as i32, y0 as i32, x1 as i32, y1 as i32);
            let a = x0 - x1;
            let b = y0 - y1;
            let dist = ((a.pow(2) + b.pow(2)) as f64).sqrt();
            if dist >= min_dist {
                if let Some(s) = &mut segments {
                    s.push(Segment::new([x0, y0], [x1, y1]));
                }
            }
        }

        min_dist *= 1.1;
    }
    Ok(Segment::union_all(segments.unwrap_or_default()))
}
fn get_contours(sobel: &Mat) -> Result<Vec<Vector<Point>>, opencv::Error> {
    let (_, thresh) = threshold(sobel, 100.0, 255.0, THRESH_BINARY)?;
    let contours = find_contours(&thresh, RETR_EXTERNAL, CHAIN_APPROX_SIMPLE)?;
    let mut arr = contours.iter().collect::<Vec<_>>();
    let mut out = vec![];
    for _ in 0..2 {
        if let Some(contour) = arr.pop() {
            out.push(contour);
        }
    }
    Ok(out)
}

fn find_contours(
    thresh: &Mat,
    mode: i32,
    method: i32,
) -> Result<Vector<Vector<Point>>, opencv::Error> {
    let mut contours = Vector::<Vector<Point>>::new();
    imgproc::find_contours(thresh, &mut contours, mode, method, Point::default())?;
    Ok(contours)
}

fn threshold(
    src: &impl ToInputArray,
    thresh: f64,
    maxval: f64,
    typ: i32,
) -> Result<(f64, Mat), opencv::Error> {
    let mut dst = Mat::default();
    let t = imgproc::threshold(src, &mut dst, thresh, maxval, typ)?;
    Ok((t, dst))
}

fn get_initial_panels(
    contours: Vec<Vector<Point>>,
    width: u32,
    height: u32,
    small_panel_ratio: f64,
    l2r: bool,
) -> Result<Vec<Panel>, opencv::Error> {
    let mut panels = vec![];
    for contour in contours {
        let arclength = arc_length(&contour, true)?;
        let epsilon = 0.001 * arclength;
        let mut approx = Vector::<Point>::new();
        approx_poly_dp(&contour, &mut approx, epsilon, true)?;
        let panel = Panel::new(
            approx.iter().map(|v| panel::Point::new(v.x, v.y)).collect(),
            true,
            l2r,
        );
        if panel.is_very_small(width, height, small_panel_ratio) {
            continue;
        }
        panels.push(panel);
    }
    Ok(panels)
}

fn add_weighted(
    src1: &impl ToInputArray,
    alpha: f64,
    src2: &impl ToInputArray,
    beta: f64,
    gamma: f64,
) -> Result<Mat, opencv::Error> {
    let mut dst = Mat::default();
    opencv::core::add_weighted(src1, alpha, src2, beta, gamma, &mut dst, -1)?;
    Ok(dst)
}
fn convert_scale_abs(src: &Mat) -> Result<Mat, opencv::Error> {
    let mut dst = Mat::default();
    opencv::core::convert_scale_abs(src, &mut dst, 1.0, 0.0)?;
    Ok(dst)
}

fn sobel(
    src: &BoxedRef<'_, Mat>,
    depth: i32,
    dx: i32,
    dy: i32,
    ksize: i32,
    scale: f64,
    delta: f64,
    border_type: i32,
) -> Result<Mat, opencv::Error> {
    let mut dst = Mat::default();
    imgproc::sobel(
        &src,
        &mut dst,
        depth,
        dx,
        dy,
        ksize,
        scale,
        delta,
        border_type,
    )?;
    Ok(dst)
}
