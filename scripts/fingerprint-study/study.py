#!/usr/bin/env python3
"""相似图片指纹的阈值实测。

构造同一张图的各种变体 (缩放 0.25x 到 2x、JPEG 重压缩、两者叠加) 和不同的页, 用 App 里的 ImagePrep.inspect (连同 ImageFingerprint.swift 一起编译) 算指纹,
统计「同图」和「异图」的汉明距离分布, 给出能把两者分开的阈值。

用法: scripts/fingerprint-study/study.py [工作目录] [额外的图片目录 ...]
需要: macOS 的 swiftc、ImageMagick (magick)、sips。额外的目录里的图当作「异图」的基图加入。
"""
import itertools, json, os, random, subprocess, sys, hashlib, statistics
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
WORK = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/fingerprint-study")
EXTRA = [Path(p) for p in sys.argv[2:]]
ASPECT_TOLERANCE = 0.03

def run(*args):
    subprocess.run([str(a) for a in args], check=True, capture_output=True)

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

WORK.mkdir(parents=True, exist_ok=True)
bases = {}   # 名字 -> 路径, 彼此是不同的图
skip_same = {}

def add_base(name, path):
    digest = sha(path)
    if digest in skip_same:
        print("  (相同字节, 不重复加入)", name, "==", skip_same[digest])
        return
    skip_same[digest] = name
    bases[name] = Path(path)

real = [
    ("real-sample-a", REPO / "crates/runtimes/simple-runtime/in/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png"),
    ("real-sample-b", REPO / "imgs/232264684-5a7bcf8e-707b-4925-86b0-4212382f1680.png"),
    ("real-sample-c", REPO / "imgs/232265339-514c843a-0541-4a24-b3bc-1efa6915f757.png"),
    ("real-s12-small1", Path("/tmp/s12-work/img/small1.png")),
    ("real-s12-small2", Path("/tmp/s12-work/img/small2.png")),
]
for name, path in real:
    if path.exists():
        add_base(name, path)

strip = REPO / "imgs/01_1-optimized.png"
if strip.exists():
    # 一条长图切成相邻的页: 同一本书里版式相近的不同页
    for i in range(0, 13):
        out = WORK / f"strip-{i:02d}.png"
        run("magick", strip, "-crop", f"800x1100+0+{i * 1100}", "+repage", out)
        if int(subprocess.run(["magick", "identify", "-format", "%h", out], capture_output=True, text=True).stdout) >= 1000:
            add_base(f"real-strip-{i:02d}", out)

for p in sorted(Path("/tmp/nsh-ui-work/imgs").glob("page*.png")):
    add_base("synthetic-" + p.stem, p)
for directory in EXTRA:
    for p in sorted(directory.glob("*")):
        if p.suffix.lower() in (".png", ".jpg", ".jpeg"):
            add_base("extra-" + p.stem, p)

# 近空白页: 留白加纸的颗粒、页码和几行字, 彼此只差几个小记号 (平坦区域里的差分位最不稳, 是最难分的一类)
rng = random.Random(20261116)
blank = {}
for i in range(30):
    out = WORK / f"blank-{i:02d}.png"
    args = ["magick", "-size", "1000x1400", "xc:white", "-seed", str(100 + i), "+noise", "Gaussian", "-attenuate", "0.03", "-colorspace", "Gray", "-fill", "black"]
    for _ in range(rng.randint(1, 4)):
        x, y = rng.randint(80, 880), rng.randint(80, 1280)
        args += ["-draw", f"rectangle {x},{y} {x + rng.randint(20, 90)},{y + rng.randint(4, 10)}"]
    run(*args, out)
    blank[f"blank-{i:02d}"] = out

variants = []   # (基图名, 变体名, 路径)
def make(base_name, path, groups):
    stem = WORK / base_name
    img = str(path)
    for pct in (25, 50, 75, 150, 200):
        out = Path(f"{stem}-s{pct}.png")
        run("magick", img, "-filter", "Lanczos", "-resize", f"{pct}%", out)
        variants.append((base_name, f"scale-{pct}", out, groups))
    for q in (90, 70, 50, 30):
        out = Path(f"{stem}-q{q}.jpg")
        run("magick", img, "-quality", str(q), out)
        variants.append((base_name, f"jpeg-q{q}", out, groups))
    out = Path(f"{stem}-s50q50.jpg")
    run("magick", img, "-filter", "Lanczos", "-resize", "50%", "-quality", "50", out)
    variants.append((base_name, "scale-50+jpeg-q50", out, groups))
    out = Path(f"{stem}-sips50.png")
    w, h = [int(v) for v in subprocess.run(["magick", "identify", "-format", "%w %h", img], capture_output=True, text=True).stdout.split()]
    run("sips", "-z", str(max(1, h // 2)), str(max(1, w // 2)), img, "--out", out)
    variants.append((base_name, "scale-50-sips", out, groups))

for name, path in bases.items():
    make(name, path, "page")
for name, path in blank.items():
    make(name, path, "blank")

paths = [str(p) for p in bases.values()] + [str(p) for p in blank.values()] + [str(v[2]) for v in variants]
listing = WORK / "paths.txt"
listing.write_text("\n".join(paths) + "\n")
binary = WORK / "fingerprint-study"
run("swiftc", "-O", REPO / "app/ImageTranslator/Host/ImageFingerprint.swift", REPO / "app/ImageTranslator/Host/HostImages.swift", HERE / "main.swift", "-o", binary)
out = subprocess.run([str(binary), str(listing)], check=True, capture_output=True, text=True).stdout
info = {}
APP_MARGIN = 0
for line in out.splitlines():
    row = json.loads(line)
    if "margin" in row:
        APP_MARGIN = row["margin"]
    elif "hash" in row:
        info[row["path"]] = (int(row["hash"], 16), row["width"], row["height"], row["grid"])
    else:
        print("没有指纹:", row["path"])

def dist(a, b):
    return bin(a[0] ^ b[0] if isinstance(a, tuple) else a ^ b).count("1")

def aspect_diff(a, b):
    ra, rb = a[1] / a[2], b[1] / b[2]
    return abs(ra - rb) / max(ra, rb)

def stats(values):
    values = sorted(values)
    if not values:
        return "无"
    pick = lambda q: values[min(len(values) - 1, int(q * len(values)))]
    return f"n={len(values)} min={values[0]} p1={pick(0.01)} p50={pick(0.5)} p99={pick(0.99)} max={values[-1]}"

positives = {"page": [], "blank": []}
by_kind = {}
for base, kind, path, group in variants:
    original = bases.get(base) or blank[base]
    a, b = info.get(str(original)), info.get(str(path))
    if a is None or b is None:
        continue
    d = dist(a[0], b[0])
    positives[group].append((d, aspect_diff(a, b)))
    by_kind.setdefault(kind, []).append(d)

negatives = {"page": [], "blank": [], "page-vs-blank": []}
def pair_up(names, paths_by_name, group):
    for x, y in itertools.combinations(names, 2):
        a, b = info.get(str(paths_by_name[x])), info.get(str(paths_by_name[y]))
        if a and b:
            negatives[group].append((dist(a[0], b[0]), aspect_diff(a, b), x, y))

pair_up(list(bases), bases, "page")
pair_up(list(blank), blank, "blank")
for x in bases:
    for y in blank:
        a, b = info.get(str(bases[x])), info.get(str(blank[y]))
        if a and b:
            negatives["page-vs-blank"].append((dist(a[0], b[0]), aspect_diff(a, b), x, y))

print()
print("== 同图 (基图 对 它的变体), 距离")
for group in positives:
    print(f"  {group}: {stats([d for d, _ in positives[group]])}")
for kind, values in sorted(by_kind.items()):
    print(f"    {kind:20s} {stats(values)}")
print("== 异图 (不同的图两两比), 距离")
for group in negatives:
    print(f"  {group}: {stats([n[0] for n in negatives[group]])}")
print("== 同图里宽高比差的最大值:", max(a for g in positives.values() for _, a in g))
closest = sorted(n for g in negatives.values() for n in g)[:6]
print("== 最近的几对异图:", [(d, round(asp, 3), x, y) for d, asp, x, y in closest])

allpos = [p for g in positives.values() for p in g]
allneg = [n for g in negatives.values() for n in g]
print()
print("阈值   同图漏判(距离>T 或 宽高比差>=3%)   异图误判(距离<=T 且 宽高比差<3%)")
for t in (0, 2, 4, 6, 8, 10, 12, 16, 20, 24, 28, 32, 40):
    missed = sum(1 for d, asp in allpos if d > t or asp >= ASPECT_TOLERANCE)
    wrong = sum(1 for d, asp, *_ in allneg if d <= t and asp < ASPECT_TOLERANCE)
    print(f"{t:4d}   {missed:4d}/{len(allpos)} = {100 * missed / len(allpos):5.1f}%          {wrong:5d}/{len(allneg)} = {100 * wrong / len(allneg):6.2f}%")
def dhash(grid, margin):
    value = 0
    for row in range(16):
        for column in range(16):
            value = (value << 1) | (1 if grid[row * 17 + column] > grid[row * 17 + column + 1] + margin else 0)
    return value

# Swift 的哈希 (App 当前的 margin) 和这里按格子重新算的必须一致, 否则 margin 的扫描不可信。
assert all(dhash(v[3], APP_MARGIN) == v[0] for v in info.values()), "Python 与 Swift 的 dHash 不一致"
print(f"(App 当前的 margin = {APP_MARGIN}; 上面「同图」「异图」的距离用的就是它)")

print()
print("== margin (左边比右边亮多少灰度级才记 1) 的扫描; 异图里不含那一对「原图和它的译图」(real-sample-a / real-sample-c)")
print("margin  同图(页) max  同图(页) p99  同图(近空白) p99 max   异图(页) min  p1  p50   异图(近空白) 最大")
page_pos = [(base, path) for base, kind, path, group in variants if group == "page"]
blank_pos = [(base, path) for base, kind, path, group in variants if group == "blank"]
def pairs_dist(items, margin, names_to_path):
    out = []
    for base, path in items:
        original = names_to_path[base]
        a, b = info.get(str(original)), info.get(str(path))
        if a and b:
            out.append(bin(dhash(a[3], margin) ^ dhash(b[3], margin)).count("1"))
    return sorted(out)
for margin in (0, 1, 2, 3, 4, 6, 8, 12):
    pp = pairs_dist(page_pos, margin, bases)
    bp = pairs_dist(blank_pos, margin, blank)
    neg = []
    for x, y in itertools.combinations(list(bases), 2):
        if {x, y} == {"real-sample-a", "real-sample-c"}:
            continue
        a, b = info.get(str(bases[x])), info.get(str(bases[y]))
        if a and b:
            neg.append(bin(dhash(a[3], margin) ^ dhash(b[3], margin)).count("1"))
    neg.sort()
    nb = [bin(dhash(info[str(blank[x])][3], margin) ^ dhash(info[str(blank[y])][3], margin)).count("1") for x, y in itertools.combinations(list(blank), 2) if str(blank[x]) in info and str(blank[y]) in info]
    print(f"{margin:5d}   {pp[-1]:8d}   {pp[int(0.99 * len(pp))]:8d}   {bp[int(0.99 * len(bp))]:8d} {bp[-1]:4d}   {neg[0]:10d} {neg[int(0.01 * len(neg))]:4d} {neg[len(neg) // 2]:4d}   {max(nb):8d}")