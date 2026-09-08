use std::{
    f64::{self, consts::PI},
    ops::{Add, Div, Mul, Sub},
};

use geo::{Area as _, ConvexHull as _, Distance, Euclidean, MultiPoint, Point, Polygon};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quadrilateral {
    pts: [MyPoint; 4],
    score: f64,
    vertical: bool,
}

pub struct BBox {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

fn max_coords(pts: &[MyPoint; 4]) -> MyPoint {
    pts.iter()
        .fold((i64::MIN, i64::MIN), |acc, point| {
            (acc.0.max(point.x), acc.1.max(point.y))
        })
        .into()
}

fn min_coords(pts: &[MyPoint; 4]) -> MyPoint {
    pts.iter()
        .fold((i64::MAX, i64::MAX), |acc, point| {
            (acc.0.min(point.x), acc.1.min(point.y))
        })
        .into()
}

impl Quadrilateral {
    pub fn scale(mut self, factor: f64) -> Self {
        self.pts = [
            self.pts[0].scale(factor),
            self.pts[1].scale(factor),
            self.pts[2].scale(factor),
            self.pts[3].scale(factor),
        ];
        self
    }

    pub fn set_vert(&mut self, vert: bool) {
        self.vertical = vert;
    }

    pub fn aabb(&self) -> BBox {
        let max_coord = max_coords(&self.pts);
        let min_coord = min_coords(&self.pts);
        BBox {
            x: min_coord.x,
            y: min_coord.y,
            w: max_coord.x - min_coord.x,
            h: max_coord.y - min_coord.y,
        }
    }

    pub fn xyxy(&self) -> (i64, i64, i64, i64) {
        let aabb = self.aabb();
        (aabb.x, aabb.y, aabb.x + aabb.w, aabb.y + aabb.h)
    }

    pub fn clip_to(self, width: i64, height: i64) -> Option<Self> {
        let pts = self
            .pts
            .iter()
            .map(|p| (p.x.clamp(0, width), p.y.clamp(0, height)))
            .collect();
        let q = Self::new(pts, self.score);
        let aabb = q.aabb();
        (aabb.w > 0 && aabb.h > 0).then_some(q)
    }

    pub fn poly_distance(&self, other: &Self) -> f64 {
        Euclidean.distance(&self.polygon(), &other.polygon())
    }

    pub fn distance(&self, other: &Self, rho: f64) -> f64 {
        let mut pattern = if !self.vertical { "h_left" } else { "v_top" };
        let fs = self.font_size().max(other.font_size());
        if !self.vertical {
            let poly1 = MultiPoint::from(vec![
                Point::from(self.pts[0].to_tuple()),
                Point::from(self.pts[3].to_tuple()),
                Point::from(self.pts[0].to_tuple()),
                Point::from(self.pts[3].to_tuple()),
            ])
            .convex_hull();
            let poly2 = MultiPoint::from(vec![
                Point::from(self.pts[2].to_tuple()),
                Point::from(self.pts[1].to_tuple()),
                Point::from(self.pts[2].to_tuple()),
                Point::from(self.pts[1].to_tuple()),
            ])
            .convex_hull();
            let poly3 = MultiPoint::from(vec![
                Point::from(self.pts[0].to_tuple()),
                Point::from(self.pts[1].to_tuple()),
                Point::from(self.pts[0].to_tuple()),
                Point::from(self.pts[1].to_tuple()),
            ])
            .convex_hull();
            let dist1 = poly1.exterior().unsigned_area() as f64 / fs;
            let dist2 = poly2.exterior().unsigned_area() as f64 / fs;
            let dist3 = poly3.exterior().unsigned_area() as f64 / fs;
            if dist1 < fs * rho {
                pattern = "h_left";
            }
            if dist2 < fs * rho && dist2 < dist1 {
                pattern = "h_right";
            }

            if dist3 < fs * rho && dist3 < dist1 && dist3 < dist2 {
                pattern = "h_middle";
            }

            if pattern == "h_left" {
                return self.pts[0].dist(&other.pts[0]);
            } else if pattern == "h_right" {
                return self.pts[1].dist(&other.pts[1]);
            } else {
                let sestr = self.structure();
                let otstr = other.structure();
                return sestr[0].dist(&otstr[0]);
            }
        } else {
            let poly1 = MultiPoint::from(vec![
                Point::from(self.pts[0].to_tuple()),
                Point::from(self.pts[1].to_tuple()),
                Point::from(self.pts[0].to_tuple()),
                Point::from(self.pts[1].to_tuple()),
            ])
            .convex_hull();
            let poly2 = MultiPoint::from(vec![
                Point::from(self.pts[2].to_tuple()),
                Point::from(self.pts[3].to_tuple()),
                Point::from(self.pts[2].to_tuple()),
                Point::from(self.pts[3].to_tuple()),
            ])
            .convex_hull();
            let dist1 = poly1.exterior().unsigned_area() as f64 / fs;
            let dist2 = poly2.exterior().unsigned_area() as f64 / fs;
            if dist1 < fs * rho {
                pattern = "v_top";
            }
            if dist2 < fs * rho && dist2 < dist1 {
                pattern = "v_bottom"
            }
            if pattern == "v_top" {
                return self.pts[0].dist(&other.pts[0]);
            } else {
                return self.pts[2].dist(&other.pts[2]);
            }
        }
    }

    pub fn centroid(&self) -> MyPoint<f64> {
        let sum = self.pts.iter().fold((0i64, 0i64), |acc, point| {
            (acc.0 + point.x, acc.1 + point.y)
        });
        (sum.0 as f64 / 4.0, sum.1 as f64 / 4.0).into()
    }

    pub fn is_approximate_axis_aligned(&self) -> bool {
        let [l1a, l1b, l2a, l2b] = self.structure();
        let v1 = l1b - l1a;
        let v2 = l2b - l2a;
        let e1 = MyPoint::from((0.0, 1.0));
        let e2 = MyPoint::from((1.0, 0.0));
        let norm1 = v1.euclidean_norm();
        let unit_v1 = v1.to_f64() / norm1;
        let norm2 = v2.euclidean_norm();
        let unit_v2 = v1.to_f64() / norm2;
        if unit_v1.dot(e1).abs() < 0.05
            || unit_v1.dot(e2).abs() < 0.05
            || unit_v2.dot(e1).abs() < 0.05
            || unit_v2.dot(e2).abs() < 0.05
        {
            return true;
        }
        false
    }

    pub fn pts(&self) -> &[MyPoint; 4] {
        &self.pts
    }
    pub fn score(&self) -> f64 {
        self.score
    }

    pub fn vertical(&self) -> bool {
        self.vertical
    }

    pub fn new(points: Vec<(i64, i64)>, score: f64) -> Self {
        let pts: [(i64, i64); 4] = points.try_into().expect("Vec must have exactly 4 elements");

        let (pts, vertical) = sort_pnts(pts);
        Self {
            pts,
            score,
            vertical,
        }
    }
    pub fn new2(points: Vec<MyPoint>, score: f64) -> Self {
        //TODO: restore direction & no order
        Self::new(
            points.into_iter().map(|v| v.to_tuple()).collect::<Vec<_>>(),
            score,
        )
    }

    pub fn polygon(&self) -> Polygon {
        let points: Vec<Point> = self.pts.iter().map(|&p| p.to_f64().to_point()).collect();
        MultiPoint::from(points).convex_hull()
    }

    pub fn area(&self) -> f64 {
        self.polygon().unsigned_area()
    }

    pub fn structure(&self) -> [MyPoint; 4] {
        let p1 = self.pts[0].midpoint(self.pts[1], 2);
        let p2 = self.pts[2].midpoint(self.pts[3], 2);
        let p3 = self.pts[1].midpoint(self.pts[2], 2);
        let p4 = self.pts[3].midpoint(self.pts[0], 2);

        [p1, p2, p3, p4]
    }

    pub fn aspect_ratio(&self) -> f64 {
        let [l1a, l1b, l2a, l2b] = self.structure();
        let v1 = l1b.to_f64() - l1a.to_f64();
        let v2 = l2b.to_f64() - l2a.to_f64(); // horizontal

        let vertical_len = v1.euclidean_norm();
        let horizontal_len = v2.euclidean_norm();

        horizontal_len / vertical_len
    }

    pub fn font_size(&self) -> f64 {
        let [l1a, l1b, l2a, l2b] = self.structure();
        let v1 = l1b - l1a;
        let v2 = l2b - l2a;
        v1.euclidean_norm().min(v2.euclidean_norm())
    }

    fn cosangle(&self) -> f64 {
        let [l1a, l1b, _, _] = self.structure();
        let v1 = l1b - l1a;
        let norm = v1.euclidean_norm();
        if norm == 0.0 {
            return 1.0;
        }
        let unit_v1 = v1.to_f64() / norm;
        let e2 = MyPoint::from((1.0, 0.0));
        unit_v1.dot(e2)
    }

    pub fn angle(&self) -> f64 {
        (self.cosangle().acos() + PI) % PI
    }
}

/// Direction must be provided for sorting.
/// The longer structure vector (mean of long side vectors) of input points is used to determine the direction.
/// It is reliable enough for text lines but not for blocks.
fn sort_pnts(pts: [(i64, i64); 4]) -> ([MyPoint; 4], bool) {
    let mut pairwise_vec = [(0i64, 0i64); 16];
    let mut idx = 0;
    for i in 0..4 {
        for j in 0..4 {
            pairwise_vec[idx] = (pts[i].0 - pts[j].0, pts[i].1 - pts[j].1);
            idx += 1;
        }
    }

    let mut pairwise_vec_norm: [f64; 16] = [0.0; 16];
    for i in 0..16 {
        let (x, y) = pairwise_vec[i];
        pairwise_vec_norm[i] = ((x * x + y * y) as f64).sqrt();
    }

    let mut indices: [usize; 16] = {
        let mut tmp = [0; 16];
        for i in 0..16 {
            tmp[i] = i;
        }
        tmp
    };
    indices.sort_by(|&i, &j| {
        pairwise_vec_norm[i]
            .partial_cmp(&pairwise_vec_norm[j])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let long_side_ids = [indices[8], indices[10]];
    let mut long_side_vecs = [
        pairwise_vec[long_side_ids[0]],
        pairwise_vec[long_side_ids[1]],
    ];

    let inner_prod =
        long_side_vecs[0].0 * long_side_vecs[1].0 + long_side_vecs[0].1 * long_side_vecs[1].1;
    if inner_prod < 0 {
        long_side_vecs[0] = (-long_side_vecs[0].0, -long_side_vecs[0].1);
    }

    let sum_x = long_side_vecs[0].0 + long_side_vecs[1].0;
    let sum_y = long_side_vecs[0].1 + long_side_vecs[1].1;

    let mean_x = (sum_x as f64 / 2.0).abs();
    let mean_y = (sum_y as f64 / 2.0).abs();

    let is_vertical = mean_x <= mean_y;

    let mut pts_sorted = pts;

    if is_vertical {
        pts_sorted.sort_by_key(|&(_, y)| y);

        let mut top = [pts_sorted[0], pts_sorted[1]];
        let mut bottom = [pts_sorted[2], pts_sorted[3]];

        top.sort_by_key(|&(x, _)| x); // left to right
        bottom.sort_by_key(|&(x, _)| -x); // right to left

        (
            [
                top[0].into(),
                top[1].into(),
                bottom[0].into(),
                bottom[1].into(),
            ],
            is_vertical,
        )
    } else {
        pts_sorted.sort_by_key(|&(x, _)| x);

        let mut left = [pts_sorted[0], pts_sorted[1]];
        let mut right = [pts_sorted[2], pts_sorted[3]];

        left.sort_by_key(|&(_, y)| y); // top to bottom
        right.sort_by_key(|&(_, y)| y); // top to bottom

        (
            [
                left[0].into(),
                right[0].into(),
                right[1].into(),
                left[1].into(),
            ],
            is_vertical,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MyPoint<T = i64>
where
    T: Copy,
{
    pub x: T,
    pub y: T,
}

impl<T: Copy> From<(T, T)> for MyPoint<T> {
    fn from((x, y): (T, T)) -> Self {
        Self { x, y }
    }
}
impl MyPoint<i64> {
    pub fn scale(self, factor: f64) -> Self {
        Self {
            x: (self.x as f64 * factor) as i64,
            y: (self.y as f64 * factor) as i64,
        }
    }
    pub fn dist(&self, other: &Self) -> f64 {
        f64::sqrt((self.x - other.x).pow(2) as f64 + (self.y - other.y).pow(2) as f64)
    }

    fn euclidean_norm(&self) -> f64 {
        ((self.x.pow(2) + self.y.pow(2)) as f64).sqrt()
    }

    pub fn to_f64(self) -> MyPoint<f64> {
        MyPoint {
            x: self.x as f64,
            y: self.y as f64,
        }
    }
}

impl MyPoint<f64> {
    pub fn to_point(self) -> Point {
        Point::new(self.x, self.y)
    }
    pub fn dist(&self, other: &Self) -> f64 {
        f64::sqrt((self.x - other.x).powi(2) + (self.y - other.y).powi(2))
    }
    fn euclidean_norm(&self) -> f64 {
        ((self.x.powi(2) + self.y.powi(2)) as f64).sqrt()
    }

    pub fn norm(self) -> f64 {
        self.x.hypot(self.y)
    }

    pub fn to_i64(self) -> MyPoint<i64> {
        MyPoint {
            x: self.x as i64,
            y: self.y as i64,
        }
    }
}

impl<T: Copy> MyPoint<T> {
    pub fn to_tuple(self) -> (T, T) {
        (self.x, self.y)
    }
}

impl<T: Add<Output = T> + Copy> Add for MyPoint<T> {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }
}

impl<T: Sub<Output = T> + Copy> Sub for MyPoint<T> {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }
}

impl<T: Add<Output = T> + Div<Output = T> + Copy> MyPoint<T> {
    pub fn midpoint(self, other: Self, two: T) -> Self {
        Self {
            x: (self.x + other.x) / two,
            y: (self.y + other.y) / two,
        }
    }
}

impl<T: Div<Output = T> + Copy> Div for MyPoint<T> {
    type Output = Self;

    fn div(self, other: Self) -> Self {
        Self {
            x: self.x / other.x,
            y: self.y / other.y,
        }
    }
}

impl<T: Div<Output = T> + Copy> Div<T> for MyPoint<T> {
    type Output = Self;

    fn div(self, rhs: T) -> Self::Output {
        Self {
            x: self.x / rhs,
            y: self.y / rhs,
        }
    }
}
impl<T: Mul<Output = T> + Copy> Mul<T> for MyPoint<T> {
    type Output = Self;

    fn mul(self, rhs: T) -> Self::Output {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
        }
    }
}

impl<T: Copy + Mul<Output = T> + Add<Output = T>> MyPoint<T> {
    fn dot(self, rhs: Self) -> T {
        self.x * rhs.x + self.y * rhs.y
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_sort_pnts2() {
        let pts = [(0, 0), (10, 0), (0, 1), (10, 1)];
        // This forms a horizontal rectangle
        // Width is 10, height is 1, so long sides are horizontal
        let expected = [
            MyPoint::from((0, 0)),
            MyPoint::from((10, 0)),
            MyPoint::from((10, 1)),
            MyPoint::from((0, 1)),
        ];
        let actual = sort_pnts(pts);

        assert_eq!(actual, (expected, false))
    }

    #[test]
    fn test_sort_pnts() {
        let pts = [(169, 6), (207, 6), (169, 164), (207, 164)];
        let expected = [
            MyPoint::from((169, 6)),
            MyPoint::from((207, 6)),
            MyPoint::from((207, 164)),
            MyPoint::from((169, 164)),
        ];
        let actual = sort_pnts(pts);
        assert_eq!(actual, (expected, true))
    }

    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn test_quadrilateral_new_and_accessors() {
        let points = vec![(0, 0), (10, 0), (10, 1), (0, 1)];
        // allow:clone[test]
        let quad = Quadrilateral::new(points.clone(), 0.9);

        assert_eq!(
            quad.pts(),
            &[
                MyPoint::from((0, 0)),
                MyPoint::from((10, 0)),
                MyPoint::from((10, 1)),
                MyPoint::from((0, 1))
            ]
        );
        assert_eq!(quad.score(), 0.9);
        assert!(!quad.vertical());
    }

    #[test]
    fn test_area() {
        let points = vec![(0, 0), (4, 0), (4, 3), (0, 3)];
        let quad = Quadrilateral::new(points, 1.0);

        let area = quad.area();
        assert_relative_eq!(area, 12.0, epsilon = 1e-6);
    }

    #[test]
    fn test_aspect_ratio_horizontal() {
        let points = vec![(0, 0), (10, 0), (10, 2), (0, 2)];
        let quad = Quadrilateral::new(points, 1.0);

        let aspect = quad.aspect_ratio();
        assert!(aspect > 4.5 && aspect < 5.5); // 10 / 2
    }

    #[test]
    fn test_aspect_ratio_vertical() {
        let points = vec![(0, 0), (2, 0), (2, 10), (0, 10)];
        let quad = Quadrilateral::new(points, 1.0);

        let aspect = quad.aspect_ratio();
        assert!(aspect < 0.3);
    }

    #[test]
    fn clip_to_keeps_points_inside_and_drops_empty() {
        let q = Quadrilateral::new(vec![(-4, -2), (12, -1), (13, 20), (-3, 18)], 0.8);
        let clipped = q.clip_to(10, 8).expect("nonzero after clip");
        for p in clipped.pts() {
            assert!((0..=10).contains(&p.x));
            assert!((0..=8).contains(&p.y));
        }
        let outside = Quadrilateral::new(vec![(20, 20), (24, 20), (24, 28), (20, 28)], 0.5);
        assert!(outside.clip_to(10, 8).is_none());
    }

    #[test]
    fn test_structure_midpoints() {
        let points = vec![(0, 0), (10, 0), (10, 4), (0, 4)];
        let quad = Quadrilateral::new(points, 1.0);
        let structure = quad.structure();

        assert_eq!(structure[0], MyPoint::from((5, 0)));
        assert_eq!(structure[1], MyPoint::from((5, 4)));
        assert_eq!(structure[2], MyPoint::from((10, 2)));
        assert_eq!(structure[3], MyPoint::from((0, 2)));
    }
}
