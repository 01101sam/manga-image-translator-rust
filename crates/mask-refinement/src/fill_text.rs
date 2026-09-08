use anyhow::anyhow;
use base_util::{ndarray_utils, opencv_utils::as_slice};
use geo::{
    Area, BooleanOps as _, Centroid, ConvexHull as _, Distance as _, Euclidean, MultiPoint, Point,
};
use interface_detector::textlines::Quadrilateral;
use log::warn;
use ndarray::{concatenate, Array1, Array2, Array3, Axis};
use opencv::{
    boxed_ref::BoxedRefMut,
    core::{
        bitwise_or, no_array, Mat, MatExprTraitConst, MatTrait, MatTraitConst as _, Point_, Rect,
        Scalar, Size, BORDER_CONSTANT, BORDER_DEFAULT, CV_32S,
    },
    imgproc::{
        bilateral_filter, connected_components_with_stats, dilate, get_structuring_element,
        morphology_default_border_value, rectangle, CC_STAT_AREA, CC_STAT_HEIGHT, CC_STAT_LEFT,
        CC_STAT_TOP, CC_STAT_WIDTH, LINE_8, MORPH_ELLIPSE,
    },
};
use ordered_float::OrderedFloat;
use rayon::prelude::*;

#[derive(Debug, Clone)]
pub struct RatioMatSlice<'a, T> {
    data: &'a [T],
    rows: usize,
    cols: usize,
}

impl<'a, T> RatioMatSlice<'a, T> {
    pub fn new(data: &'a [T], rows: usize, cols: usize) -> Self {
        Self { data, rows, cols }
    }

    /// Get a reference to the element at (row, col) without bounds checks
    pub fn get(&self, row: usize, col: usize) -> &T {
        &self.data[row * self.cols + col]
    }
}

#[derive(Debug, Clone)]
pub struct RatioMat<T> {
    data: Vec<T>,
    rows: usize,
    cols: usize,
}

fn extend_rect(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    max_x: i32,
    max_y: i32,
    extend_size: i32,
) -> (i32, i32, i32, i32) {
    let x1 = i32::max(x - extend_size, 0);
    let y1 = i32::max(y - extend_size, 0);
    let w1 = i32::min(w + extend_size * 2, max_x - x1 - 1);
    let h1 = i32::min(h + extend_size * 2, max_y - y1 - 1);
    (x1, y1, w1, h1)
}

impl RatioMat<i32> {
    pub fn four_filled_with_rects(rows: usize) -> Self {
        let mut data = Vec::with_capacity(rows * 4);
        for _ in 0..rows {
            data.extend_from_slice(&[i32::MAX, i32::MAX, i32::MIN, i32::MIN]);
        }
        Self {
            data,
            rows,
            cols: 4,
        }
    }
}

impl<T: Default + Clone + Copy> RatioMat<T> {
    /// Create a new matrix filled with `T::default()`
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            data: vec![T::default(); rows * cols],
            rows,
            cols,
        }
    }

    /// Get a contiguous submatrix with boundary checks
    pub fn slice_contiguous(&self, y: usize, x: usize, h: usize, w: usize) -> Option<RatioMat<T>> {
        // Clamp start and end indices
        let y_start = y.min(self.rows);
        let x_start = x.min(self.cols);
        let y_end = (y + h).min(self.rows);
        let x_end = (x + w).min(self.cols);

        let new_h = y_end.saturating_sub(y_start);
        let new_w = x_end.saturating_sub(x_start);

        if new_h == 0 || new_w == 0 {
            return None;
        }

        let mut sub_data = Vec::with_capacity(new_h * new_w);

        for row in y_start..y_end {
            let start = row * self.cols + x_start;
            let end = start + new_w;
            sub_data.extend_from_slice(&self.data[start..end]);
        }

        Some(RatioMat {
            data: sub_data,
            rows: new_h,
            cols: new_w,
        })
    }

    /// Get a reference to the element at (row, col) without bounds checks
    pub fn get(&self, row: usize, col: usize) -> &T {
        &self.data[row * self.cols + col]
    }

    /// Set a single element (row, col)
    pub fn set(&mut self, row: usize, col: usize, value: T) {
        self.data[row * self.cols + col] = value;
    }

    /// Get a full row as a slice
    pub fn get_row(&self, row: usize) -> &[T] {
        let start = row * self.cols;
        &self.data[start..start + self.cols]
    }
}

pub fn apply_mask_ratio<T: Copy + Eq + Default>(
    textline_ccs: &mut RatioMat<u8>,
    labels: &RatioMatSlice<T>,
    y1: usize,
    x1: usize,
    h1: usize,
    w1: usize,
    label: T,
) {
    for y in y1..y1 + h1 {
        for x in x1..x1 + w1 {
            if *labels.get(y, x) == label {
                textline_ccs.set(y, x, 255);
            }
        }
    }
}

pub fn complete_mask(
    textlines: Vec<Quadrilateral>,
    img: BoxedRefMut<'_, Mat>,
    mut mask: BoxedRefMut<'_, Mat>,
    keep_threshold: f64,
    dilation_offset: f64,
    kernel_size: i32,
) -> anyhow::Result<Option<Mat>> {
    let bboxes = textlines.iter().map(|v| v.aabb()).collect::<Vec<_>>();
    let polys = textlines.iter().map(|v| v.polygon()).collect::<Vec<_>>();
    let mut labels = Mat::default();
    let mut stats = Mat::default();
    let mut centroids = Mat::default();

    for bbox in bboxes {
        let (x, y, w, h) = (bbox.x as i32, bbox.y as i32, bbox.w as i32, bbox.h as i32);
        rectangle(
            &mut mask,
            Rect::new(x, y, w, h),
            Scalar::all(0.0),
            1,
            LINE_8,
            0,
        )?;
    }
    let num_labels =
        connected_components_with_stats(&mask, &mut labels, &mut stats, &mut centroids, 8, CV_32S)?;
    let m = textlines.len();
    let mut ratio_mat: RatioMat<f64> = RatioMat::new(num_labels as usize, m);
    let mut dist_mat: RatioMat<f64> = RatioMat::new(num_labels as usize, m);
    let mut textline_rects = RatioMat::four_filled_with_rects(m);
    let mut valid = false;
    let mut textline_ccs: Vec<RatioMat<u8>> =
        vec![RatioMat::new(mask.rows() as usize, mask.cols() as usize); m];
    let label_rows = labels.rows() as usize;
    let label_cols = labels.cols() as usize;
    let labels = RatioMatSlice::new(as_slice(&mut labels), label_rows, label_cols);

    for label in 1..num_labels {
        if *stats.at_2d::<i32>(label, CC_STAT_AREA)? <= 9 {
            continue;
        }

        let x1 = *stats.at_2d::<i32>(label, CC_STAT_LEFT)?;
        let y1 = *stats.at_2d::<i32>(label, CC_STAT_TOP)?;
        let w1 = *stats.at_2d::<i32>(label, CC_STAT_WIDTH)?;
        let h1 = *stats.at_2d::<i32>(label, CC_STAT_HEIGHT)?;
        let area1 = *stats.at_2d::<i32>(label, CC_STAT_AREA)?;
        let cc_pts = [[x1, y1], [x1 + w1, y1], [x1 + w1, y1 + h1], [x1, y1 + h1]]
            .into_iter()
            .map(|v| Point::new(v[0] as f64, v[1] as f64));

        let cc_poly = MultiPoint::from_iter(cc_pts).convex_hull();
        let label_u = label as usize;
        for (tl_idx, poly) in polys.iter().enumerate() {
            let area2 = poly.unsigned_area();
            let overlapping_area = poly.intersection(&cc_poly).unsigned_area();
            ratio_mat.set(label_u, tl_idx, overlapping_area / area2.min(area1 as f64));
            dist_mat.set(
                label_u,
                tl_idx,
                Euclidean.distance(poly, &cc_poly.centroid().ok_or(anyhow!("No centroid"))?),
            );
        }
        let mut avg = ratio_mat
            .get_row(label_u)
            .iter()
            .enumerate()
            .max_by(|(i1, v1), (i2, v2)| {
                OrderedFloat(**v1).cmp(&OrderedFloat(**v2)).then(i2.cmp(i1)) // reverse to prefer the smaller index
            })
            .map(|v| v.0)
            .ok_or(anyhow!("unexpected shape"))?;
        let area2 = polys[avg].unsigned_area();
        if area1 as f64 >= area2 {
            continue;
        }

        if *ratio_mat.get(label_u, avg) <= keep_threshold {
            avg = dist_mat
                .get_row(label_u)
                .iter()
                .enumerate()
                .min_by(|(i1, v1), (i2, v2)| {
                    OrderedFloat(**v1).cmp(&OrderedFloat(**v2)).then(i2.cmp(i1))
                })
                .map(|v| v.0)
                .ok_or(anyhow!("empty row"))?;
            let unit = textlines[avg]
                .font_size()
                .min(w1 as f64)
                .min(h1 as f64)
                .max(10.0);
            if *dist_mat.get(label_u, avg) >= 0.5 * unit {
                continue;
            }
        }

        apply_mask_ratio(
            &mut textline_ccs[avg],
            &labels,
            y1 as usize,
            x1 as usize,
            h1 as usize,
            w1 as usize,
            label,
        );
        textline_rects.set(avg, 0, *textline_rects.get(avg, 0).min(&x1));
        textline_rects.set(avg, 1, *textline_rects.get(avg, 1).min(&y1));
        textline_rects.set(avg, 2, *textline_rects.get(avg, 2).max(&(x1 + w1)));
        textline_rects.set(avg, 3, *textline_rects.get(avg, 3).max(&(y1 + h1)));
        valid = true
    }

    if !valid {
        return Ok(None);
    }

    let mut final_mask = Mat::zeros_size(mask.size()?, mask.typ())?.to_mat()?;
    // Each textline's DenseCRF is independent and single-threaded, so run them in parallel and
    // merge the dilated regions afterwards (bitwise or is order independent).
    let dilated: Vec<anyhow::Result<Option<(Rect, Mat)>>> = textline_ccs
        .par_iter_mut()
        .enumerate()
        .map(|(i, cc)| {
            let [x1, y1, x2, y2] = textline_rects.get_row(i).try_into()?;
            if x1 == i32::MAX || y1 == i32::MAX || x2 == i32::MIN || y2 == i32::MIN {
                warn!("x or y coordinate not updated");
                return Ok(None);
            }
            let w1 = x2 - x1;
            let h1 = y2 - y1;

            let text_size = textlines[i].font_size().min(w1.min(h1) as f64);
            let (x1, y1, w1, h1) = extend_rect(
                x1,
                y1,
                w1,
                h1,
                img.cols(),
                img.rows(),
                (text_size * 0.1) as i32,
            );
            // TODO: Need to think of better way to determine dilate_size.
            let dilate_size = (((text_size + dilation_offset) * 0.3) as i32 / 2 * 2 + 1).max(3);
            let kern = get_structuring_element(
                MORPH_ELLIPSE,
                Size::new(dilate_size, dilate_size),
                Point_::new(-1, -1),
            )?;
            let cc_region =
                cc.slice_contiguous(y1 as usize, x1 as usize, h1 as usize, w1 as usize);
            let cc_region = match cc_region {
                Some(v) => v,
                None => return Ok(None),
            };

            let x1 = x1.clamp(0, img.cols() - 1);
            let y1 = y1.clamp(0, img.rows() - 1);

            let w1 = ((x1 + w1).min(img.cols()) - x1).max(0);
            let h1 = ((y1 + h1).min(img.rows()) - y1).max(0);

            let roi = Rect::new(x1, y1, w1, h1);

            // A non-isolated ROI borrows its border pixels from the parent image, so filtering
            // the ROI equals filtering the whole image and cropping.
            // ponytail: overlapping ROIs filter shared pixels twice; filter their union once if
            // that ever shows up in a profile.
            let mut roi_mat = Mat::default();
            bilateral_filter(
                &Mat::roi(&img, roi)?,
                &mut roi_mat,
                17,
                80.0,
                80.0,
                BORDER_DEFAULT,
            )?;
            let img_region = as_slice(&mut roi_mat);
            let cc_region = refine_mask(img_region, w1 as u32, h1 as u32, cc_region)?;

            let cc_region_shape = cc_region.shape();
            let mut cc = Mat::from_slice_mut(&mut cc.data)?;
            let mut cc = cc.reshape_mut(1, img.rows() as i32)?;
            let cc_region = ndarray_utils::as_slice(cc_region.view());
            let cc_region = Mat::from_slice(cc_region.as_ref())?;
            let cc_region = cc_region.reshape(1, cc_region_shape[0] as i32)?;
            let mut roi_mat = Mat::roi_mut(&mut cc, roi)?;
            cc_region.copy_to(&mut roi_mat)?;
            let (x2, y2, w2, h2) =
                extend_rect(x1, y1, w1, h1, img.cols(), img.rows(), -(-dilate_size / 2));
            let x2 = x2.clamp(0, cc.cols() - 1);
            let y2 = y2.clamp(0, cc.rows() - 1);

            let w2 = ((x2 + w2).min(cc.cols()) - x2).max(0);
            let h2 = ((y2 + h2).min(cc.rows()) - y2).max(0);
            let roi = Rect::new(x2, y2, w2, h2);
            let src = Mat::roi(&cc, roi)?;
            let mut temp = Mat::default();
            dilate(
                &src,
                &mut temp,
                &kern,
                Point_::new(-1, -1),
                1,
                BORDER_CONSTANT,
                morphology_default_border_value()?,
            )?;
            Ok(Some((roi, temp)))
        })
        .collect();
    for region in dilated {
        let Some((roi, temp)) = region? else { continue };
        let mut roi_mat = Mat::roi_mut(&mut final_mask, roi)?;
        let roi_ptr: *mut BoxedRefMut<'_, Mat> = &mut roi_mat;
        unsafe {
            bitwise_or(&*roi_ptr, &temp, &mut *roi_ptr, &no_array())?;
        }
    }

    let kern = get_structuring_element(
        MORPH_ELLIPSE,
        Size::new(kernel_size, kernel_size),
        Point_::new(-1, -1),
    )?;
    let mut dst = Mat::default();
    dilate(
        &final_mask,
        &mut dst,
        &kern,
        Point_::new(-1, -1),
        1,
        BORDER_CONSTANT,
        morphology_default_border_value()?,
    )?;

    Ok(Some(dst))
}

fn mask_to_feat_first(rawmask: Array2<u8>) -> anyhow::Result<Array2<f32>> {
    let (h, w) = rawmask.dim();

    // (H, W) -> (H, W, 1)
    let rawmask = rawmask.into_shape((h, w, 1))?;

    let invmask = rawmask.map(|&v| !v);

    let mask_softmax: Array3<u8> = concatenate(Axis(2), &[invmask.view(), rawmask.view()])?;

    let mask_softmax = mask_softmax.map(|&v| v as f32 / 255.0);

    let mask_softmax = mask_softmax.permuted_axes([2, 0, 1]);

    Ok(mask_softmax.into_shape_clone((2, h * w))?)
}

fn unary_from_softmax(sm: Array2<f32>, clip: Option<f32>) -> anyhow::Result<Array2<f32>> {
    let num_cls = sm.shape()[0];

    let sm = if let Some(c) = clip {
        sm.mapv(|x| x.max(c).min(1.0))
    } else {
        sm
    };

    let rest = sm.len() / num_cls;
    let sm_flat = sm.into_shape((num_cls, rest))?;

    Ok(sm_flat.mapv(|x| -x.ln()))
}

fn refine_mask(
    rgbimg: &[u8],
    width: u32,
    height: u32,
    rawmask: RatioMat<u8>,
) -> anyhow::Result<Array2<u8>> {
    let feat_first = mask_to_feat_first(Array2::from_shape_vec(
        (rawmask.rows, rawmask.cols),
        rawmask.data,
    )?)?;

    let mut unary = unary_from_softmax(feat_first, Some(1e-5))?;
    let unary = match unary.as_slice() {
        Some(slice) => slice,
        None => {
            unary = unary.to_owned();
            unary
                .as_slice()
                .ok_or(anyhow::anyhow!("is not contiguous"))?
        }
    };
    let res = densecrf::densecrf(unary, width, height, 2, rgbimg, 5)?;
    Ok(res
        .axis_iter(Axis(1))
        .map(|v| if v[0] >= v[1] { 0u8 } else { 255u8 })
        .collect::<Array1<_>>()
        .into_shape((height as usize, width as usize))?)
}
