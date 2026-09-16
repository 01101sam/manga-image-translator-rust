//! 目标框与排版适配（横竖共用）。
//!
//! 渲染只读 `TextBlock::axis`（存储轴），方向不在此重算。

use textline_merge::TextBlock;

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
pub fn dest_axes(dst: [(i64, i64); 4]) -> (f32, f32) {
    let mid = |a: (i64, i64), b: (i64, i64)| {
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

#[cfg(test)]
mod tests {
    use interface_detector::textlines::MyPoint;
    use interface_translator::LangIdDetector;

    use super::*;

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
}
