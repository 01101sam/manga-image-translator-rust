"""Export pixai-tagger-v0.9 (frozen wd-eva02-large encoder + linear head + sigmoid, see the repo's
handler.py) to a static [1,3,448,448] ONNX and the tag table crates/modules/tagger/pixai reads.

usage:
    python3 -m venv /tmp/pixai-venv --system-site-packages
    /tmp/pixai-venv/bin/pip install torch torchvision timm onnx onnxsim onnxruntime
    /tmp/pixai-venv/bin/python scripts/pixai_tagger_export.py SNAPSHOT_DIR [OUT_DIR] [--check IMAGE]

SNAPSHOT_DIR is the HF snapshot holding model_v0.9.pth, tags_v0.9_13k.json and char_ip_map.json.
OUT_DIR defaults to models/tagger/pixai. `--check IMAGE` runs the PyTorch reference and the exported
ONNX (CPU EP) on IMAGE with handler.py's preprocessing and prints the max abs probability drift.
"""
import hashlib
import json
import sys
from collections import Counter
from pathlib import Path

import numpy as np
import onnx
import onnxsim
import timm
import torch
from onnx import helper, numpy_helper

INPUT_SIZE = 448
NUM_FEATURES = 1024
NUM_CLASSES = 13461


class TaggingHead(torch.nn.Module):
    def __init__(self):
        super().__init__()
        self.head = torch.nn.Sequential(torch.nn.Linear(NUM_FEATURES, NUM_CLASSES))

    def forward(self, x):
        return torch.sigmoid(self.head(x))


def build_model(weights):
    encoder = timm.create_model("hf_hub:SmilingWolf/wd-eva02-large-tagger-v3", pretrained=False)
    encoder.reset_classifier(0)
    model = torch.nn.Sequential(encoder, TaggingHead())
    model.load_state_dict(torch.load(weights, map_location="cpu", weights_only=True))
    return model.eval()


def tag_table(snapshot):
    info = json.loads((snapshot / "tags_v0.9_13k.json").read_text(encoding="utf-8"))
    names = [None] * len(info["tag_map"])
    for name, idx in info["tag_map"].items():
        names[idx] = name
    assert all(n is not None for n in names) and len(names) == NUM_CLASSES
    gen = info["tag_split"]["gen_tag_count"]
    ip_map = json.loads((snapshot / "char_ip_map.json").read_text(encoding="utf-8"))
    # The 20 MB map covers 267k Danbooru characters; only the model's 3.7k character tags can fire.
    ips = [sorted(set(ip_map.get(n, []))) for n in names[gen:]]
    return {"gen_tag_count": gen, "names": names, "ips": ips}


def export_onnx(model, dst):
    torch.onnx.export(
        model,
        torch.zeros(1, 3, INPUT_SIZE, INPUT_SIZE),
        str(dst),
        input_names=["input"],
        output_names=["probs"],
        opset_version=17,
        dynamo=False,
    )
    exported = onnx.load(str(dst))
    print(f"exported: {len(exported.graph.node)} nodes")
    simplified, ok = onnxsim.simplify(exported, check_n=0)
    assert ok, "onnxsim reported a broken graph"
    print(
        f"folded: {len(simplified.graph.node)} nodes, {replace_neg(simplified)} Neg -> Mul(-1),"
        f" {pretranspose_gemm(simplified)} Gemm B pre-transposed"
    )
    # ORT names its CoreML compiled-model cache directory after this (alnum, <= 64 chars) when present;
    # otherwise it hashes the file path and a re-exported file at the same path would reuse a stale
    # compiled model (onnxruntime/core/providers/coreml/coreml_execution_provider.cc, kCOREML_CACHE_KEY).
    cache_key = hashlib.sha256(simplified.SerializeToString()).hexdigest()[:32]
    del simplified.metadata_props[:]
    simplified.metadata_props.add(key="COREML_CACHE_KEY", value=cache_key)
    onnx.save(simplified, str(dst))
    print(f"saved {dst} ({dst.stat().st_size / 1e6:.1f} MB, {len(simplified.graph.node)} nodes, COREML_CACHE_KEY {cache_key})")


def replace_neg(model):
    """ORT 1.22's CoreML EP has no Neg builder, so RoPE's two Neg per block cut the graph into 25
    partitions (each a separate CoreML model to compile and load); Mul by -1 is the same math."""
    g = model.graph
    g.initializer.append(numpy_helper.from_array(np.array(-1.0, dtype=np.float32), "neg_one"))
    negs = [n for n in g.node if n.op_type == "Neg"]
    for n in negs:
        n.CopyFrom(helper.make_node("Mul", [n.input[0], "neg_one"], list(n.output), name=n.name))
    return len(negs)


def pretranspose_gemm(model):
    """ORT 1.22's CoreML MLProgram Gemm builder passes a transB=1 weight through as a blob-file
    initializer, but for transB=0 it transposes B itself and emits the copy as hex-float text in the
    MIL program (3.5 GB here, parsed on every session load: 63 s). Transposing at export is exact."""
    g = model.graph
    inits = {t.name: t for t in g.initializer}
    consumers = Counter(i for n in g.node for i in n.input)
    count = 0
    for n in g.node:
        attrs = {a.name: helper.get_attribute_value(a) for a in n.attribute}
        if n.op_type != "Gemm" or attrs.get("transB", 0) or n.input[1] not in inits:
            continue
        b = inits[n.input[1]]
        b_t = numpy_helper.to_array(b).T.copy()
        if consumers[b.name] == 1:
            b.CopyFrom(numpy_helper.from_array(b_t, b.name))
        else:
            g.initializer.append(numpy_helper.from_array(b_t, b.name + "_t"))
            n.input[1] = b.name + "_t"
        attrs["transB"] = 1
        n.CopyFrom(helper.make_node("Gemm", list(n.input), list(n.output), name=n.name, **attrs))
        count += 1
    return count


def preprocess(image_path):
    from PIL import Image
    from torchvision import transforms

    image = Image.open(image_path)
    if image.mode in ("RGBA", "P"):
        image = image.convert("RGBA")
        background = Image.new("RGB", image.size, (255, 255, 255))
        background.paste(image, mask=image.split()[3])
        image = background
    else:
        image = image.convert("RGB")
    transform = transforms.Compose(
        [
            transforms.Resize((INPUT_SIZE, INPUT_SIZE)),
            transforms.ToTensor(),
            transforms.Normalize(mean=[0.5, 0.5, 0.5], std=[0.5, 0.5, 0.5]),
        ]
    )
    return transform(image).unsqueeze(0)


def check(model, onnx_path, table, image_path):
    import onnxruntime as ort

    x = preprocess(image_path)
    with torch.inference_mode():
        ref = model(x)[0].numpy()
    session = ort.InferenceSession(str(onnx_path), providers=["CPUExecutionProvider"])
    (out,) = session.run(None, {"input": x.numpy()})
    out = out[0]
    print(f"check {image_path}: max abs diff {np.abs(ref - out).max():.3e}, argmax {ref.argmax()} vs {out.argmax()}")
    gen = table["gen_tag_count"]
    names = table["names"]
    general = sorted(((p, names[i]) for i, p in enumerate(out[:gen]) if p > 0.3), reverse=True)
    character = sorted(((p, names[gen + i]) for i, p in enumerate(out[gen:]) if p > 0.85), reverse=True)
    print(f"general ({len(general)}):", ", ".join(f"{n} {p:.2f}" for p, n in general))
    print(f"character ({len(character)}):", ", ".join(f"{n} {p:.2f}" for p, n in character))


def main(argv):
    if len(argv) < 2:
        sys.exit(__doc__)
    args = list(argv[1:])
    image = None
    if "--check" in args:
        i = args.index("--check")
        image = args[i + 1]
        del args[i : i + 2]
    snapshot = Path(args[0])
    out_dir = Path(args[1]) if len(args) > 1 else Path("models/tagger/pixai")
    out_dir.mkdir(parents=True, exist_ok=True)

    table = tag_table(snapshot)
    (out_dir / "tags.json").write_text(json.dumps(table, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
    print(f"saved {out_dir / 'tags.json'} ({(out_dir / 'tags.json').stat().st_size / 1e6:.2f} MB)")

    model = build_model(snapshot / "model_v0.9.pth")
    onnx_path = out_dir / "model.onnx"
    export_onnx(model, onnx_path)
    if image:
        check(model, onnx_path, table, image)


if __name__ == "__main__":
    main(sys.argv)
