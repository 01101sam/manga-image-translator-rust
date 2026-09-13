#!/usr/bin/env python3
"""OCR48px E5RT 诊断 CLI。baseline 走公开 Ocr48px.detect；padding/formats 走 e5rt_canvas infer。"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path

MODEL_FILES = ("encoder.onnx", "decoder.onnx", "color_pred.onnx", "alphabet-all-v7.txt")
PAGE_A_NAME = "imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png"
PAGE_B_NAME = "imgs/232264684-5a7bcf8e-707b-4925-86b0-4212382f1680.png"
PAGE_A_TEXTS = ("そうだなあ‥", "ふふっ、", "ふふっ、")
REC_RE = {
    "baseline": re.compile(r"^E5RT_BASELINE (\S+) (.*)$"),
    "canvas": re.compile(r"^E5RT_CANVAS (\S+) (.*)$"),
}
TIME_RSS_RE = re.compile(r"(\d+)\s+maximum resident set size")
TIME_FOOT_RE = re.compile(r"(\d+)\s+peak memory footprint")
E5RT_MARKER = "E5RT encountered"
TEXT_RE = re.compile(r'text="((?:\\.|[^"\\])*)"')
FIELD_RE = re.compile(r"(\w+)=(\S+)")
CAP_RE = re.compile(
    r"number of partitions supported by CoreML: (\d+) number of nodes in the graph: (\d+) number of nodes supported by CoreML: (\d+)"
)
MEM_RE = re.compile(r"memory=\[(\d+), (\d+), (\d+)\]")
MASK_MISMATCH_RE = re.compile(r"mismatches=(\d+)")
RAW_LIST_RE = re.compile(r" raw=(\[.*?\]) pad=")
PAGE_KEYS = (("A", 1), ("B", 1), ("A", 2))
REGIONS_PER_PAGE = 3


def default_workspace() -> Path:
    return Path(__file__).resolve().parent.parent


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def file_identity(path: Path) -> dict:
    if not path.exists():
        return {"path": str(path), "exists": False}
    info = path.stat()
    row = {
        "path": str(path.resolve()),
        "exists": True,
        "size": info.st_size,
        "inode": info.st_ino,
        "nlink": info.st_nlink,
    }
    if path.is_file() and path.name == "encoder.onnx":
        row["sha256"] = sha256_file(path)
    return row


def dir_size(path: Path) -> int:
    if not path.exists():
        return 0
    return sum((Path(r) / f).stat().st_size for r, _d, fs in os.walk(path) for f in fs)


def count_files(path: Path) -> int:
    if not path.exists():
        return 0
    return sum(1 for _r, _d, fs in os.walk(path) for _ in fs)


def cargo_lock_versions(lock: Path, names: tuple[str, ...]) -> dict[str, str | None]:
    found = {name: None for name in names}
    current = None
    if not lock.is_file():
        return found
    for line in lock.read_text(encoding="utf-8").splitlines():
        if line.startswith("name = "):
            current = line.split("=", 1)[1].strip().strip('"')
        elif line.startswith("version = ") and current in found and found[current] is None:
            found[current] = line.split("=", 1)[1].strip().strip('"')
            current = None
    return found


def clang_env() -> dict[str, str]:
    env = os.environ.copy()
    clang = subprocess.check_output(["xcrun", "--find", "clang"], text=True).strip()
    lib = str(Path(clang).parent.parent / "lib")
    env["LIBCLANG_PATH"] = lib
    prev = env.get("DYLD_FALLBACK_LIBRARY_PATH", "")
    env["DYLD_FALLBACK_LIBRARY_PATH"] = lib if not prev else f"{lib}:{prev}"
    return env


def git_snapshot(workspace: Path) -> dict:
    def git(*args: str) -> str:
        proc = subprocess.run(["git", *args], cwd=workspace, text=True, capture_output=True)
        return proc.stdout if proc.returncode == 0 else proc.stderr

    return {
        "head": git("rev-parse", "HEAD").strip(),
        "status_short": git("status", "--short"),
        "output_dir_created_this_run": True,
    }


def classify_e5rt(text: str) -> dict:
    token_count = text.count(E5RT_MARKER)
    if token_count == 0:
        return {"message_count": 0, "token_count": 0, "categories": {}, "messages": []}
    msgs = [(E5RT_MARKER + part).strip() for part in text.split(E5RT_MARKER)[1:]]
    categories: dict[str, int] = {}
    for msg in msgs:
        if "unbounded dimension" in msg:
            key = "unbounded"
        elif "PropagateInputTensorShapes" in msg and "conv" in msg:
            key = "propagate_conv"
        elif "PropagateInputTensorShapes" in msg and "matmul" in msg:
            key = "propagate_matmul"
        elif "PropagateInputTensorShapes" in msg:
            key = "propagate_other"
        elif "_Cast_output_0" in msg:
            key = "_Cast_output_0"
        else:
            key = "unknown"
        categories[key] = categories.get(key, 0) + 1
    return {
        "message_count": len(msgs),
        "token_count": token_count,
        "categories": categories,
        "messages": msgs,
    }


def parse_records(stderr: str, kind: str) -> list[dict]:
    pat = REC_RE[kind]
    return [
        {"kind": m.group(1), "fields": m.group(2)}
        for line in stderr.splitlines()
        if (m := pat.match(line))
    ]


def parse_fields(fields: str) -> dict[str, str]:
    values = {key: value for key, value in FIELD_RE.findall(fields)}
    if text_match := TEXT_RE.search(fields):
        values["text"] = text_match.group(1).replace('\\"', '"').replace("\\\\", "\\")
    return values


def page_regions(records: list[dict], page: str, visit: int) -> list[dict]:
    rows = []
    for rec in records:
        if rec["kind"] != "region":
            continue
        fields = parse_fields(rec["fields"])
        if fields.get("page") == page and fields.get("visit") == str(visit):
            rows.append(fields)
    return sorted(rows, key=lambda r: int(r.get("idx", "-1")))


def accept_page_a(records: list[dict], visit: int, expected: tuple[str, ...] = PAGE_A_TEXTS) -> list[str]:
    errors = []
    regions = page_regions(records, "A", visit)
    seen: dict[str, int] = {}
    for region in regions:
        idx = region.get("idx", "")
        seen[idx] = seen.get(idx, 0) + 1
    dups = [idx for idx, n in seen.items() if n > 1]
    if dups:
        errors.append(f"page=A visit={visit} duplicate_idx={dups}")
    if len(regions) != len(expected):
        errors.append(f"page=A visit={visit} region_count got={len(regions)} want={len(expected)}")
        return errors
    for idx, (region, want) in enumerate(zip(regions, expected)):
        if region.get("idx") != str(idx):
            errors.append(f"page=A visit={visit} idx_gap got={region.get('idx')} want={idx}")
        if region.get("text") != want:
            errors.append(
                f"page=A visit={visit} idx={idx} text got={region.get('text')!r} want={want!r}"
            )
    return errors


def accept_alignment(left: list[dict], right: list[dict], label: str) -> list[str]:
    errors = []
    if len(left) != len(right):
        return [f"{label} count {len(left)}!={len(right)}"]
    for a, b in zip(left, right):
        if a.get("idx") != b.get("idx") or a.get("text") != b.get("text"):
            errors.append(f"{label} idx={a.get('idx')} {a.get('text')!r} != {b.get('text')!r}")
    return errors


def page_visits(records: list[dict]) -> list[tuple[str | None, int | None]]:
    keys = []
    for rec in records:
        if rec["kind"] != "page":
            continue
        fields = parse_fields(rec["fields"])
        visit = fields.get("visit")
        keys.append((fields.get("page"), int(visit) if visit and visit.isdigit() else None))
    return keys


def require_complete_run(run: dict) -> list[str]:
    errors = []
    name = run.get("name", "?")
    rc = run.get("wrapper_returncode")
    if rc is None:
        return [f"{name} missing wrapper_returncode"]
    if rc not in (0, 1):
        errors.append(f"{name} unexpected_exit={rc} (time wrapper; not claimed as SIGABRT)")
    recs = run.get("records") or []
    if not recs:
        errors.append(f"{name} missing_records")
    keys = page_visits(recs)
    for want in PAGE_KEYS:
        n = keys.count(want)
        if n != 1:
            errors.append(f"{name} page={want[0]} visit={want[1]} count={n} want=1")
    for page, visit in PAGE_KEYS:
        regions = page_regions(recs, page, visit)
        if len(regions) != REGIONS_PER_PAGE:
            errors.append(
                f"{name} page={page} visit={visit} region_count={len(regions)} want={REGIONS_PER_PAGE}"
            )
            continue
        for idx, region in enumerate(regions):
            if region.get("idx") != str(idx):
                errors.append(
                    f"{name} page={page} visit={visit} idx_gap got={region.get('idx')} want={idx}"
                )
    return errors


def require_run_success(run: dict) -> list[str]:
    errors = []
    name = run.get("name", "?")
    if run.get("wrapper_returncode") != 0:
        errors.append(f"{name} wrapper_returncode={run.get('wrapper_returncode')} want=0")
    recs = run.get("records") or []
    if not any(r["kind"] == "accept" for r in recs):
        errors.append(f"{name} missing_accept")
    fails = [r["fields"] for r in recs if r["kind"] == "fail"]
    if fails:
        errors.append(f"{name} unexpected_fail {fails[0]}")
    return errors


def e5rt_counts(run: dict) -> dict:
    stdout = (run.get("e5rt_stdout") or {}).get("message_count")
    stderr = (run.get("e5rt_stderr") or {}).get("message_count")
    return {"stdout": stdout, "stderr": stderr}


def judge_e5rt_repair(runs: dict, coreml_keys: list[str]) -> dict:
    counts = {k: e5rt_counts(runs[k]) for k in runs}
    coreml = [counts[k] for k in coreml_keys if k in runs]
    if not coreml or any(c["stdout"] is None or c["stderr"] is None for c in coreml):
        return {"e5rt_repair": "undetermined", "why": "missing_coreml_e5rt_count", "counts": counts}
    if any((c["stdout"] or 0) > 0 or (c["stderr"] or 0) > 0 for c in coreml):
        return {"e5rt_repair": "not_fixed", "why": f"coreml_e5rt={coreml}", "counts": counts}
    return {
        "e5rt_repair": "not_proven",
        "why": "stdout_and_stderr_zero_without_repair_claim",
        "counts": counts,
    }


def prepare_output(path: Path | None) -> Path:
    if path is None:
        return Path(tempfile.mkdtemp(prefix="ocr48px-e5rt-"))
    out = path.expanduser().resolve()
    if out.exists():
        leftover = list(out.iterdir())
        if leftover:
            raise SystemExit(f"refuse overwrite: {out} contains {[p.name for p in leftover]}")
    else:
        out.mkdir(parents=True)
    return out


def copy_models(src: Path, dst: Path) -> dict:
    dst.mkdir(parents=True, exist_ok=True)
    copied = {}
    for name in MODEL_FILES:
        source, target = src / name, dst / name
        if target.exists():
            raise SystemExit(f"refuse overwrite model copy {target}")
        shutil.copy2(source, target)
        os.chmod(target, stat.S_IRUSR | stat.S_IWUSR | stat.S_IRGRP | stat.S_IROTH)
        s, t = source.stat(), target.stat()
        copied[name] = {
            "src_inode": s.st_ino,
            "dst_inode": t.st_ino,
            "same_inode": s.st_ino == t.st_ino,
            "nlink_dst": t.st_nlink,
        }
    return copied


def cache_snap(cache: Path, shared: Path) -> dict:
    return {
        "model_cache_dir_empty": (not cache.exists()) or count_files(cache) == 0,
        "model_cache_files": count_files(cache),
        "model_cache_bytes": dir_size(cache),
        "shared_cache_files": count_files(shared),
        "shared_cache_bytes": dir_size(shared),
        "cold_means": "ModelCacheDirectory empty only; system CoreML caches unknown",
    }


def build_example(workspace: Path, env: dict[str, str], name: str) -> Path:
    subprocess.run(
        ["cargo", "build", "-p", "ocr-48px", "--example", name, "--release"],
        cwd=workspace,
        env=env,
        check=True,
    )
    return workspace / f"target/release/examples/{name}"


def run_child(
    name: str,
    binary: Path,
    work: Path,
    env: dict[str, str],
    args: list[str],
    out: Path,
    shared: Path,
    rec_kind: str,
    extra_env: dict[str, str] | None = None,
) -> dict:
    cache = work / "models" / "cache"
    before = cache_snap(cache, shared)
    child = env.copy()
    child.pop("CARGO_MANIFEST_DIR", None)
    if extra_env:
        child.update(extra_env)
    stdout_p = out / "runs" / f"{name}.stdout.txt"
    stderr_p = out / "runs" / f"{name}.stderr.txt"
    cmd = ["/usr/bin/time", "-l", str(binary), *args]
    t0 = time.time()
    proc = subprocess.run(cmd, cwd=work, env=child, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    elapsed = time.time() - t0
    stdout_p.write_bytes(proc.stdout)
    stderr_p.write_bytes(proc.stderr)
    err = proc.stderr.decode("utf-8", "replace")
    out_txt = proc.stdout.decode("utf-8", "replace")
    recs = parse_records(err, rec_kind)
    after = cache_snap(cache, shared)
    abort_text = "Abort trap" in err or "SIGABRT" in err
    return {
        "name": name,
        "wrapper": "/usr/bin/time -l",
        "wrapper_returncode": proc.returncode,
        "signal_text_seen": abort_text,
        "elapsed_s": elapsed,
        "cmd": cmd,
        "cwd": str(work),
        "stdout_path": str(stdout_p),
        "stderr_path": str(stderr_p),
        "time_max_rss_bytes": int(m.group(1)) if (m := TIME_RSS_RE.search(err)) else None,
        "time_peak_memory_footprint_bytes": int(m.group(1)) if (m := TIME_FOOT_RE.search(err)) else None,
        "e5rt_stdout": {k: v for k, v in classify_e5rt(out_txt).items() if k != "messages"},
        "e5rt_stderr": {k: v for k, v in classify_e5rt(err).items() if k != "messages"},
        "records": recs,
        "cache_before": before,
        "cache_after": after,
        "shared_bytes_delta": after["shared_cache_bytes"] - before["shared_cache_bytes"],
        "accept": any(r["kind"] == "accept" for r in recs),
        "fails": [r["fields"] for r in recs if r["kind"] == "fail"],
    }


def write_report(out: Path, title: str, bullets: list[str]) -> None:
    lines = [f"# {title}", "", f"输出目录 `{out}`。本轮新建。旧 /tmp 证据未覆盖。", ""]
    lines.extend(f"- {b}" for b in bullets)
    lines += [
        "",
        "冷启动只表示 ModelCacheDirectory 为空，系统 CoreML 缓存未知。",
        "E5RT>0 不是修复。CPU 上 E5RT=0 也不是修复。",
        f"复跑：`python3 scripts/ocr48px_e5rt.py <mode> --output-dir <empty-or-new>`。",
        "",
    ]
    (out / "report.md").write_text("\n".join(lines), encoding="utf-8")


def oracle_a(run: dict) -> list[str]:
    recs = run.get("records") or []
    errors = require_complete_run(run)
    errors.extend(require_run_success(run))
    errors.extend(accept_page_a(recs, 1))
    errors.extend(accept_page_a(recs, 2))
    errors.extend(
        accept_alignment(page_regions(recs, "A", 1), page_regions(recs, "A", 2), f"{run['name']} A-repeat")
    )
    return errors


def expected_p0_fail(msg: str) -> bool:
    return "pad_broke_text" in msg or (
        'got="そうだなあ…"' in msg and 'want="そうだなあ‥"' in msg
    )


def page_raw_texts(fields: str) -> list[str] | None:
    match = RAW_LIST_RE.search(fields)
    if not match:
        return None
    try:
        parsed = json.loads(match.group(1))
    except json.JSONDecodeError:
        return None
    if not isinstance(parsed, list) or not all(isinstance(x, str) for x in parsed):
        return None
    return parsed


def self_test() -> None:
    empty = classify_e5rt("")
    assert empty["message_count"] == 0
    assert classify_e5rt("noise before marker\n")["message_count"] == 0
    two = (
        "junk "
        f"{E5RT_MARKER} unbounded dimension which is not supported.."
        f"{E5RT_MARKER} totally novel diagnostic."
    )
    got = classify_e5rt(two)
    assert got["token_count"] == 2 and got["categories"]["unbounded"] == 1
    assert got["categories"]["unknown"] == 1
    assert "junk" not in got["messages"][0]

    good = [
        {"kind": "region", "fields": f'page=A visit=1 idx=0 role=oracle text="{PAGE_A_TEXTS[0]}"'},
        {"kind": "region", "fields": f'page=A visit=1 idx=1 role=oracle text="{PAGE_A_TEXTS[1]}"'},
        {"kind": "region", "fields": f'page=A visit=1 idx=2 role=oracle text="{PAGE_A_TEXTS[2]}"'},
    ]
    assert accept_page_a(good, 1) == []
    missing = accept_page_a(good[:2], 1)
    assert missing and "region_count" in missing[0]
    dup = good + [good[0]]
    assert any("duplicate_idx" in e for e in accept_page_a(dup, 1))
    wrong = json.loads(json.dumps(good))
    wrong[0]["fields"] = 'page=A visit=1 idx=0 role=oracle text="WRONG"'
    assert any("WRONG" in e for e in accept_page_a(wrong, 1))
    v2 = json.loads(json.dumps(good))
    for rec in v2:
        rec["fields"] = rec["fields"].replace("visit=1", "visit=2")
    later_fail = json.loads(json.dumps(v2))
    later_fail[0]["fields"] = later_fail[0]["fields"].replace(PAGE_A_TEXTS[0], "WRONG")
    both = good + later_fail
    assert accept_page_a(both, 1) == []
    assert accept_page_a(both, 2)

    b1 = [
        {"kind": "region", "fields": f'page=B visit=1 idx={i} role=diff text="B{i}"'}
        for i in range(3)
    ]
    complete = {
        "name": "x",
        "wrapper_returncode": 0,
        "records": [
            {"kind": "page", "fields": "page=A visit=1"},
            {"kind": "page", "fields": "page=B visit=1"},
            {"kind": "page", "fields": "page=A visit=2"},
            *good,
            *b1,
            *v2,
            {"kind": "accept", "fields": "pass"},
        ],
    }
    assert require_complete_run(complete) == []
    assert require_run_success(complete) == []
    assert oracle_a(complete) == []
    no_shape = {"name": "y", "wrapper_returncode": 0, "records": [{"kind": "accept", "fields": "pass"}]}
    assert require_complete_run(no_shape)
    later_run = {
        "name": "warm",
        "wrapper_returncode": 1,
        "records": [{"kind": "fail", "fields": "boom"}, {"kind": "page", "fields": "x"}] * 3,
    }
    assert require_complete_run(later_run)

    exit1_gold = {
        "name": "fake_pass",
        "wrapper_returncode": 1,
        "records": [
            {"kind": "page", "fields": "page=A visit=1"},
            {"kind": "page", "fields": "page=B visit=1"},
            {"kind": "page", "fields": "page=A visit=2"},
            *good,
            *b1,
            *v2,
            {"kind": "fail", "fields": "wrong_result_count"},
        ],
    }
    assert require_complete_run(exit1_gold) == []
    assert oracle_a(exit1_gold)

    dup_page = json.loads(json.dumps(complete))
    dup_page["records"].append({"kind": "page", "fields": "page=A visit=1"})
    assert any("count=2" in e for e in require_complete_run(dup_page))
    empty_b = json.loads(json.dumps(complete))
    empty_b["records"] = [r for r in empty_b["records"] if "page=B" not in r.get("fields", "")]
    assert any("page=B visit=1" in e for e in require_complete_run(empty_b))

    pad_ok = {
        "name": "pad",
        "wrapper_returncode": 1,
        "records": [
            {"kind": "page", "fields": 'page=A visit=1 raw=["そうだなあ‥", "ふふっ、", "ふふっ、"] pad=["そうだなあ…", "ふふっ、", "ふふっ、"]'},
            {"kind": "page", "fields": "page=B visit=1"},
            {"kind": "page", "fields": 'page=A visit=2 raw=["そうだなあ‥", "ふふっ、", "ふふっ、"] pad=["そうだなあ…", "ふふっ、", "ふふっ、"]'},
            *good,
            *b1,
            *v2,
            {"kind": "enc", "fields": "memory=[2, 34, 320] input_mask=[2, 34]"},
            {"kind": "mask", "fields": "mismatches=0"},
            {"kind": "fail", "fields": 'page=A visit=1 idx=0 text got="そうだなあ…" want="そうだなあ‥"'},
            {"kind": "fail", "fields": 'page=A visit=1 pad_broke_text raw=["そうだなあ‥"] pad=["そうだなあ…"]'},
        ],
    }
    assert padding_extra_errors(pad_ok) == []
    missing_enc = json.loads(json.dumps(pad_ok))
    missing_enc["records"] = [r for r in missing_enc["records"] if r["kind"] != "enc"]
    assert any("missing_enc" in e for e in padding_extra_errors(missing_enc))
    bad_mem = json.loads(json.dumps(pad_ok))
    for rec in bad_mem["records"]:
        if rec["kind"] == "enc":
            rec["fields"] = "memory=[2, 34] input_mask=[2, 34]"
    assert any("unparsed_memory" in e for e in padding_extra_errors(bad_mem))
    mismatch = json.loads(json.dumps(pad_ok))
    for rec in mismatch["records"]:
        if rec["kind"] == "mask":
            rec["fields"] = "mismatches=3"
    assert any("mismatches=3" in e for e in padding_extra_errors(mismatch))
    missing_mask = json.loads(json.dumps(pad_ok))
    missing_mask["records"] = [r for r in missing_mask["records"] if r["kind"] != "mask"]
    assert any("missing_mask" in e for e in padding_extra_errors(missing_mask))
    missing_shape = json.loads(json.dumps(pad_ok))
    for rec in missing_shape["records"]:
        if rec["kind"] == "enc":
            rec["fields"] = "input_mask=[2, 34]"
    assert any("unparsed_memory" in e for e in padding_extra_errors(missing_shape))
    pad_gate = padding_gates(pad_ok)
    assert pad_gate["text"] == "rejected" and pad_gate["measurement"] == "complete" and pad_gate["valid"]
    assert pad_gate["ok"]
    mismatch_gate = padding_gates(mismatch)
    assert mismatch_gate["text"] == "fail" and mismatch_gate["measurement"] == "complete"
    assert not mismatch_gate["valid"] and not mismatch_gate["ok"]
    missing_enc_gate = padding_gates(missing_enc)
    assert missing_enc_gate["text"] == "fail" and not missing_enc_gate["ok"]

    nn_b = json.loads(json.dumps(complete))
    for rec in nn_b["records"]:
        if rec["kind"] == "region" and "page=B visit=1 idx=1" in rec["fields"]:
            rec["fields"] = 'page=B visit=1 idx=1 role=diff text="CHANGED"'
    b_diffs = page_b_diff_vs_cpu({"cpu_cold": complete, "nn_cold": nn_b})
    assert b_diffs["cpu_cold"] == []
    assert b_diffs["nn_cold"]
    assert oracle_a(nn_b) == []

    mlp = {
        "mlp_cold": {"e5rt_stdout": {"message_count": 33}, "e5rt_stderr": {"message_count": 0}},
        "mlp_warm": {"e5rt_stdout": {"message_count": 33}, "e5rt_stderr": {"message_count": 0}},
        "cpu_cold": {"e5rt_stdout": {"message_count": 0}, "e5rt_stderr": {"message_count": 0}},
    }
    judged = judge_e5rt_repair(mlp, ["mlp_cold", "mlp_warm"])
    assert judged["e5rt_repair"] == "not_fixed"
    stderr_only = {
        "mlp_cold": {"e5rt_stdout": {"message_count": 0}, "e5rt_stderr": {"message_count": 33}},
    }
    assert judge_e5rt_repair(stderr_only, ["mlp_cold"])["e5rt_repair"] == "not_fixed"
    zeros = {
        "nn_cold": {"e5rt_stdout": {"message_count": 0}, "e5rt_stderr": {"message_count": 0}},
    }
    assert judge_e5rt_repair(zeros, ["nn_cold"])["e5rt_repair"] == "not_proven"
    self_test_old_logs()


def cmd_baseline(args: argparse.Namespace) -> int:
    self_test()
    if args.self_test:
        print(json.dumps({"self_test": "pass"}))
        return 0
    ws = (args.workspace or default_workspace()).resolve()
    models = (args.models_dir or (ws / "models/ocr/48px")).resolve()
    shared = ws / "models/cache"
    out = prepare_output(args.output_dir)
    (out / "runs").mkdir()
    work = out / "workdir"
    (work / "models/cache").mkdir(parents=True)
    env = clang_env()
    copied = copy_models(models, work / "models/ocr/48px")
    binary = build_example(ws, env, "e5rt_baseline")
    page_a, page_b = ws / PAGE_A_NAME, ws / PAGE_B_NAME
    cold = run_child("cold_aba", binary, work, env, [str(page_a), str(page_b)], out, shared, "baseline")
    warm = run_child("warm_aba", binary, work, env, [str(page_a), str(page_b)], out, shared, "baseline")
    wrong = run_child(
        "wrong_expect",
        binary,
        work,
        env,
        [str(page_a), str(page_b)],
        out,
        shared,
        "baseline",
        extra_env={"E5RT_PAGE_A_EXPECT": "WRONG|ふふっ、|ふふっ、"},
    )
    errors = oracle_a(cold) + oracle_a(warm)
    errors += accept_alignment(
        page_regions(cold["records"], "B", 1),
        page_regions(warm["records"], "B", 1),
        "B-diff",
    )
    rust_wrong = wrong["wrapper_returncode"] != 0 and any("WRONG" in f for f in wrong["fails"])
    e5rt = judge_e5rt_repair({"cold": cold, "warm": warm}, ["cold", "warm"])
    results = {
        "mode": "baseline",
        "surface": "ocr_48px::Ocr48px.detect",
        "output_dir": str(out),
        "git": git_snapshot(ws),
        "models": {"src": file_identity(models / "encoder.onnx"), "copied": copied},
        "crate_versions": cargo_lock_versions(ws / "Cargo.lock", ("ort", "ort-sys")),
        "e5rt_repair": e5rt,
        "text": "pass" if not errors else "fail",
        "errors": errors,
        "wrong_expect_rejected": rust_wrong,
        "runs": {k: {kk: vv for kk, vv in r.items() if kk != "records"} | {"records": r["records"]} for k, r in (("cold_aba", cold), ("warm_aba", warm), ("wrong_expect", wrong))},
    }
    (out / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    write_report(
        out,
        "OCR48px E5RT 基线（公开 detect）",
        [
            f"文字 {results['text']}",
            f"E5RT 修复 {e5rt['e5rt_repair']} ({e5rt['why']})",
            f"冷 E5RT stdout/stderr {e5rt_counts(cold)} 暖 {e5rt_counts(warm)}",
            f"冷 ModelCacheDirectory 空={cold['cache_before']['model_cache_dir_empty']}",
            "页时间是 detect 整页墙钟，不是 e5rt_canvas infer_ms。",
            f"故意错误期望拒绝={rust_wrong}",
        ],
    )
    print(json.dumps({"out": str(out), "text": results["text"], "e5rt_repair": e5rt}, ensure_ascii=False))
    if errors or not rust_wrong:
        return 1
    return 0


def padding_extra_errors(run: dict) -> list[str]:
    errors = []
    recs = run.get("records") or []
    encs = [r for r in recs if r["kind"] == "enc"]
    masks = [r for r in recs if r["kind"] == "mask"]
    if not encs:
        errors.append("missing_enc")
    if not masks:
        errors.append("missing_mask")
    for rec in encs:
        match = MEM_RE.search(rec["fields"])
        if not match:
            errors.append(f"unparsed_memory {rec['fields']}")
        elif match.group(3) != "320":
            errors.append(f"memory last dim {match.group(0)}")
    for rec in masks:
        match = MASK_MISMATCH_RE.search(rec["fields"])
        if not match:
            errors.append(f"unparsed_mask {rec['fields']}")
        elif match.group(1) != "0":
            errors.append(f"mask mismatches={match.group(1)}")
    for rec in recs:
        if rec["kind"] != "page" or "page=A" not in rec["fields"]:
            continue
        raw = page_raw_texts(rec["fields"])
        if raw is None:
            errors.append(f"unparsed_raw {rec['fields']}")
        elif tuple(raw) != PAGE_A_TEXTS:
            errors.append(f"raw_gold {raw!r}")
    return errors


def padding_gates(run: dict) -> dict:
    recs = run.get("records") or []
    fails = run.get("fails")
    if fails is None:
        fails = [r["fields"] for r in recs if r["kind"] == "fail"]
    obs = require_complete_run(run)
    extra = padding_extra_errors(run)
    unexpected = [f for f in fails if not expected_p0_fail(f)]
    extra.extend(f"unexpected_fail {f}" for f in unexpected)
    rejected = any("pad_broke_text" in f for f in fails)
    if not run.get("records"):
        text_state = "undetermined"
    elif extra:
        text_state = "fail"
    elif rejected:
        text_state = "rejected"
    elif run.get("accept") and not obs:
        text_state = "pass"
    else:
        text_state = "fail"
    measurement = "complete" if not obs else "incomplete"
    valid = not extra
    return {
        "errors": obs + extra,
        "text": text_state,
        "measurement": measurement,
        "valid": valid,
        "ok": text_state == "rejected" and measurement == "complete" and valid,
    }


def page_b_diff_vs_cpu(runs: dict) -> dict[str, list[str]]:
    cpu = page_regions((runs.get("cpu_cold") or {}).get("records") or [], "B", 1)
    diffs: dict[str, list[str]] = {}
    for name, run in runs.items():
        recs = run.get("records") or []
        diffs[name] = accept_alignment(cpu, page_regions(recs, "B", 1), f"{name} B-vs-cpu_cold")
    return diffs


def self_test_old_logs() -> None:
    path = Path("/tmp/image-translator-e5rt-20260914-0641/padding-control/runs/cpu_both.stderr.txt")
    if not path.is_file():
        return
    recs = parse_records(path.read_text(encoding="utf-8", errors="replace"), "canvas")
    run = {
        "name": "old_pad",
        "wrapper_returncode": 1,
        "records": recs,
        "fails": [r["fields"] for r in recs if r["kind"] == "fail"],
        "accept": False,
    }
    assert require_complete_run(run) == []
    assert padding_extra_errors(run) == []
    dropped = {
        **run,
        "records": [r for r in recs if r["kind"] not in ("enc", "mask")],
    }
    extra = padding_extra_errors(dropped)
    assert any("missing_enc" in e for e in extra)
    assert any("missing_mask" in e for e in extra)
    dup = {**run, "records": recs + [{"kind": "page", "fields": "page=A visit=1"}]}
    assert any("count=2" in e for e in require_complete_run(dup))
    old_gate = padding_gates(run)
    assert old_gate["ok"] and old_gate["text"] == "rejected"
    dropped_gate = padding_gates(dropped)
    assert dropped_gate["text"] == "fail" and not dropped_gate["valid"] and not dropped_gate["ok"]
    mismatch_old = {
        **run,
        "records": [
            {**r, "fields": r["fields"].replace("mismatches=0", "mismatches=3")}
            if r["kind"] == "mask"
            else r
            for r in recs
        ],
    }
    mismatch_gate = padding_gates(mismatch_old)
    assert not mismatch_gate["ok"] and not mismatch_gate["valid"]
    empty_b = {
        **run,
        "records": [r for r in recs if not (r["kind"] in ("page", "region") and "page=B" in r.get("fields", ""))],
    }
    empty_b_gate = padding_gates(empty_b)
    assert empty_b_gate["measurement"] == "incomplete" and not empty_b_gate["ok"]
    missing_shape = {
        **run,
        "records": [
            {**r, "fields": r["fields"].replace(MEM_RE.search(r["fields"]).group(0), "memory=[2, 34]")}
            if r["kind"] == "enc" and MEM_RE.search(r["fields"])
            else r
            for r in recs
        ],
    }
    shape_gate = padding_gates(missing_shape)
    assert any("unparsed_memory" in e for e in shape_gate["errors"])
    assert not shape_gate["ok"]
    fmt_root = Path("/tmp/image-translator-e5rt-20260914-0641/finalization/reruns/formats/runs")
    cpu_fmt = fmt_root / "cpu_cold.stderr.txt"
    mlp_fmt = fmt_root / "mlp_cold.stderr.txt"
    if cpu_fmt.is_file() and mlp_fmt.is_file():
        cpu_run = {"records": parse_records(cpu_fmt.read_text(encoding="utf-8", errors="replace"), "canvas")}
        mlp_run = {"records": parse_records(mlp_fmt.read_text(encoding="utf-8", errors="replace"), "canvas")}
        same = page_b_diff_vs_cpu({"cpu_cold": cpu_run, "mlp_cold": mlp_run})
        assert same["cpu_cold"] == []
        mutated = json.loads(json.dumps(mlp_run))
        for rec in mutated["records"]:
            if rec["kind"] == "region" and "page=B visit=1 idx=1" in rec["fields"]:
                rec["fields"] = rec["fields"].replace('text="I"', 'text="CHANGED"')
        changed = page_b_diff_vs_cpu({"cpu_cold": cpu_run, "mlp_cold": mutated})
        assert changed["mlp_cold"]


def cmd_padding(args: argparse.Namespace) -> int:
    self_test()
    if args.self_test:
        print(json.dumps({"self_test": "pass"}))
        return 0
    ws = (args.workspace or default_workspace()).resolve()
    models = (args.models_dir or (ws / "models/ocr/48px")).resolve()
    shared = ws / "models/cache"
    out = prepare_output(args.output_dir)
    (out / "runs").mkdir()
    env = clang_env()
    work = out / "workdir" / "cpu"
    (work / "models/cache").mkdir(parents=True)
    copied = copy_models(models, work / "models/ocr/48px")
    binary = build_example(ws, env, "e5rt_canvas")
    page_a, page_b = str(ws / PAGE_A_NAME), str(ws / PAGE_B_NAME)
    run = run_child("cpu_both", binary, work, env, ["cpu", "both", page_a, page_b], out, shared, "canvas")
    gate = padding_gates(run)
    recs = run["records"]
    raw_oracle = [r["fields"] for r in recs if r["kind"] == "page" and "page=A" in r["fields"]]
    results = {
        "mode": "padding",
        "surface": "e5rt_canvas infer after prepare; not Ocr48px.detect page benchmark",
        "output_dir": str(out),
        "git": git_snapshot(ws),
        "models": {"src": file_identity(models / "encoder.onnx"), "copied": copied},
        "measurement": gate["measurement"],
        "valid": gate["valid"],
        "text": gate["text"],
        "e5rt_repair": "not_applicable_cpu_only",
        "errors": gate["errors"],
        "fails": run["fails"],
        "page_a": raw_oracle,
        "runs": {"cpu_both": run},
    }
    (out / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    write_report(
        out,
        "OCR48px P0 padding 对照（诊断 infer）",
        [
            f"文字 {gate['text']}",
            f"测量 {gate['measurement']} valid={gate['valid']}",
            "已知反例：N2W283 二次垫把 そうだなあ‥ 变成 そうだなあ…。",
            "pad_broke_text 是预期否定；mask/shape/原图 gold 错误不能被它遮住。",
            "memory 契约 [N,seq,320]。不是生产 detect 基准。",
        ],
    )
    print(
        json.dumps(
            {
                "out": str(out),
                "text": gate["text"],
                "measurement": gate["measurement"],
                "valid": gate["valid"],
            },
            ensure_ascii=False,
        )
    )
    return 0 if gate["ok"] else 1


def cmd_formats(args: argparse.Namespace) -> int:
    self_test()
    if args.self_test:
        print(json.dumps({"self_test": "pass"}))
        return 0
    ws = (args.workspace or default_workspace()).resolve()
    models = (args.models_dir or (ws / "models/ocr/48px")).resolve()
    shared = ws / "models/cache"
    out = prepare_output(args.output_dir)
    (out / "runs").mkdir()
    env = clang_env()
    binary = build_example(ws, env, "e5rt_canvas")
    page_a, page_b = str(ws / PAGE_A_NAME), str(ws / PAGE_B_NAME)
    kinds = (("cpu", "cpu"), ("nn", "coreml-nn"), ("mlp", "coreml-mlp"))
    copies = {}
    runs = {}
    for short, cli in kinds:
        work = out / "workdir" / short
        copies[short] = copy_models(models, work / "models/ocr/48px")
        (work / "models/cache").mkdir(parents=True)
        for phase in ("cold", "warm"):
            name = f"{short}_{phase}"
            print(f"running {name}", flush=True)
            runs[name] = run_child(name, binary, work, env, [cli, "raw", page_a, page_b], out, shared, "canvas")
    errors = []
    texts = {}
    for name, run in runs.items():
        err = oracle_a(run)
        errors.extend(err)
        texts[name] = "pass" if not err and run["accept"] else "fail"
    caps = {}
    for name, run in runs.items():
        found = []
        for rec in run["records"]:
            if rec["kind"] != "ort":
                continue
            if m := CAP_RE.search(rec["fields"]):
                found.append({"partitions": int(m.group(1)), "nodes": int(m.group(2)), "supported": int(m.group(3))})
        caps[name] = found[0] if found else None
    repair = judge_e5rt_repair(runs, ["mlp_cold", "mlp_warm", "nn_cold", "nn_warm"])
    pages = {
        name: [r["fields"] for r in run["records"] if r["kind"] == "page"]
        for name, run in runs.items()
    }
    b_diffs = page_b_diff_vs_cpu(runs)
    e5rt_io = {name: e5rt_counts(run) for name, run in runs.items()}
    results = {
        "mode": "formats",
        "surface": "e5rt_canvas infer; not Ocr48px.detect and not baseline 3.27s page clock",
        "output_dir": str(out),
        "git": git_snapshot(ws),
        "models": {"src": file_identity(models / "encoder.onnx"), "copied": copies},
        "measurement": "complete"
        if all(not require_complete_run(r) for r in runs.values())
        else "incomplete",
        "text": texts,
        "e5rt_repair": repair,
        "e5rt_stdout_stderr": e5rt_io,
        "performance": "measured_only_unjudged",
        "capability": caps,
        "pages": pages,
        "page_b_diff_vs_cpu": b_diffs,
        "page_b_role": "cross_arm_diff_not_gold",
        "errors": errors,
        "runs": {k: {kk: vv for kk, vv in r.items() if kk != "records"} | {"record_count": len(r["records"])} for k, r in runs.items()},
    }
    (out / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    write_report(
        out,
        "OCR48px encoder 格式对照（诊断 infer）",
        [
            f"文字 {texts}",
            f"E5RT 修复 {repair['e5rt_repair']} ({repair['why']})",
            f"E5RT stdout/stderr {e5rt_io}",
            f"页 B 对 CPU 差分 { {k: (v or 'same') for k, v in b_diffs.items()} }",
            "页 B 只做跨臂差分，不是金标准，不报准确率。",
            f"GetCapability {caps}",
            "本轮 infer 先于诊断 encoder。历史 131–141ms 轮次顺序不同，两轮不可直接互比。",
            "性能只记录，不做自动放行。",
        ],
    )
    print(json.dumps({"out": str(out), "text": texts, "e5rt_repair": repair}, ensure_ascii=False))
    if any(v != "pass" for v in texts.values()):
        return 1
    return 0 if repair["e5rt_repair"] != "undetermined" else 2


def main() -> int:
    parser = argparse.ArgumentParser(description="OCR48px E5RT diagnostics")
    parser.add_argument("mode", choices=("baseline", "padding", "formats", "self-test"))
    parser.add_argument("--workspace", type=Path, default=None)
    parser.add_argument("--output-dir", type=Path, default=None)
    parser.add_argument("--models-dir", type=Path, default=None)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.mode == "self-test" or args.self_test:
        self_test()
        print(json.dumps({"self_test": "pass"}))
        return 0
    if args.mode == "baseline":
        return cmd_baseline(args)
    if args.mode == "padding":
        return cmd_padding(args)
    return cmd_formats(args)


if __name__ == "__main__":
    raise SystemExit(main())
