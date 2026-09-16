use interface_detector::textlines::{MyPoint, Quadrilateral, long_edge_is_vertical};
use serde::{Deserialize, Serialize};

use crate::TextBlock;

/// 块级阅读/排版轴：区域出生时由行几何投票一次，之后只读。
///
/// 列序右→左写进名字：竖排新列一律画在左边（−x），首列在最右。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScriptAxis {
    /// 字形沿 +x，行沿 +y。
    #[default]
    Horizontal,
    /// 字形直立沿 +y 堆叠，列沿 −x 前进（右→左）。
    VerticalRtl,
}

impl ScriptAxis {
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::VerticalRtl)
    }
}

/// 页级先验的含糊判定：块 AABB 宽高比落在此容差内视为近方形。
pub const PAGE_PRIOR_ASPECT_TOLERANCE: f64 = 1.25;

/// 每行 `long_edge_is_vertical` 一票，再多数决。
///
/// 不读 `Quadrilateral.vertical` 旗标：48px OCR 会覆写它，换后端会换结果；
/// 纯几何投票让同一组行在任何后端、任何时刻投出同一方向。
/// 空输入判横（与旧 `vertical()` 的零面积回退一致）。
pub fn script_axis_from_line_quads(lines: &[[MyPoint; 4]]) -> ScriptAxis {
    if lines.is_empty() {
        return ScriptAxis::Horizontal;
    }
    let (horizontal, vertical) = count_line_axes(lines);
    let count = horizontal + vertical;
    if vertical * 2 == count {
        return most_extreme_line_axis(lines);
    }
    if vertical * 2 > count {
        ScriptAxis::VerticalRtl
    } else {
        ScriptAxis::Horizontal
    }
}

/// 页级方向先验：含糊块随页级非含糊块的面积加权多数。
///
/// 无先验（全含糊）或加权持平时不改任何块。
pub fn apply_page_axis_prior(blocks: &mut [TextBlock]) {
    let ambiguous: Vec<bool> = blocks.iter().map(is_ambiguous_block).collect();
    let mut horizontal_area = 0i64;
    let mut vertical_area = 0i64;
    for (block, &is_ambiguous) in blocks.iter().zip(&ambiguous) {
        if is_ambiguous {
            continue;
        }
        let (x1, y1, x2, y2) = block.xyxy();
        let area = (x2 - x1).max(0) * (y2 - y1).max(0);
        match block.axis {
            ScriptAxis::Horizontal => horizontal_area += area,
            ScriptAxis::VerticalRtl => vertical_area += area,
        }
    }
    if horizontal_area == vertical_area {
        return;
    }
    let prior = if vertical_area > horizontal_area {
        ScriptAxis::VerticalRtl
    } else {
        ScriptAxis::Horizontal
    };
    for (block, &is_ambiguous) in blocks.iter_mut().zip(&ambiguous) {
        if is_ambiguous {
            block.axis = prior;
        }
    }
}

/// 返回（横票数，竖票数）。
fn count_line_axes(lines: &[[MyPoint; 4]]) -> (u32, u32) {
    let mut horizontal = 0;
    let mut vertical = 0;
    for line in lines {
        if long_edge_is_vertical(line_to_tuples(line)) {
            vertical += 1;
        } else {
            horizontal += 1;
        }
    }
    (horizontal, vertical)
}

fn line_to_tuples(line: &[MyPoint; 4]) -> [(i64, i64); 4] {
    [
        line[0].to_tuple(),
        line[1].to_tuple(),
        line[2].to_tuple(),
        line[3].to_tuple(),
    ]
}

/// 持平时取宽高比（或其倒数）最极端一行的轴。
fn most_extreme_line_axis(lines: &[[MyPoint; 4]]) -> ScriptAxis {
    // 初值沿用旧 merge 投票：全退化行（aspect 非数）时判竖。
    let mut best_ratio = -100.0;
    let mut best_axis = ScriptAxis::VerticalRtl;
    for line in lines {
        let aspect = Quadrilateral::new2(line.to_vec(), 0.0).aspect_ratio();
        if aspect.max(1.0 / aspect) > best_ratio {
            best_ratio = aspect.max(1.0 / aspect);
            best_axis = if long_edge_is_vertical(line_to_tuples(line)) {
                ScriptAxis::VerticalRtl
            } else {
                ScriptAxis::Horizontal
            };
        }
    }
    best_axis
}

/// 含糊块：组内投票持平，或近方形且文本短。
fn is_ambiguous_block(block: &TextBlock) -> bool {
    if !block.lines.is_empty() {
        let (horizontal, vertical) = count_line_axes(&block.lines);
        if horizontal == vertical {
            return true;
        }
    }
    let (x1, y1, x2, y2) = block.xyxy();
    let (w, h) = ((x2 - x1) as f64, (y2 - y1) as f64);
    if w > 0.0 && h > 0.0 {
        let aspect = w / h;
        if (1.0 / PAGE_PRIOR_ASPECT_TOLERANCE..=PAGE_PRIOR_ASPECT_TOLERANCE).contains(&aspect)
            && block.text.chars().count() < 3
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(x: i64, y: i64, w: i64, h: i64) -> [MyPoint; 4] {
        [
            MyPoint { x, y },
            MyPoint { x: x + w, y },
            MyPoint { x: x + w, y: y + h },
            MyPoint { x, y: y + h },
        ]
    }

    fn test_block(lines: Vec<[MyPoint; 4]>, text: &str, axis: ScriptAxis) -> TextBlock {
        TextBlock {
            lines,
            text: text.to_owned(),
            font_size: 30,
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
    fn unanimous_and_majority_votes() {
        let v = |x| quad(x, 0, 10, 100);
        let h = |y| quad(0, y, 100, 10);
        assert_eq!(
            script_axis_from_line_quads(&[v(0), v(20), v(40)]),
            ScriptAxis::VerticalRtl
        );
        assert_eq!(
            script_axis_from_line_quads(&[h(0), h(20), h(40)]),
            ScriptAxis::Horizontal
        );
        assert_eq!(
            script_axis_from_line_quads(&[v(0), v(20), h(0)]),
            ScriptAxis::VerticalRtl
        );
        assert_eq!(
            script_axis_from_line_quads(&[h(0), h(20), v(0)]),
            ScriptAxis::Horizontal
        );
    }

    #[test]
    fn tie_break_picks_most_extreme_line() {
        // 1:1 持平：横行 aspect=10 对竖行 aspect=0.5（倒数 2）→ 横胜。
        assert_eq!(
            script_axis_from_line_quads(&[quad(0, 0, 100, 10), quad(0, 0, 20, 40)]),
            ScriptAxis::Horizontal
        );
        // 反向：竖行 aspect=0.05（倒数 20）对横行 aspect=2 → 竖胜。
        assert_eq!(
            script_axis_from_line_quads(&[quad(0, 0, 5, 100), quad(0, 0, 40, 20)]),
            ScriptAxis::VerticalRtl
        );
    }

    #[test]
    fn vote_beats_max_area_aabb() {
        // 最大行 AABB 宽高比 5:1（按最大行判会说横），但行多数竖 → 判竖。
        let lines = [
            quad(0, 0, 100, 20),
            quad(0, 40, 10, 60),
            quad(20, 40, 10, 60),
            quad(40, 40, 10, 60),
        ];
        let areas: Vec<i64> = lines
            .iter()
            .map(|q| {
                let xs = q.map(|p| p.x);
                let ys = q.map(|p| p.y);
                (xs.iter().max().unwrap() - xs.iter().min().unwrap())
                    * (ys.iter().max().unwrap() - ys.iter().min().unwrap())
            })
            .collect();
        let biggest = areas.iter().enumerate().max_by_key(|(_, a)| *a).unwrap().0;
        assert_eq!(biggest, 0);
        assert_eq!(
            script_axis_from_line_quads(&lines),
            ScriptAxis::VerticalRtl
        );
    }

    #[test]
    fn empty_lines_default_horizontal() {
        assert_eq!(script_axis_from_line_quads(&[]), ScriptAxis::Horizontal);
    }

    #[test]
    fn constructor_stores_same_vote() {
        let det = interface_translator::LangIdDetector::new().unwrap();
        let lines = vec![quad(0, 0, 100, 20), quad(0, 40, 10, 60), quad(20, 40, 10, 60)];
        let expected = script_axis_from_line_quads(&lines);
        let block = TextBlock::new(
            lines,
            vec!["a".into(), "b".into(), "c".into()],
            30,
            0.0,
            1.0,
            None,
            None,
            &det,
        );
        assert_eq!(block.axis(), expected);
        assert_eq!(block.vertical(), expected.is_vertical());
    }

    #[test]
    fn ambiguous_block_follows_page_majority() {
        // 页级横多数：近方小块（短文本，初判竖）改横。
        let mut blocks = vec![
            test_block(vec![quad(0, 0, 300, 40)], "这是横排长文本块一", ScriptAxis::Horizontal),
            test_block(vec![quad(0, 100, 300, 40)], "这是横排长文本块二", ScriptAxis::Horizontal),
            test_block(vec![quad(0, 200, 30, 30)], "啊", ScriptAxis::VerticalRtl),
        ];
        apply_page_axis_prior(&mut blocks);
        assert_eq!(blocks[2].axis(), ScriptAxis::Horizontal);
        // 反向：页级竖多数。
        let mut blocks = vec![
            test_block(
                vec![quad(0, 0, 40, 300)],
                "这是竖排长文本块一",
                ScriptAxis::VerticalRtl,
            ),
            test_block(
                vec![quad(100, 0, 40, 300)],
                "这是竖排长文本块二",
                ScriptAxis::VerticalRtl,
            ),
            test_block(vec![quad(200, 0, 30, 30)], "啊", ScriptAxis::Horizontal),
        ];
        apply_page_axis_prior(&mut blocks);
        assert_eq!(blocks[2].axis(), ScriptAxis::VerticalRtl);
    }

    #[test]
    fn clear_blocks_keep_axis_against_majority() {
        // 非含糊块即使与页级多数相悖也不改。
        let mut blocks = vec![
            test_block(vec![quad(0, 0, 300, 40)], "横排长文本块一", ScriptAxis::Horizontal),
            test_block(vec![quad(0, 100, 300, 40)], "横排长文本块二", ScriptAxis::Horizontal),
            test_block(vec![quad(0, 200, 300, 40)], "横排长文本块三", ScriptAxis::Horizontal),
            test_block(vec![quad(400, 0, 40, 300)], "竖排长文本块", ScriptAxis::VerticalRtl),
        ];
        apply_page_axis_prior(&mut blocks);
        assert!(
            blocks[..3]
                .iter()
                .all(|b| b.axis() == ScriptAxis::Horizontal)
        );
        assert_eq!(blocks[3].axis(), ScriptAxis::VerticalRtl);
    }

    #[test]
    fn tied_votes_make_block_ambiguous() {
        // 组内 1:1 持平 → 含糊，随页级多数（即使文本很长）。
        let mut blocks = vec![
            test_block(vec![quad(0, 0, 300, 40)], "横排长文本块一", ScriptAxis::Horizontal),
            test_block(vec![quad(0, 100, 300, 40)], "横排长文本块二", ScriptAxis::Horizontal),
            test_block(
                vec![quad(0, 200, 100, 10), quad(0, 220, 10, 100)],
                "持平块但文本很长很长",
                ScriptAxis::VerticalRtl,
            ),
        ];
        apply_page_axis_prior(&mut blocks);
        assert_eq!(blocks[2].axis(), ScriptAxis::Horizontal);
    }

    #[test]
    fn page_without_prior_keeps_axes() {
        // 全含糊页：无先验，保持原轴。
        let mut blocks = vec![
            test_block(vec![quad(0, 0, 30, 30)], "啊", ScriptAxis::VerticalRtl),
            test_block(vec![quad(50, 0, 30, 30)], "哦", ScriptAxis::Horizontal),
        ];
        apply_page_axis_prior(&mut blocks);
        assert_eq!(blocks[0].axis(), ScriptAxis::VerticalRtl);
        assert_eq!(blocks[1].axis(), ScriptAxis::Horizontal);
        // 先验加权持平（等面积）：同样不改。
        let mut blocks = vec![
            test_block(vec![quad(0, 0, 200, 50)], "横排长文本块", ScriptAxis::Horizontal),
            test_block(vec![quad(0, 100, 50, 200)], "竖排长文本块", ScriptAxis::VerticalRtl),
            test_block(vec![quad(300, 0, 30, 30)], "啊", ScriptAxis::VerticalRtl),
        ];
        apply_page_axis_prior(&mut blocks);
        assert_eq!(blocks[2].axis(), ScriptAxis::VerticalRtl);
    }
}
