use std::f64::consts::PI;

use crate::panel::Point;

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct Segment {
    a: [i32; 2],
    b: [i32; 2],
}

impl Segment {
    pub fn to_xyrb(&self) -> [u32; 4] {
        [
            self.left() as u32,
            self.top() as u32,
            self.right() as u32,
            self.bottom() as u32,
        ]
    }
    pub fn new(start: [i32; 2], end: [i32; 2]) -> Self {
        Self { a: start, b: end }
    }

    pub fn dist(&self) -> f64 {
        f64::sqrt(self.dist_x(false).pow(2) as f64 + self.dist_y(false).pow(2) as f64)
    }
    pub fn dist_x(&self, keep_sign: bool) -> i32 {
        let dist = self.b[0] - self.a[0];
        match keep_sign {
            true => dist,
            false => dist.abs(),
        }
    }
    pub fn dist_y(&self, keep_sign: bool) -> i32 {
        let dist = self.b[1] - self.a[1];
        match keep_sign {
            true => dist,
            false => dist.abs(),
        }
    }

    fn left(&self) -> i32 {
        i32::min(self.a[0], self.b[0])
    }

    fn top(&self) -> i32 {
        i32::min(self.a[1], self.b[1])
    }

    fn right(&self) -> i32 {
        i32::max(self.a[0], self.b[0])
    }

    fn bottom(&self) -> i32 {
        i32::max(self.a[1], self.b[1])
    }

    pub fn center(&self) -> Point {
        Point::new(
            self.left() + self.dist_x(false) / 2,
            self.top() + self.dist_y(false) / 2,
        )
    }

    pub fn projected_point(&self, point: &Point) -> Point {
        let a = Point::new(self.a[0], self.a[1]);
        let b = Point::new(self.b[0], self.b[1]);
        let ap = *point - a;
        let ab = b - a;
        if ab == Point::zero() {
            return a;
        }
        let result = a.to_f64() + ab.to_f64() * (ap.dot(&ab) as f64 / ab.dot(&ab) as f64);

        result.round_ties_even()
    }

    pub fn may_contain(&self, point: &Point) -> bool {
        point.x >= self.left()
            && point.x <= self.right()
            && point.y >= self.top()
            && point.y <= self.bottom()
    }

    fn intersect(&self, other: &Self) -> Option<Segment> {
        if !self.angle_ok_with(other) {
            return None;
        }
        let gutter = f64::max(self.dist(), other.dist()) * 5.0 / 100.0;

        // from here, segments are almost parallel

        // segments are apart ?
        if (self.right() as f64) < (other.left() as f64) - gutter|| // self left from other
				(self.left() as f64) > (other.right() as f64) + gutter|| // self right from other
				(self.bottom() as f64) < (other.top() as f64) - gutter|| // self above other
				(self.top() as f64) > (other.bottom() as f64) + gutter
        /*  self below other */
        {
            return None;
        }

        let projected_c = self.projected_point(&Point::from(other.a));
        let dist_c_to_ab = Segment::new(other.a, projected_c.into()).dist();

        let projected_d = self.projected_point(&Point::from(other.b));
        let dist_d_to_ab = Segment::new(other.b, projected_d.into()).dist();
        if (dist_c_to_ab + dist_d_to_ab) / 2.0 > gutter {
            return None;
        }

        let mut sorted_dots = [self.a, self.b, other.a, other.b];
        sorted_dots.sort_by_key(|p| p[0] + p[1]);
        Some(Segment::new(sorted_dots[1], sorted_dots[2]))
    }

    pub fn intersect_all(&self, segments: &[Segment]) -> Vec<Self> {
        let segments_match = segments
            .iter()
            .filter_map(|v| self.intersect(v))
            .collect::<Vec<_>>();

        Segment::union_all(segments_match)
    }

    fn union(&self, other: &Self) -> Option<Self> {
        self.intersect(other)?;
        let mut dots = [self.a, self.b, other.a, other.b];
        dots.sort_by_key(|p| p[0] + p[1]);
        Some(Segment::new(dots[0], dots[3]))
    }

    pub fn union_all(mut segments: Vec<Segment>) -> Vec<Segment> {
        let mut i = 0;
        while i < segments.len() {
            let mut j = i + 1;
            let mut merged = false;
            while j < segments.len() {
                if let Some(u) = segments[i].union(&segments[j]) {
                    segments[i] = u;
                    segments.swap_remove(j);
                    merged = true;
                } else {
                    j += 1;
                }
            }
            if !merged {
                i += 1;
            }
        }
        segments
    }

    fn angle_with(&self, other: &Self) -> f64 {
        (self.angle() - other.angle()).abs().to_degrees()
    }

    fn angle_ok_with(&self, other: &Self) -> bool {
        let angle = self.angle_with(other);
        angle < 10.0 || (angle - 180.0).abs() < 10.0
    }

    fn angle(&self) -> f64 {
        if self.dist_x(false) != 0 {
            f64::atan(self.dist_y(false) as f64 / self.dist_x(false) as f64)
        } else {
            PI / 2.0
        }
    }

    pub fn along_polygon(polygon: &[Point], mut i: usize, mut j: usize) -> Segment {
        let n = polygon.len();
        // 环形索引中，首个顶点的前一个顶点位于末尾。
        let wrap = |k: usize| (k + n - 1) % n;
        let dot1 = polygon[i];
        let dot2 = polygon[j];
        let mut split_segment = Segment::new(dot1.into(), dot2.into());
        loop {
            i = wrap(i);
            let add_segment = Segment::new(polygon[i].into(), polygon[(i + 1) % n].into());
            if add_segment.angle_ok_with(&split_segment) {
                split_segment = Segment::new(add_segment.a, split_segment.b);
            } else {
                break;
            }
        }

        loop {
            j = (j + 1) % n;
            let add_segment = Segment::new(polygon[wrap(j)].into(), polygon[j].into());
            if add_segment.angle_ok_with(&split_segment) {
                split_segment = Segment::new(split_segment.a, add_segment.b)
            } else {
                break;
            }
        }

        split_segment
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_contained_segment_does_not_panic() {
        let a = Segment::new([0, 0], [10, 0]);
        let b = Segment::new([0, 0], [5, 0]);
        let u = a.union(&b).expect("overlap");
        assert_eq!(u.to_xyrb(), [0, 0, 10, 0]);
    }

    #[test]
    fn union_identical_does_not_panic() {
        let a = Segment::new([0, 0], [10, 0]);
        let u = a.union(&a).expect("identical");
        assert_eq!(u.to_xyrb(), [0, 0, 10, 0]);
    }

    #[test]
    fn union_all_merges_overlap() {
        let out = Segment::union_all(vec![
            Segment::new([0, 0], [5, 0]),
            Segment::new([4, 0], [10, 0]),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_xyrb(), [0, 0, 10, 0]);
    }

    #[test]
    fn projected_point_rounds_halfway_coordinates_to_even() {
        let diagonal = Segment::new([0, 0], [4, 4]);
        for (input, expected) in [(1, 0), (3, 2), (5, 2), (-1, 0), (-3, -2)] {
            let point = diagonal.projected_point(&Point::new(input, 0));
            assert_eq!([point.x, point.y], [expected, expected]);
        }
    }

    #[test]
    fn along_polygon_wraps_index_zero() {
        use crate::panel::Point;
        let poly = [
            Point::new(0, 0),
            Point::new(10, 0),
            Point::new(10, 10),
            Point::new(0, 10),
        ];
        let s = Segment::along_polygon(&poly, 0, 2);
        assert_eq!(s.to_xyrb(), [0, 0, 10, 10]);
    }
}
