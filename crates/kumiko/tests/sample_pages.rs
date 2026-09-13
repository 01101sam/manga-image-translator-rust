use std::path::PathBuf;

use kumiko::{detect_panels, Panel};

fn detect(name: &str) -> (u32, u32, Vec<Panel>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../imgs")
        .join(name);
    let img = image::open(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .to_rgb8();
    let (w, h) = (img.width(), img.height());
    // 样张是日漫，编号用 rtl（ltr=false）
    let panels = detect_panels(img.into_raw(), w, h, false, None, true);
    println!("{name} {w}x{h} panels={}", panels.len());
    for (i, p) in panels.iter().enumerate() {
        println!("  {i} x={} y={} w={} h={}", p.x, p.y, p.w(), p.h());
    }
    (w, h, panels)
}

fn full_page(p: &Panel, w: u32, h: u32) -> bool {
    p.x <= 8 && p.y <= 8 && p.w() >= w as i32 - 16 && p.h() >= h as i32 - 16
}

#[test]
fn two_panel_page_is_stacked_not_full_page() {
    let (w, h, panels) = detect("232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png");
    assert_eq!(w, 1487);
    assert_eq!(h, 2048);
    assert_eq!(panels.len(), 2);
    assert!(panels.iter().all(|p| !full_page(p, w, h)));
    let top = &panels[0];
    let bottom = &panels[1];
    assert!(
        top.b <= bottom.y + 8,
        "panels should follow top-to-bottom reading order without overlap"
    );
    assert!(top.h() < h as i32 / 2);
    assert!(bottom.h() > h as i32 / 2);
}

#[test]
fn four_panel_page_keeps_left_column_and_right_stack() {
    let (w, h, panels) = detect("232264684-5a7bcf8e-707b-4925-86b0-4212382f1680.png");
    assert_eq!(w, 2297);
    assert_eq!(h, 3123);
    assert_eq!(panels.len(), 4);
    assert!(
        !panels.iter().any(|p| full_page(p, w, h)),
        "collapsed to a full-page panel"
    );
    let left = panels.iter().min_by_key(|p| p.x).unwrap();
    assert!(left.w() > w as i32 / 2);
    assert!(left.h() > h as i32 * 9 / 10);
    assert!(panels[..3].iter().all(|p| p.x > w as i32 / 2));
    assert!(panels[3].x < w as i32 / 2);
    assert!(
        panels[..3].windows(2).all(|pair| pair[0].b <= pair[1].y),
        "right-column panels should precede the left column in top-to-bottom order"
    );
}
