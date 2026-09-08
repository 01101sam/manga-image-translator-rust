use std::sync::Mutex;

use ndarray::{s, Array4, ArrayView1, ArrayViewD, Ix3};
use ort::{inputs, session::Session, value::Tensor};

/// Runs the session synchronously on the calling thread; see `dbnet` for why `RunAsync` is avoided.
pub fn decode(
    model: &Mutex<Session>,
    img: Array4<f32>,
    blank: i64,
) -> anyhow::Result<Vec<Vec<(i64, f32, f32, f32, f32, f32, f32, f32)>>> {
    let mut model = model.lock().expect("session mutex poisoned");
    let out = model.run(inputs! {"images"=> Tensor::from_array(img)?})?;
    let logits: ArrayViewD<f32> = out["pred_char_logits"].try_extract_array()?;
    let logits = logits.into_dimensionality::<Ix3>()?;
    let colors: ArrayViewD<f32> = out["pred_color_values"].try_extract_array()?;
    let colors = colors.into_dimensionality::<Ix3>()?;
    let (batch, steps, _) = logits.dim();
    let mut pred_chars = vec![Vec::new(); batch];
    for b in 0..batch {
        let mut last_ch = blank;
        for t in 0..steps {
            let row = logits.slice(s![b, t, ..]);
            let (pred_ch, max) = argmax(row);
            if pred_ch != last_ch && pred_ch != blank {
                // Log-softmax of the argmax entry only: the whole (steps x vocab) log-softmax
                // that used to be materialized was 40% of the per-page time.
                let lp = -row.iter().map(|&x| (x - max).exp()).sum::<f32>().ln();
                let c = |i| f32::clamp(colors[[b, t, i]], 0.0, 1.0);
                pred_chars[b].push((pred_ch, lp, c(0), c(1), c(2), c(3), c(4), c(5)));
            }
            last_ch = pred_ch;
        }
    }
    Ok(pred_chars)
}

/// First index of the maximum, matching `ndarray_stats::QuantileExt::argmax`.
fn argmax(row: ArrayView1<f32>) -> (i64, f32) {
    row.iter()
        .enumerate()
        .fold((0, f32::NEG_INFINITY), |(bi, bv), (i, &v)| {
            if v > bv {
                (i as i64, v)
            } else {
                (bi, bv)
            }
        })
}
