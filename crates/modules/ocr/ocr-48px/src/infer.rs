use std::{collections::HashMap, mem, sync::Mutex};

use base_util::ndarray_utils::as_slice;
use ndarray::{
    s, stack, Array, Array1, Array2, Array3, Array4, Array5, ArrayView2, ArrayView3, ArrayViewD,
    Axis, Ix2,
};
use ordered_float::OrderedFloat;
use ort::{
    inputs,
    session::{Session, SessionOutputs},
    value::{Tensor, Value, ValueTypeMarker},
};
const DECODER_LAYER_COUNT: u8 = 5;

use crate::hypo::Hypothesis;

pub struct Pred {
    pub out_idx: Vec<i64>,
    pub prob: f32,
    pub fg_pred: Vec<(f32, f32, f32)>,
    pub bg_pred: Vec<(f32, f32, f32)>,
    pub fg_ind_pred: Vec<(f32, f32)>,
    pub bg_ind_pred: Vec<(f32, f32)>,
}

fn stack_input_mask(input_mask: ArrayView2<bool>, hypos: &[Hypothesis]) -> Array2<bool> {
    let arrays_to_stack: Vec<Array1<bool>> = hypos
        .iter()
        .map(|hyp| input_mask.row(hyp.memory_idx as usize).to_owned())
        .collect();

    stack(
        Axis(0),
        &arrays_to_stack.iter().map(|a| a.view()).collect::<Vec<_>>(),
    )
    .expect("stack failed")
}

fn next_token_batch<'b, T: ValueTypeMarker>(
    hypos: &mut Vec<Hypothesis>,
    memory: &'b Value,
    memory_mask: &'b Value<T>,
    decoder: &'b mut Session,
    beams_k: i32,
) -> SessionOutputs<'b> {
    let n = hypos.len();
    let offset = Tensor::from_array(Array::from_elem((), hypos[0].len() as i64)).unwrap();
    let num_layers = DECODER_LAYER_COUNT as usize;
    let l_glob = hypos[0].out_idx.len();
    let e = memory.shape()[2] as usize;
    let mut activation_cache: Array5<f32> = Array5::zeros((n, num_layers + 1, l_glob, 1, e));
    for (n, hyp) in hypos.iter().enumerate() {
        for l in 0..num_layers + 1 {
            let layer_act = &hyp.cached_activations[l];
            if layer_act.len() > 0 {
                let prev_l = layer_act.shape()[0];
                let squeezed = layer_act.index_axis(Axis(1), 0);
                activation_cache
                    .slice_mut(s![n, l, ..prev_l, 0, ..])
                    .assign(&squeezed);
            }
        }
    }
    let last_toks = hypos
        .iter()
        .map(|h| *h.out_idx.last().unwrap())
        .collect::<Vec<_>>();
    let last_toks = Tensor::from_array(Array1::from(last_toks)).unwrap();
    let memory_idxs_tensor = Tensor::from_array(Array1::from(
        hypos.iter().map(|v| v.memory_idx).collect::<Vec<_>>(),
    ))
    .unwrap();
    let l = Tensor::from_array(Array::from_elem((), l_glob as i64)).unwrap();
    let beams_k = Tensor::from_array(Array::from_elem((), beams_k as i32)).unwrap();
    let activation_cache = Tensor::from_array(activation_cache).unwrap();
    let out = decoder.run(inputs!{"memory" => memory, "memory_mask" => memory_mask, "last_toks"=>last_toks, "memory_idxs_tensor"=>memory_idxs_tensor, "activation_cache"=>activation_cache, "offset"=>offset, "L"=>l, "beams_k" => beams_k}).unwrap();
    let activation_cache: ArrayViewD<f32> = out
        .get("out_activation_cache")
        .unwrap()
        .try_extract_array()
        .unwrap();

    for n in 0..n {
        let acts = (0..(num_layers + 1))
            .map(|l| {
                if l_glob > 0 {
                    let a: ArrayView3<f32> = activation_cache.slice(s![n, l, ..l_glob, .., ..]);
                    a.to_owned()
                } else {
                    Array3::zeros((0, 1, e))
                }
            })
            .collect::<Vec<_>>();
        hypos[n].cached_activations = acts;
    }
    out
}

/// Runs the sessions synchronously on the calling thread; see `dbnet` for why `RunAsync` is avoided.
pub fn infer(
    encoder: &Mutex<Session>,
    decoder: &Mutex<Session>,
    color_pred: &Mutex<Session>,
    img: Array4<f32>,
    new_widths: Vec<i32>,
    start_tok: i64,
    end_tok: i64,
    beams_k: i32,
    max_seq_length: i32,
    max_finished_hypos: usize,
) -> Vec<Pred> {
    let n = img.shape()[0];
    let img = Tensor::from_array(img).unwrap();
    let img_widths = Tensor::from_array(Array1::from(new_widths)).unwrap();
    let mut encoder = encoder.lock().expect("session mutex poisoned");
    let out = encoder
        .run(inputs! {"img" => img, "img_widths" => img_widths})
        .unwrap();
    let memory = out.get("memory").unwrap();
    let input_mask = out.get("input_mask").unwrap();
    let mut decoder = decoder.lock().expect("session mutex poisoned");
    let mut hypos = (0..n as i64)
        .map(|i| Hypothesis::new(DECODER_LAYER_COUNT, 320, i, start_tok, end_tok))
        .collect::<Vec<_>>();
    let out = next_token_batch(&mut hypos, memory, input_mask, &mut decoder, beams_k);
    let pred_chars_values = out
        .get("out_pred_chars_values")
        .unwrap()
        .try_extract_array::<f32>()
        .unwrap()
        .into_dimensionality::<Ix2>()
        .unwrap();
    let pred_chars_index = out
        .get("out_pred_chars_index")
        .unwrap()
        .try_extract_array::<i64>()
        .unwrap()
        .into_dimensionality::<Ix2>()
        .unwrap();
    let mut hypos = hypos
        .iter()
        .enumerate()
        .flat_map(|(i, hypo)| {
            (0..beams_k as usize).map(move |k| {
                // allow:clone[beam search]
                hypo.clone().extend(
                    *pred_chars_index.get((i, k)).unwrap(),
                    *pred_chars_values.get((i, k)).unwrap(),
                )
            })
        })
        .collect::<Vec<_>>();
    drop(out);
    let mut hypos_per_sample = Default::default();
    let mut finished_hypos: HashMap<i64, Vec<Hypothesis>> = HashMap::new();
    for _ in 0..max_seq_length {
        let im = input_mask.try_extract_array().unwrap();
        let im = stack_input_mask(im.into_dimensionality().unwrap(), &hypos);
        let im = Tensor::from_array(im).unwrap();
        let out = next_token_batch(&mut hypos, memory, &im, &mut decoder, beams_k);
        let pred_chars_values = out
            .get("out_pred_chars_values")
            .unwrap()
            .try_extract_array::<f32>()
            .unwrap()
            .into_dimensionality::<Ix2>()
            .unwrap();
        let pred_chars_index = out
            .get("out_pred_chars_index")
            .unwrap()
            .try_extract_array::<i64>()
            .unwrap()
            .into_dimensionality::<Ix2>()
            .unwrap();
        hypos_per_sample = HashMap::new();

        for (i, h) in hypos.iter().enumerate() {
            let entries = (0..beams_k as usize)
                .map(|k| {
                    // allow:clone[beam search]
                    h.clone().extend(
                        *pred_chars_index.get((i, k)).unwrap(),
                        *pred_chars_values.get((i, k)).unwrap(),
                    )
                })
                .collect::<Vec<_>>();

            hypos_per_sample
                .entry(h.memory_idx)
                .or_insert_with(Vec::new)
                .extend(entries);
        }
        hypos = Vec::new();
        for (i, cur_hypos) in hypos_per_sample.iter_mut() {
            cur_hypos.sort_by_key(|v| OrderedFloat(v.sort_key()));
            let cur_hypos = &cur_hypos[..=(cur_hypos.len() - 1).min(beams_k as usize)];
            let mut to_added_hypos = vec![];
            let mut sample_done = false;
            for h in cur_hypos {
                if h.seq_end() {
                    let entry = finished_hypos.entry(*i).or_default();
                    entry.push(h.clone());
                    if entry.len() >= max_finished_hypos {
                        sample_done = true;
                        break;
                    }
                } else if to_added_hypos.len() < beams_k as usize {
                    to_added_hypos.push(h);
                }
            }
            if !sample_done {
                hypos.extend(to_added_hypos.into_iter().cloned());
            }
        }
        if hypos.is_empty() {
            break;
        }
    }
    for i in 0..n as i64 {
        if !finished_hypos.contains_key(&i) {
            let cur_hypos = hypos_per_sample.get_mut(&i).unwrap();
            cur_hypos.sort_by_key(|a| OrderedFloat(a.sort_key()));
            let cur_hypo = cur_hypos.remove(0);
            finished_hypos.entry(i).or_default().push(cur_hypo);
        }
    }

    assert_eq!(finished_hypos.iter().len(), n);
    let mut res = Vec::new();
    let mut color_pred = color_pred.lock().expect("session mutex poisoned");

    for i in 0..n as i64 {
        let mut cur_hypos = finished_hypos.remove(&i).unwrap();
        cur_hypos.sort_by_key(|a| OrderedFloat(a.sort_key()));
        let mut cur_hypo = cur_hypos.remove(0);
        let mut out_idx = vec![];
        mem::swap(&mut out_idx, &mut cur_hypo.out_idx);
        let prob = cur_hypo.prob();
        let decoded = Tensor::from_array(cur_hypo.output_owned()).unwrap();
        let out = color_pred.run(inputs![decoded]).unwrap();

        let fg_pred = out
            .get("fg_pred")
            .unwrap()
            .try_extract_array::<f32>()
            .unwrap();
        let bg_pred = out
            .get("bg_pred")
            .unwrap()
            .try_extract_array::<f32>()
            .unwrap();
        let fg_ind_pred = out
            .get("fg_ind_pred")
            .unwrap()
            .try_extract_array::<f32>()
            .unwrap();
        let bg_ind_pred = out
            .get("bg_ind_pred")
            .unwrap()
            .try_extract_array::<f32>()
            .unwrap();

        out_idx.remove(0);
        res.push(Pred {
            out_idx,
            prob,
            fg_pred: to_tuple3(as_slice(fg_pred).to_vec()),
            bg_pred: to_tuple3(as_slice(bg_pred).to_vec()),
            fg_ind_pred: to_tuple2(as_slice(fg_ind_pred).to_vec()),
            bg_ind_pred: to_tuple2(as_slice(bg_ind_pred).to_vec()),
        });
    }
    res
}

fn to_tuple2(items: Vec<f32>) -> Vec<(f32, f32)> {
    assert!(items.len() % 2 == 0, "Length must be a multiple of 2");

    let len = items.len() / 2;
    let ptr = items.as_ptr() as *const (f32, f32);

    std::mem::forget(items);

    unsafe { Vec::from_raw_parts(ptr as *mut (f32, f32), len, len) }
}

fn to_tuple3(items: Vec<f32>) -> Vec<(f32, f32, f32)> {
    assert!(items.len() % 3 == 0, "Length must be a multiple of 3");

    let len = items.len() / 3;
    let ptr = items.as_ptr() as *const (f32, f32, f32);

    std::mem::forget(items);

    unsafe { Vec::from_raw_parts(ptr as *mut (f32, f32, f32), len, len) }
}
