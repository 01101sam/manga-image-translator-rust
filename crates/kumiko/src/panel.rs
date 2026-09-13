use std::{
    hash::Hash,
    mem,
    ops::{Add, Mul, Sub},
};

use crate::{page::D, segment::Segment};

#[derive(Clone)]
pub struct Panel {
    pub polygon: Option<Vec<Point>>,
    pub x: i32,
    pub y: i32,
    pub r: i32,
    pub b: i32,
    splittable: bool,
    segments: Option<Vec<Segment>>,
    coverage: Option<()>,
    ltr: bool,
}

impl Panel {
    pub fn get(&self, d: D) -> i32 {
        match d {
            D::X => self.x,
            D::Y => self.y,
            D::R => self.r,
            D::B => self.b,
        }
    }
    pub fn set(&mut self, d: D, value: i32) {
        match d {
            D::X => self.x = value,
            D::Y => self.y = value,
            D::R => self.r = value,
            D::B => self.b = value,
        }
    }
}

impl Hash for Panel {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.x.hash(state);
        self.y.hash(state);
        self.r.hash(state);
        self.b.hash(state);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Point<T = i32> {
    pub x: T,
    pub y: T,
}

impl Point<i32> {
    pub fn to_f64(&self) -> Point<f64> {
        Point {
            x: self.x as f64,
            y: self.y as f64,
        }
    }
}

impl Point<f64> {
    pub fn round_ties_even(&self) -> Point<i32> {
        Point {
            x: self.x.round_ties_even() as i32,
            y: self.y.round_ties_even() as i32,
        }
    }
}

impl<T: Copy + Mul<Output = T> + Add<Output = T>> Point<T> {
    pub fn dot(&self, other: &Point<T>) -> T {
        self.x * other.x + self.y * other.y
    }
}

impl From<Point> for [i32; 2] {
    fn from(point: Point) -> Self {
        [point.x, point.y]
    }
}

impl<T> Point<T> {
    pub fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

impl Point {
    pub fn zero() -> Self {
        Self { x: 0, y: 0 }
    }
}

fn bounding_rect(points: &[Point]) -> Option<[i32; 4]> {
    if points.is_empty() {
        return None;
    }

    let mut min_x = points[0].x;
    let mut min_y = points[0].y;
    let mut max_x = points[0].x;
    let mut max_y = points[0].y;

    for p in points.iter().skip(1) {
        if p.x < min_x {
            min_x = p.x;
        }
        if p.y < min_y {
            min_y = p.y;
        }
        if p.x > max_x {
            max_x = p.x;
        }
        if p.y > max_y {
            max_y = p.y;
        }
    }

    Some([min_x, min_y, max_x, max_y])
}

impl Panel {
    pub fn area(&self) -> i32 {
        self.w() * self.h()
    }

    pub fn from_xyrb([x, y, r, b]: [u32; 4], splittable: bool, ltr: bool) -> Self {
        Self {
            polygon: None,
            x: x as i32,
            y: y as i32,
            r: r as i32,
            b: b as i32,
            splittable,
            segments: None,
            coverage: None,
            ltr,
        }
    }

    pub fn from_xywh(x: u32, y: u32, w: u32, h: u32, splittable: bool, ltr: bool) -> Self {
        Self {
            polygon: None,
            x: x as i32,
            y: y as i32,
            r: x as i32 + w as i32,
            b: y as i32 + h as i32,
            splittable,
            segments: None,
            coverage: None,
            ltr,
        }
    }

    pub fn new(polygon: Vec<Point>, splittable: bool, ltr: bool) -> Self {
        let [x, y, r, b] = bounding_rect(&polygon).unwrap();
        Self {
            polygon: Some(polygon),
            x,
            y,
            r,
            b,
            splittable,
            segments: None,
            coverage: None,
            ltr,
        }
    }

    pub fn w(&self) -> i32 {
        self.r - self.x
    }
    pub fn h(&self) -> i32 {
        self.b - self.y
    }
    fn wt(&self) -> f64 {
        //wt = width threshold (under which two edge coordinates are considered equal)
        (self.w() as f64) / 10.0
    }
    fn ht(&self) -> f64 {
        // ht = height threshold
        (self.h() as f64) / 10.0
    }

    pub fn is_close(&self, other: &Self) -> bool {
        let c1x = self.x as f64 + self.w() as f64 / 2.0;
        let c1y = self.y as f64 + self.h() as f64 / 2.0;
        let c2x = other.x as f64 + other.w() as f64 / 2.0;
        let c2y = other.y as f64 + other.h() as f64 / 2.0;

        (c1x - c2x).abs() <= (self.w() + other.w()) as f64 * 0.75
            && (c1y - c2y).abs() <= (self.h() + other.h()) as f64 * 0.75
    }

    pub fn contains(&self, other: &Self) -> bool {
        let o_panel = self.overlap_panel(other);
        match o_panel {
            Some(panel) => panel.area() as f64 / other.area() as f64 > 0.50,
            None => false,
        }
    }

    pub fn is_small(
        &self,
        extra_ratio: f64,
        width: u32,
        height: u32,
        small_panel_ratio: f64,
    ) -> bool {
        (self.w() as f64) < width as f64 * small_panel_ratio * extra_ratio
            || (self.h() as f64) < height as f64 * small_panel_ratio * extra_ratio
    }

    pub fn is_very_small(&self, width: u32, height: u32, small_panel_ratio: f64) -> bool {
        self.is_small(1.0 / 10.0, width, height, small_panel_ratio)
    }

    pub fn split(
        &mut self,
        width: u32,
        height: u32,
        small_panel_ratio: f64,
        ltr: bool,
        segments: &[Segment],
    ) -> Option<PanelSplit> {
        if !self.splittable {
            return None;
        }
        let split = self.cached_split(width, height, small_panel_ratio, ltr, segments);
        if split.is_none() {
            self.splittable = false;
        }

        split
    }
    fn cached_split(
        &self,
        width: u32,
        height: u32,
        small_panel_ratio: f64,
        ltr: bool,
        segments: &[Segment],
    ) -> Option<PanelSplit> {
        let mut original_polygon = self.polygon.as_ref()?.clone();
        let mut polygon = vec![];
        // panel should be splittable in two non-small subpanels

        if self.is_small(2.0, width, height, small_panel_ratio) {
            return None;
        }

        let min_hops = 3;
        let max_dist_x = self.w() / 3;
        let max_dist_y = self.h() / 3;
        let max_diagonal = (max_dist_x.pow(2) as f64 + max_dist_y.pow(2) as f64).sqrt();
        let dots_along_lines_dist = max_diagonal / 5.0;
        let min_dist_between_dots_x = max_dist_x as f64 / 10.0;
        let min_dist_between_dots_y = max_dist_y as f64 / 10.0;
        let mut extra_dots = vec![];
        let mut intermediary_dots = vec![];

        for i in 0..original_polygon.len() {
            let j = (i + 1) % original_polygon.len();
            let dot1 = original_polygon[i];
            let dot2 = original_polygon[j];
            let seg = Segment::new(dot1.into(), dot2.into());

            if (seg.dist_x(false) as f64) < min_dist_between_dots_x
                && (seg.dist_y(false) as f64) < min_dist_between_dots_y
            {
                original_polygon[j] = seg.center();
                continue;
            }

            polygon.push(dot1);

            // Add dots on *long* edges, by projecting other polygon dots on this segment
            let mut add_dots = vec![];

            // should be splittable in [dot1, dot1b(?), projected_dot3, dot2b(?), dot2]
            if seg.dist() < dots_along_lines_dist * 2.0 {
                continue;
            }

            for (k, dot3) in original_polygon.iter().enumerate() {
                if (k as isize - i as isize).abs() < min_hops {
                    continue;
                }

                let projected_dot3 = seg.projected_point(dot3);
                // Segment should be able to contain projected_dot3

                if !seg.may_contain(&projected_dot3) {
                    continue;
                }

                // dot3 should be close to current segment − distance(dot3, projected_dot3) should be short
                let project = Segment::new((*dot3).into(), projected_dot3.into());
                if project.dist_x(false) as f64 > max_dist_x as f64
                    || project.dist_y(false) as f64 > max_dist_y as f64
                {
                    continue;
                }
                //append dot3 as intermediary dot on segment(dot1, dot2)
                add_dots.push(projected_dot3);
                intermediary_dots.push(projected_dot3);
            }

            // Add also a dot near each end of the segment (provoke segment matching)
            let alpha_x = f64::acos(seg.dist_x(true) as f64 / seg.dist());
            let alpha_y = f64::asin(seg.dist_y(true) as f64 / seg.dist());
            let dist_x = (f64::cos(alpha_x) * dots_along_lines_dist) as i32;
            let dist_y = (f64::sin(alpha_y) * dots_along_lines_dist) as i32;
            let dist = Point::new(dist_x, dist_y);
            let dot1b = dot1 + dist;

            //if len(intermediary_dots) == 0 or Segment(dot1b, intermediary_dots[0]).dist() > dots_along_lines_dist:
            add_dots.push(dot1b);
            extra_dots.push(dot1b);

            let dot2b = dot2 - dist;

            //if len(intermediary_dots) == 0 or Segment(dot2b, intermediary_dots[-1]).dist() > dots_along_lines_dist:
            add_dots.push(dot2b);
            extra_dots.push(dot2b);
            let mut add_dots = add_dots.iter().collect::<Vec<_>>();
            add_dots.sort_by_key(|dot| Segment::new(dot1.into(), (**dot).into()).dist() as i32);
            for dot in add_dots {
                polygon.push(*dot);
            }
        }

        // Re-merge nearby dots together
        let mut original_polygon = vec![];
        mem::swap(&mut original_polygon, &mut polygon);
        for i in 0..original_polygon.len() {
            let j = (i + 1) % original_polygon.len();
            let dot1 = original_polygon[i];
            let dot2 = original_polygon[j];
            let seg = Segment::new(dot1.into(), dot2.into());
            // merge nearby dots together
            if (seg.dist_x(false) as f64) < min_dist_between_dots_x
                && (seg.dist_y(false) as f64) < min_dist_between_dots_y
            {
                intermediary_dots = intermediary_dots
                    .into_iter()
                    .filter(|dot| ![dot1, dot2].contains(dot))
                    .collect();
                extra_dots = extra_dots
                    .into_iter()
                    .filter(|dot| ![dot1, dot2].contains(dot))
                    .collect();
                original_polygon[j] = seg.center();
                continue;
            }

            polygon.push(dot1);
        }

        // Find dots nearby one another
        let mut nearby_dots = vec![];
        for i in 0..(polygon.len() - min_hops as usize) {
            for j in (i + min_hops as usize)..polygon.len() {
                let dot1 = polygon[i];
                let dot2 = polygon[j];
                let seg = Segment::new(dot1.into(), dot2.into());
                if (seg.dist_x(false) as f64) <= max_dist_x as f64
                    && (seg.dist_y(false) as f64) <= max_dist_y as f64
                {
                    nearby_dots.push(Point::new(i as i32, j as i32));
                }
            }
        }

        if nearby_dots.is_empty() {
            return None;
        }

        let mut splits = vec![];
        for dots in nearby_dots {
            let poly1len = polygon.len() as i32 - dots.y + dots.x;
            let poly2len = dots.y - dots.x;
            if poly1len.min(poly2len) <= 2 {
                continue;
            }
            let mut poly1: Vec<Point> = Vec::with_capacity(poly1len as usize);
            let mut poly2: Vec<Point> = Vec::with_capacity(poly2len as usize);
            for (i, point) in polygon.iter().enumerate() {
                if i <= dots.x as usize || i > dots.y as usize {
                    poly1.push(*point);
                } else {
                    poly2.push(*point);
                }
            }
            let panel1 = Panel::new(poly1, true, ltr);
            let panel2 = Panel::new(poly2, true, ltr);
            if panel1.is_small(1.0, width, height, small_panel_ratio)
                || panel2.is_small(1.0, width, height, small_panel_ratio)
            {
                continue;
            }

            if &panel1 == self || &panel2 == self {
                continue;
            }

            if panel1.overlaps(&panel2) {
                continue;
            }
            let split_segment = Segment::along_polygon(&polygon, dots.x as usize, dots.y as usize);

            let split = PanelSplit::new(self.clone(), panel1, panel2, split_segment, segments);
            if !splits.contains(&split) {
                splits.push(split);
            }
        }
        let splits = splits
            .into_iter()
            .filter(|split| split.segments_coverage() > 50.0 / 100.0);

        //return the split that best matches segments (~panel edges)
        splits.max_by_key(|split| ordered_float::OrderedFloat(split.covered_dist))
    }

    pub fn contains_segment(&self, segment: &Segment) -> bool {
        let other = Panel::from_xyrb(segment.to_xyrb(), true, self.ltr);
        self.overlaps(&other)
    }

    pub fn overlap_panel(&self, other: &Self) -> Option<Panel> {
        if self.x > other.r || other.x > self.r {
            // panels are left and right from one another
            return None;
        }
        if self.y > other.b || other.y > self.b {
            // panels are above and below one another
            return None;
        }
        // if we're here, panels overlap at least a bit
        let x = i32::max(self.x, other.x);
        let y = i32::max(self.y, other.y);
        let r = i32::min(self.r, other.r);
        let b = i32::min(self.b, other.b);
        Some(Panel::from_xyrb(
            [x as u32, y as u32, r as u32, b as u32],
            true,
            self.ltr,
        ))
    }

    fn overlaps(&self, other: &Self) -> bool {
        let opanel = self.overlap_panel(other);
        let opanel = match opanel {
            Some(s) => s,
            None => return false,
        };
        let area_ratio = 0.1;
        let smallest_panel_area = i32::min(self.area(), other.area());
        if smallest_panel_area == 0 {
            // probably a horizontal or vertical segment
            return true;
        }
        opanel.area() as f64 / smallest_panel_area as f64 > area_ratio
    }

    fn get_segments(&mut self, all_segments: &[Segment]) -> &Vec<Segment> {
        if self.segments.is_some() {
            return &self.segments.as_ref().unwrap();
        } else {
            self.segments = Some(
                all_segments
                    .iter()
                    .filter(|s| self.contains_segment(s))
                    .cloned()
                    .collect(),
            );

            self.get_segments(all_segments)
        }
    }

    fn same_col(&self, other: &Panel) -> bool {
        let (left, right) = if self.x < other.x {
            (self, other)
        } else {
            (other, self)
        };

        if right.x > left.r {
            // stricly left
            return false;
        }

        if right.r < left.r {
            //contained
            return true;
        }

        // intersect
        let intersection_x = left.r.min(right.r) - right.x;
        let min_w = left.w().min(right.w());
        min_w == 0 || intersection_x as f64 / min_w as f64 >= 1.0 / 3.0
    }

    fn same_row(&self, other: &Panel) -> bool {
        let (above, below) = if self.y < other.y {
            (self, other)
        } else {
            (other, self)
        };

        if below.y > above.b {
            // stricly above
            return false;
        }

        if below.b < above.b {
            // contained
            return true;
        }
        // intersect
        let intersection_y = above.b.min(below.b) - below.y;
        let min_h = above.h().min(below.h());
        min_h == 0 || intersection_y as f64 / min_h as f64 >= 1.0 / 3.0
    }

    pub fn find_neighbour_panel<'a>(&self, d: D, panels: &'a [Panel]) -> Option<&'a Panel> {
        match d {
            D::X => self.find_left_panel(panels),
            D::Y => self.find_top_panel(panels),
            D::R => self.find_right_panel(panels),
            D::B => self.find_bottom_panel(panels),
        }
    }

    pub fn find_top_panel<'a>(&self, panels: &'a [Panel]) -> Option<&'a Panel> {
        // 共用水平边界的分镜也属于上下相邻。
        let all_top = panels.iter().filter(|p| p.b <= self.y && p.same_col(self));
        all_top.max_by_key(|p| p.b)
    }

    pub fn find_all_left_panels<'a>(&self, panels: &'a [Panel]) -> Vec<&'a Panel> {
        panels
            .iter()
            .filter(|p| p.r <= self.x && p.same_row(self))
            .collect()
    }

    fn find_bottom_panel<'a>(&self, panels: &'a [Panel]) -> Option<&'a Panel> {
        let all_bottom = panels.iter().filter(|p| p.y >= self.b && p.same_col(self));
        all_bottom.min_by_key(|p| p.y)
    }

    pub fn find_left_panel<'a>(&self, panels: &'a [Panel]) -> Option<&'a Panel> {
        let all_left = self.find_all_left_panels(panels).into_iter();
        all_left.max_by_key(|p| p.r)
    }

    pub fn find_all_right_panels<'a>(&self, panels: &'a [Panel]) -> Vec<&'a Panel> {
        panels
            .iter()
            .filter(|p| p.x >= self.r && p.same_row(self))
            .collect()
    }

    pub fn diagonal(&self) -> Segment {
        Segment::new([self.x, self.y], [self.r, self.b])
    }

    pub fn bumps_into(&self, other_panels: &[&Panel]) -> bool {
        for other in other_panels {
            if *other == self {
                continue;
            }
            if self.overlaps(other) {
                return true;
            }
        }

        false
    }

    pub fn group_with(&self, other: &Panel) -> Panel {
        let min_x = i32::min(self.x, other.x);
        let min_y = i32::min(self.y, other.y);
        let max_r = i32::max(self.r, other.r);
        let max_b = i32::max(self.b, other.b);
        Panel::from_xywh(
            min_x as u32,
            min_y as u32,
            max_r as u32 - min_x as u32,
            max_b as u32 - min_y as u32,
            true,
            self.ltr,
        )
    }

    fn find_right_panel<'a>(&self, panels: &'a [Panel]) -> Option<&'a Panel> {
        let all_right = self.find_all_right_panels(panels).into_iter();
        all_right.min_by_key(|p| p.x)
    }

    pub fn merge(&self, other: &Self, panels: &[Panel]) -> Self {
        let other_panels = panels
            .iter()
            .filter(|&v| v != self && v != other)
            .collect::<Vec<_>>();
        let mut possible_panels = vec![self.clone()];

        // expand self in all four directions where other is
        if other.x < self.x {
            possible_panels.push(Panel::from_xyrb(
                [other.x as u32, self.y as u32, self.r as u32, self.b as u32],
                true,
                self.ltr,
            ));
        }
        if other.r > self.r {
            let mut to_extend = vec![];

            for pp in possible_panels.iter() {
                to_extend.push(Panel::from_xyrb(
                    [pp.x as u32, pp.y as u32, other.r as u32, pp.b as u32],
                    true,
                    self.ltr,
                ))
            }
            possible_panels.extend(to_extend);
        }
        if other.y < self.y {
            let mut to_extend = vec![];

            for pp in possible_panels.iter() {
                to_extend.push(Panel::from_xyrb(
                    [pp.x as u32, other.y as u32, pp.r as u32, pp.b as u32],
                    true,
                    self.ltr,
                ))
            }
            possible_panels.extend(to_extend);
        }
        if other.b > self.b {
            let mut to_extend = vec![];

            for pp in possible_panels.iter() {
                to_extend.push(Panel::from_xyrb(
                    [pp.x as u32, pp.y as u32, pp.r as u32, other.b as u32],
                    true,
                    self.ltr,
                ))
            }
            possible_panels.extend(to_extend);
        }

        // don't take a merged panel that bumps into other panels on page
        // take the largest merged panel
        possible_panels
            .into_iter()
            .filter(|p| !p.bumps_into(&other_panels))
            .max_by_key(|p| p.area())
            .unwrap_or(self.clone())
    }
}

impl Sub<Point> for Point {
    type Output = Point;

    fn sub(self, rhs: Point) -> Self::Output {
        Point::new(self.x - rhs.x, self.y - rhs.y)
    }
}
impl PartialEq for Panel {
    fn eq(&self, other: &Self) -> bool {
        ((self.x - other.x).abs() as f64) < self.wt()
            && ((self.y - other.y).abs() as f64) < self.ht()
            && ((self.r - other.r).abs() as f64) < self.wt()
            && ((self.b - other.b).abs() as f64) < self.ht()
    }
}

impl Eq for Panel {}

impl PartialOrd for Panel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Panel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        // panel is above other
        if other.y as f64 >= self.b as f64 - self.ht()
            && other.y as f64 >= self.y as f64 - self.ht()
        {
            return Ordering::Less;
        }

        // panel is below other
        if self.y as f64 >= other.b as f64 - self.ht()
            && self.y as f64 >= other.y as f64 - self.ht()
        {
            return Ordering::Greater;
        }

        // panel is left from other
        if other.x as f64 >= self.r as f64 - self.wt()
            && other.x as f64 >= self.x as f64 - self.wt()
        {
            return if self.ltr {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }

        // panel is right from other
        if self.x as f64 >= other.r as f64 - self.wt()
            && self.x as f64 >= other.x as f64 - self.wt()
        {
            return if self.ltr {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }

        unreachable!("should not happen");
    }
}

pub struct PanelSplit {
    panel: Panel,
    pub subpanels: [Panel; 2],
    segment: Segment,
    covered_dist: f64,
    matching_segments: Vec<Segment>,
}

impl PanelSplit {
    pub fn new(
        mut panel: Panel,
        subpanel1: Panel,
        subpanel2: Panel,
        split_segment: Segment,
        all_segments: &[Segment],
    ) -> Self {
        let matching_segments = split_segment.intersect_all(panel.get_segments(all_segments));
        let covered_dist = matching_segments.iter().map(|s| s.dist()).sum();

        PanelSplit {
            panel,
            subpanels: [subpanel1, subpanel2],
            segment: split_segment,
            covered_dist,
            matching_segments,
        }
    }
    pub fn segments_coverage(&self) -> f64 {
        let segment_dist = self.segment.dist();
        if segment_dist != 0.0 {
            self.covered_dist / segment_dist
        } else {
            0.0
        }
    }
}

impl PartialEq for PanelSplit {
    fn eq(&self, other: &Self) -> bool {
        self.segment == other.segment
    }
}

impl<T: Mul<Output = T> + Copy> Mul<T> for Point<T> {
    type Output = Point<T>;

    fn mul(self, rhs: T) -> Self::Output {
        Point {
            x: self.x * rhs,
            y: self.y * rhs,
        }
    }
}

impl<T: Add<Output = T> + Copy> Add<Point<T>> for Point<T> {
    type Output = Point<T>;

    fn add(self, rhs: Point<T>) -> Self::Output {
        Point::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl<T: Copy> From<[T; 2]> for Point<T> {
    fn from(value: [T; 2]) -> Self {
        Point::new(value[0], value[1])
    }
}
