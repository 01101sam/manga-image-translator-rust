"""Export a fixed-canvas lama_mpe ONNX that ORT's CoreML EP can run as a single partition.

The dynamic-shape model.onnx builds its FFT from Range/Sin/Cos + Einsum at run time and computes every
Reshape shape from the input size, which CoreML cannot host (300+ partitions, slower than the CPU EP).
Freezing the canvas folds all of that into constants; the remaining rewrites below replace the few ops
the CoreML builders still reject. Every rewrite is an exact algebraic equivalent (measured output drift
vs the dynamic model on the CPU EP: max abs 2.4e-6 in [0, 1] float space).

usage:
    python3 -m venv .venv --system-site-packages
    .venv/bin/pip install onnx onnx-simplifier
    .venv/bin/python scripts/lama_mpe_static_export.py models/inpainter/lama_mpe/model.onnx WIDTH HEIGHT [OUT.onnx]

OUT defaults to model_static_{WIDTH}x{HEIGHT}.onnx next to the source. Both sides must be multiples of 16
(the FFC operates on the /8 feature map and needs an even size there). Repeat per canvas tier registered in
crates/modules/inpainter/lama_mpe/src/lib.rs. The exported model takes `direct` as float32 (the dynamic model
takes int64); the other inputs keep their dtype.
"""
import hashlib
import sys
from collections import Counter
from pathlib import Path

import numpy as np
import onnx
import onnxsim
from onnx import helper, numpy_helper, shape_inference

# ORT 1.22 (bundled by the Rust `ort` crate) rejects any CoreML tensor with a dimension above this.
COREML_DIM_LIMIT = 16384


def op_histogram(model):
    return sorted(Counter(n.op_type for n in model.graph.node).items(), key=lambda kv: -kv[1])


def value_shapes(model):
    inferred = shape_inference.infer_shapes(model)
    infos = list(inferred.graph.value_info) + list(model.graph.input) + list(model.graph.output)
    return {vi.name: [d.dim_value for d in vi.type.tensor_type.shape.dim] for vi in infos}


def replace_nodes(graph, nodes):
    del graph.node[:]
    graph.node.extend(nodes)
    # Stale inferred shapes from before the rewrite would otherwise be merged into the new graph.
    del graph.value_info[:]


def fold_and_check(model, **kwargs):
    model, ok = onnxsim.simplify(model, check_n=0, **kwargs)
    assert ok, "onnxsim reported a broken graph"
    return model


def rewrite_coreml_unsupported(model):
    """Einsum, Neg, rank-5 Gather and output_padding ConvTranspose have no CoreML builder."""
    g = model.graph
    inits = {t.name: t for t in g.initializer}
    producer = {o: n for n in g.node for o in n.output}
    shapes = value_shapes(model)

    def const(name, arr):
        g.initializer.append(numpy_helper.from_array(arr, name))
        return name

    neg_one = const("static_export_neg_one", np.array(-1.0, dtype=np.float32))
    counts = Counter()
    ordered = []
    for n in g.node:
        ordered.append(n)
        if n.op_type == "Einsum":
            assert helper.get_attribute_value(n.attribute[0]) == b"ij,jk->ik"
            n.CopyFrom(helper.make_node("MatMul", list(n.input), list(n.output), name=n.name))
            counts["Einsum -> MatMul"] += 1
        elif n.op_type == "Neg":
            n.CopyFrom(helper.make_node("Mul", [n.input[0], neg_one], list(n.output), name=n.name))
            counts["Neg -> Mul"] += 1
        elif n.op_type == "Gather" and len(shapes.get(n.input[0], [])) == 5:
            # The FourierUnit reads real/imag back out of [1,C,H,W,2]; that tensor is a Reshape+Transpose of
            # the rank-4 conv output whose channels interleave (re, im), so gather even/odd channels there.
            idx = int(numpy_helper.to_array(inits[n.input[1]]))
            transpose = producer[n.input[0]]
            assert transpose.op_type == "Transpose" and list(helper.get_attribute_value(transpose.attribute[0])) == [0, 1, 3, 4, 2]
            assert helper.get_attribute_value(n.attribute[0]) == 4
            reshape = producer[transpose.input[0]]
            assert reshape.op_type == "Reshape" and numpy_helper.to_array(inits[reshape.input[1]]).tolist()[:3] == [1, -1, 2]
            conv_out = reshape.input[0]
            channels = const(f"{n.name}_channels", np.arange(idx, shapes[conv_out][1], 2, dtype=np.int64))
            n.CopyFrom(helper.make_node("Gather", [conv_out, channels], list(n.output), name=n.name, axis=1))
            counts["rank-5 Gather -> channel Gather"] += 1
        elif n.op_type == "ConvTranspose":
            attrs = {a.name: helper.get_attribute_value(a) for a in n.attribute}
            if "output_padding" not in attrs:
                continue
            # output_padding only shortens the end-side crop of the full transposed conv, so run it uncropped
            # and take the same window with a Slice.
            pads = list(attrs.pop("pads"))
            attrs.pop("output_padding")
            out_h, out_w = shapes[n.output[0]][2:]
            uncropped = n.output[0] + "_uncropped"
            starts = const(f"{n.name}_starts", np.array(pads[:2], dtype=np.int64))
            ends = const(f"{n.name}_ends", np.array([pads[0] + out_h, pads[1] + out_w], dtype=np.int64))
            axes = const(f"{n.name}_axes", np.array([2, 3], dtype=np.int64))
            ordered.append(helper.make_node("Slice", [uncropped, starts, ends, axes], [n.output[0]], name=n.name + "_crop"))
            n.CopyFrom(helper.make_node("ConvTranspose", list(n.input), [uncropped], name=n.name, pads=[0, 0, 0, 0], **attrs))
            counts["ConvTranspose output_padding -> Slice"] += 1
    replace_nodes(g, ordered)
    return counts


def rewrite_for_ort_1_22(model):
    """Dimension limit, reflect Pad and free-standing Cast are rejected by ORT 1.22's CoreML EP."""
    g = model.graph
    inits = {t.name: t for t in g.initializer}
    producer = {o: n for n in g.node for o in n.output}
    shapes = value_shapes(model)
    graph_inputs = {vi.name for vi in g.input}
    int_consts = {}

    def const(name, arr):
        g.initializer.append(numpy_helper.from_array(arr, name))
        return name

    def i64(vals):
        key = tuple(vals)
        if key not in int_consts:
            int_consts[key] = const("static_export_i64_" + "_".join(map(str, vals)), np.array(vals, dtype=np.int64))
        return int_consts[key]

    def transposed(name):
        t_name = name + "_T"
        if t_name not in inits:
            inits[t_name] = numpy_helper.from_array(numpy_helper.to_array(inits[name]).T.copy(), t_name)
            g.initializer.append(inits[t_name])
        return t_name

    counts = Counter()
    ordered = []
    rename = {}
    for n in g.node:
        for i, name in enumerate(n.input):
            n.input[i] = rename.get(name, name)
        if n.op_type == "Reshape" and n.input[0] in graph_inputs and max(shapes[n.output[0]]) > COREML_DIM_LIMIT:
            # MPE flattens H*W to look up embeddings; Gather/MatMul accept the unflattened input directly and the
            # Reshape back to [1,H,W,C] then becomes an identity that onnxsim removes.
            rename[n.output[0]] = n.input[0]
            counts["MPE flatten bypassed"] += 1
            continue
        if n.op_type == "Cast" and n.input[0] in graph_inputs:
            # The CoreML Cast builder only accepts a Cast that follows ArgMax; retype the graph input instead.
            (vi,) = [vi for vi in g.input if vi.name == n.input[0]]
            vi.type.tensor_type.elem_type = helper.get_attribute_value(n.attribute[0])
            rename[n.output[0]] = n.input[0]
            counts[f"Cast folded into input '{n.input[0]}'"] += 1
            continue
        if n.op_type == "MatMul" and n.input[0] in inits and len(shapes.get(n.input[1], [])) == 2 and shapes[n.input[1]][1] > COREML_DIM_LIMIT:
            # Inverse DFT as A[K,K] @ X[K,M] with M = C*H*W flattened by a Reshape from [K,1,C,W]: run the same
            # product as [1,C,W,K] @ A^T so no dimension exceeds the limit (Transposes replace both Reshapes).
            reshape_in = producer[n.input[1]]
            if reshape_in.op_type == "Reshape":
                assert shapes[reshape_in.input[0]][1] == 1
                reshape_in.CopyFrom(helper.make_node("Transpose", [reshape_in.input[0]], list(reshape_in.output), name=reshape_in.name, perm=[1, 2, 3, 0]))
            (reshape_out,) = [c for c in g.node if n.output[0] in c.input]
            assert reshape_out.op_type == "Reshape"
            reshape_out.CopyFrom(helper.make_node("Transpose", [reshape_out.input[0]], list(reshape_out.output), name=reshape_out.name, perm=[3, 0, 1, 2]))
            n.CopyFrom(helper.make_node("MatMul", [n.input[1], transposed(n.input[0])], list(n.output), name=n.name))
            counts["wide inverse-DFT MatMul -> batched"] += 1
        if n.op_type == "Pad" and helper.get_attribute_value(n.attribute[0]) == b"reflect":
            pads = numpy_helper.to_array(inits[n.input[1]]).tolist()
            x, shape = n.input[0], shapes[n.input[0]]
            assert len(shape) == 4 and pads[0] == pads[1] == pads[4] == pads[5] == 0

            def reflect(inp, axis, size, before, after, out):
                def row(lo):
                    name = f"{out}_s{lo}"
                    ordered.append(helper.make_node("Slice", [inp, i64([lo]), i64([lo + 1]), i64([axis])], [name]))
                    return name

                parts = [row(i) for i in range(before, 0, -1)] + [inp] + [row(size - 1 - i) for i in range(1, after + 1)]
                ordered.append(helper.make_node("Concat", parts, [out], name=out, axis=axis))

            reflect(x, 2, shape[2], pads[2], pads[6], n.name + "_h")
            reflect(n.name + "_h", 3, shape[3], pads[3], pads[7], n.output[0])
            counts["reflect Pad -> Slice+Concat"] += 1
            continue
        ordered.append(n)
    replace_nodes(g, ordered)
    return counts


def export(src, width, height, dst):
    assert width % 16 == 0 and height % 16 == 0, "canvas sides must be multiples of 16"
    model = onnx.load(src)
    print(f"{src}: {len(model.graph.node)} nodes")
    canvas = {"image": [1, 3, height, width], "mask": [1, 1, height, width], "rel_pos": [1, height, width], "direct": [1, height, width, 4]}
    model = fold_and_check(model, overwrite_input_shapes=canvas)
    print(f"frozen {width}x{height} + folded: {len(model.graph.node)} nodes")
    print("pass 1:", dict(rewrite_coreml_unsupported(model)))
    model = fold_and_check(model)
    print("pass 2:", dict(rewrite_for_ort_1_22(model)))
    model = fold_and_check(model)
    print(f"final: {len(model.graph.node)} nodes {op_histogram(model)}")
    # ORT keys its CoreML compiled-model cache on this (alnum, <= 64 chars) when present; otherwise it hashes
    # the file path and a re-exported file at the same path would silently reuse a stale compiled model.
    cache_key = hashlib.sha256(model.SerializeToString()).hexdigest()[:32]
    del model.metadata_props[:]
    model.metadata_props.add(key="COREML_CACHE_KEY", value=cache_key)
    onnx.save(model, dst)
    print(f"saved {dst} ({Path(dst).stat().st_size / 1e6:.1f} MB, COREML_CACHE_KEY {cache_key})")


if __name__ == "__main__":
    if len(sys.argv) not in (4, 5):
        sys.exit(__doc__)
    src, width, height = Path(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
    dst = sys.argv[4] if len(sys.argv) == 5 else str(src.with_name(f"model_static_{width}x{height}.onnx"))
    export(str(src), width, height, dst)
