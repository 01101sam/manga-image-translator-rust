use interface_image::RawImage;

use crate::TextBlock;

pub fn sort_regions(
    regions: Vec<TextBlock>,
    right_to_left: bool,
    img: Option<&RawImage>,
    force_simple: bool,
) -> Vec<TextBlock> {
    if regions.is_empty() {
        return regions;
    }
    if force_simple {
        return simple_sort(regions, right_to_left);
    }
    if let Some(img) = img {
        if let Some(panels) = detect_sorted_panels(img, right_to_left) {
            return sort_into_panels(regions, panels, right_to_left);
        }
        return simple_sort(regions, right_to_left);
    }
    smart_sort(regions, right_to_left)
}

fn detect_sorted_panels(
    img: &RawImage,
    right_to_left: bool,
) -> Option<Vec<(i32, i32, i32, i32)>> {
    let rgb = rgb_bytes(img);
    let panels = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        kumiko::detect_panels(
            rgb,
            img.width as u32,
            img.height as u32,
            !right_to_left,
            None,
            true,
        )
    }))
    .ok()?;
    if panels.is_empty() {
        return None;
    }
    Some(sort_panels_fill(
        panels.into_iter().map(|p| (p.x, p.y, p.r, p.b)).collect(),
        right_to_left,
    ))
}

fn sort_into_panels(
    regions: Vec<TextBlock>,
    panels: Vec<(i32, i32, i32, i32)>,
    right_to_left: bool,
) -> Vec<TextBlock> {
    let mut grouped: Vec<Vec<TextBlock>> = (0..panels.len()).map(|_| Vec::new()).collect();
    let mut leftover = Vec::new();
    for region in regions {
        let (cx, cy) = region.center();
        let idx = panels.iter().position(|&(x1, y1, x2, y2)| {
            cx >= x1 as i64 && cx <= x2 as i64 && cy >= y1 as i64 && cy <= y2 as i64
        });
        match idx {
            Some(i) => grouped[i].push(region),
            None => {
                let nearest = panels
                    .iter()
                    .enumerate()
                    .map(|(i, &(x1, y1, x2, y2))| {
                        let dx = (x1 as i64 - cx).max(0).max(cx - x2 as i64);
                        let dy = (y1 as i64 - cy).max(0).max(cy - y2 as i64);
                        (dx * dx + dy * dy, i)
                    })
                    .min_by_key(|(d, _)| *d);
                match nearest {
                    Some((_, i)) => grouped[i].push(region),
                    None => leftover.push(region),
                }
            }
        }
    }

    let mut sorted = Vec::new();
    for group in grouped {
        sorted.extend(smart_sort(group, right_to_left));
    }
    sorted.extend(smart_sort(leftover, right_to_left));
    sorted
}

fn rgb_bytes(img: &RawImage) -> Vec<u8> {
    if img.channels == 3 {
        return img.data.clone();
    }
    img.data
        .chunks(img.channels as usize)
        .flat_map(|c| [c[0], c[1], c[2]])
        .collect()
}

fn smart_sort(regions: Vec<TextBlock>, right_to_left: bool) -> Vec<TextBlock> {
    if regions.len() <= 1 {
        return regions;
    }
    let xs = regions.iter().map(|r| r.center().0 as f64).collect::<Vec<_>>();
    let ys = regions.iter().map(|r| r.center().1 as f64).collect::<Vec<_>>();
    let x_std = stddev(&xs);
    let y_std = stddev(&ys);
    if x_std > y_std {
        group_sort(regions, right_to_left, true)
    } else {
        group_sort(regions, right_to_left, false)
    }
}

fn group_sort(mut regions: Vec<TextBlock>, right_to_left: bool, horizontal: bool) -> Vec<TextBlock> {
    regions.sort_by(|a, b| {
        let (ac, bc) = (a.center(), b.center());
        if horizontal {
            let ax = if right_to_left { -ac.0 } else { ac.0 };
            let bx = if right_to_left { -bc.0 } else { bc.0 };
            ax.cmp(&bx)
        } else {
            ac.1.cmp(&bc.1)
        }
    });
    let mut sorted = Vec::new();
    let mut group = Vec::new();
    let mut prev: Option<i64> = None;
    let gap = if horizontal { 20 } else { 15 };
    for region in regions {
        let key = if horizontal {
            region.center().0
        } else {
            region.center().1
        };
        if let Some(p) = prev {
            if (key - p).abs() > gap {
                sort_group(&mut group, right_to_left, horizontal);
                sorted.append(&mut group);
            }
        }
        prev = Some(key);
        group.push(region);
    }
    sort_group(&mut group, right_to_left, horizontal);
    sorted.append(&mut group);
    sorted
}

fn sort_group(group: &mut [TextBlock], right_to_left: bool, horizontal: bool) {
    group.sort_by(|a, b| {
        let (ac, bc) = (a.center(), b.center());
        if horizontal {
            ac.1.cmp(&bc.1)
        } else if right_to_left {
            bc.0.cmp(&ac.0)
        } else {
            ac.0.cmp(&bc.0)
        }
    });
}

pub fn simple_sort(regions: Vec<TextBlock>, right_to_left: bool) -> Vec<TextBlock> {
    let mut sorted: Vec<TextBlock> = Vec::new();
    let mut incoming = regions;
    incoming.sort_by_key(|r| r.center().1);
    for region in incoming {
        let cy = region.center().1;
        let rx = region.center().0;
        let insert_at = sorted.iter().position(|other| {
            let (_, y1, _, y2) = other.xyxy();
            if cy > y2 {
                return false;
            }
            if cy < y1 {
                return true;
            }
            let ox = other.center().0;
            (right_to_left && rx > ox) || (!right_to_left && rx < ox)
        });
        match insert_at {
            Some(i) => sorted.insert(i, region),
            None => sorted.push(region),
        }
    }
    sorted
}

pub fn sort_panels_fill(
    mut panels: Vec<(i32, i32, i32, i32)>,
    right_to_left: bool,
) -> Vec<(i32, i32, i32, i32)> {
    if panels.is_empty() {
        return panels;
    }
    panels.sort_by_key(|p| p.1);
    let avg_h = panels.iter().map(|p| (p.3 - p.1) as f64).sum::<f64>() / panels.len() as f64;
    let y_thr = 10.0_f64.max(avg_h * 0.3);
    let mut remaining = panels;
    let mut ordered = Vec::new();
    while !remaining.is_empty() {
        let base_y = remaining[0].1 as f64;
        let mut row = Vec::new();
        let mut i = 0;
        while i < remaining.len() {
            if ((remaining[i].1 as f64) - base_y).abs() <= y_thr {
                row.push(remaining.remove(i));
            } else {
                i += 1;
            }
        }
        row.sort_by(|a, b| {
            if right_to_left {
                b.0.cmp(&a.0)
            } else {
                a.0.cmp(&b.0)
            }
        });
        ordered.extend(row);
    }
    ordered
}

fn stddev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let var = values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / values.len() as f64;
    var.sqrt()
}

#[cfg(test)]
mod tests {
    use interface_detector::textlines::MyPoint;

    use super::*;
    use crate::{TextBlock, script_axis_from_line_quads};

    fn block(x: i64, y: i64, w: i64, h: i64) -> TextBlock {
        let lines = vec![[
            MyPoint { x, y },
            MyPoint { x: x + w, y },
            MyPoint { x: x + w, y: y + h },
            MyPoint { x, y: y + h },
        ]];
        let axis = script_axis_from_line_quads(&lines);
        TextBlock {
            lines,
            text: format!("{x},{y}"),
            font_size: 12,
            angle: 0.0,
            prob: 1.0,
            fg_color: None,
            bg_color: None,
            skip_translate: false,
            language: None,
            translations: Default::default(),
            axis,
        }
    }

    #[test]
    fn simple_sort_rtl_reads_right_first_in_a_row() {
        let sorted = simple_sort(vec![block(10, 10, 20, 20), block(80, 12, 20, 20)], true);
        assert_eq!(sorted[0].text, "80,12");
        assert_eq!(sorted[1].text, "10,10");
    }

    #[test]
    fn force_simple_matches_simple_sort() {
        let blocks = vec![block(10, 10, 20, 20), block(80, 12, 20, 20)];
        let forced = sort_regions(blocks, true, None, true);
        assert_eq!(forced[0].text, "80,12");
        assert_eq!(forced[1].text, "10,10");
    }

    #[test]
    fn sort_regions_empty() {
        assert!(sort_regions(vec![], true, None, false).is_empty());
    }

    #[test]
    fn panel_fill_keeps_top_row_rtl() {
        let panels = vec![(0, 0, 50, 50), (60, 0, 110, 50), (0, 80, 50, 130)];
        let ordered = sort_panels_fill(panels, true);
        assert_eq!(ordered[0], (60, 0, 110, 50));
        assert_eq!(ordered[1], (0, 0, 50, 50));
        assert_eq!(ordered[2], (0, 80, 50, 130));
    }
}
