use image::{DynamicImage, GenericImageView};
use kumiko::detect_panels;

fn main() {
    let path = std::env::args().nth(1).expect("image path");
    let img = image::open(path).unwrap().to_rgb8();
    let w = img.width();
    let h = img.height();
    let panels = detect_panels(img.clone().into_raw(), w, h, true, None, true);
    let img = DynamicImage::from(img);
    println!("{}", panels.len());
    for (i, panel) in panels.into_iter().enumerate() {
        let img = img.view(
            panel.x as u32,
            panel.y as u32,
            panel.w() as u32,
            panel.h() as u32,
        );
        img.to_image().save(format!("{i}.png")).unwrap();
    }
}
