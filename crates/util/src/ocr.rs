use std::{path::PathBuf, sync::Arc};

use base_util::opencv_utils::to_continous2;
use interface_detector::textlines::Quadrilateral;
use interface_image::{RawImage, RawImageCow};
use ndarray::{s, Array4, Axis};
use opencv::core::{Mat, MatTraitConst as _, MatTraitConstManual};
use parking_lot::Mutex;

use crate::{resize::get_transformed_region, text_direction::generate_text_direction};

pub fn prepare(
    image: &RawImage,
    areas: &[Arc<parking_lot::Mutex<Quadrilateral>>],
    text_height: u32,
    max_batch_size: usize,
    debug_path: &Option<PathBuf>,
) -> anyhow::Result<Vec<(Array4<f32>, Vec<i32>, Vec<Arc<Mutex<Quadrilateral>>>)>> {
    let img = image.clone().to_image().unwrap().to_rgb8();
    let mut kept = Vec::with_capacity(areas.len());
    for (v, ver) in generate_text_direction(areas.to_vec()) {
        v.lock().set_vert(ver);
        let t = get_transformed_region(&*v.lock(), &img, text_height)?;
        if let Some(t) = t {
            kept.push((t, v));
        }
    }
    let (region_imgs, areas): (Vec<_>, Vec<_>) = kept.into_iter().unzip();
    let mut perm: Vec<usize> = (0..region_imgs.len()).collect();
    perm.sort_by_key(|&i| {
        let aabb = areas[i].lock().aabb();
        let scale = text_height as f64 / aabb.w as f64;
        (aabb.h as f64 * scale) as u32
    });
    let v = perm
        .chunks(max_batch_size)
        .enumerate()
        .map(|(ii, indices)| {
            prepare_chunk(
                ii,
                indices,
                &region_imgs,
                &areas,
                text_height as usize,
                &debug_path,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

fn prepare_chunk(
    ii: usize,
    indices: &[usize],
    region_imgs: &[Mat],
    areas: &[Arc<parking_lot::Mutex<Quadrilateral>>],
    text_height: usize,
    debug_path: &Option<PathBuf>,
) -> anyhow::Result<(Array4<f32>, Vec<i32>, Vec<Arc<Mutex<Quadrilateral>>>)> {
    let n = indices.len();
    let img_slice = indices.iter().map(|v| &region_imgs[*v]).collect::<Vec<_>>();
    let areas = indices
        .iter()
        .map(|v| areas[*v].clone())
        .collect::<Vec<_>>();

    let widths = img_slice.iter().map(|v| v.cols()).collect::<Vec<_>>();
    let max_width = widths.iter().max().copied().unwrap_or_default() as usize + 7;
    let mut region = Array4::<u8>::zeros((n, text_height, max_width, 3));
    for (i, tmp) in img_slice.iter().enumerate() {
        let tmp = to_continous2(tmp);
        let data = tmp.data_bytes().expect("to_continous used");
        let rows = tmp.rows() as usize;
        let cols = tmp.cols() as usize;
        let row_stride = tmp.step1(0).unwrap();
        for y in 0..rows.min(text_height) {
            let row_start = y * row_stride;
            let row_end = row_start + (cols.min(max_width) * 3); // 3 channels
            let row_slice = &data[row_start..row_end];
            region
                .slice_mut(s![i, y, 0..cols.min(max_width), ..])
                .assign(
                    &ndarray::ArrayView::from_shape((cols.min(max_width), 3), row_slice).unwrap(),
                );
        }
    }
    if let Some(v) = debug_path {
        for (i, img) in region.axis_iter(Axis(0)).enumerate() {
            RawImageCow::from(img)
                .to_owned()
                .to_image()?
                .save(v.join(format!("patch_{ii}_{i}.png")))?
        }
    }
    let images = region
        .mapv(|v| (v as f32 - 127.5) / 127.5)
        .permuted_axes([0, 3, 1, 2]);
    Ok((images, widths, areas))
}
