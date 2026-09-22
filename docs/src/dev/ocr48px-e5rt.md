# OCR48px E5RT 诊断

生产编码器使用 `new_session_mlprogram_static_shapes`。本页说明如何复跑，以及哪些数字不能当成修复。

## 入口

```
python3 scripts/ocr48px_e5rt.py self-test
python3 scripts/ocr48px_e5rt.py baseline --output-dir /tmp/e5rt-baseline-new
python3 scripts/ocr48px_e5rt.py padding --output-dir /tmp/e5rt-padding-new
python3 scripts/ocr48px_e5rt.py formats --output-dir /tmp/e5rt-formats-new
```

默认新建临时目录。`--output-dir` 必须是空目录或不存在的路径。已有非空目录拒绝覆盖。`--workspace` / `--models-dir` 可选。只复制模型到隔离 workdir，不改仓库 `models/` 与 `models/cache`。

两个 Rust 例子不可互换：

- `e5rt_baseline` 调用生产 OCR 接口 `Ocr48px::detect`，但仅测固定已知区域：页 A 3 区、页 B 3 区。不是全页检测，也不是 30 行基准。
- `e5rt_canvas` 在 `prepare` 之后直接调同源 `infer`。用于 padding / 格式对照。它的页时间不是 detect 基准。

## 本轮已核实事实

- 原生 ORT `GetVersionString=1.22.0`。crate `ort` / `ort-sys` 是 `2.0.0-rc.10`。
- 生产编码器是 MLProgram，并打开 `RequireStaticInputShapes`。2026-09-22 同机 `detect` 基线，改前 `/tmp/e5rt-baseline-20260922` 冷暖 stdout 各 33 条 E5RT，页 A 第二次 631ms。改后 `/tmp/e5rt-baseline-static-20260922` 冷暖 stdout 与 stderr 都是 0，页 A 第二次 246ms。页 A 金标准与页 B 文本与改前一致。脚本标签是 `not_proven`，因为全 0 不会写成 `fixed`。
- `new_session` 是 NeuralNetwork。`new_session_mlprogram` 是不限制静态输入的 MLProgram。`e5rt_canvas` 的 `coreml-mlp` 仍走后者。`new_session_mlprogram_static_shapes` 才是生产编码器。
- memory 运行时是 `[N,seq,320]`，mask 是 `[N,seq]`。所测偶数 W 上 `seq = floor(W/4)`（例如 W=138 时 seq=34，不要写成 138/4 的整数除法歧义）。
- 私有二次垫到 `N=2 W=283`、填 `-1.0` 后，页 A 金标准 `そうだなあ‥` 变成 `そうだなあ…`。该画布禁止接线。
- NeuralNetwork 有 CoreML 分区，E5RT=0，但页延迟与 CPU 同级。格式对照峰值 2.45–2.80GB 来自 `/usr/bin/time -l` 的 `peak memory footprint` 字段，不要混称 max RSS。不能因为零日志换生产。

## 预期失败

`padding` 在 CPU `both` 上应报告 `text=rejected`（已知反例）。这是成功复现，不是脚本崩溃。`pad_broke_text` 是预期否定；mask / memory shape / 原图 gold 错误不能被它遮住。

`formats` 的动态 MLProgram（`coreml-mlp`）仍会看到 E5RT=33。报告必须是 `e5rt_repair=not_fixed`。E5RT 同时数 stdout 与 stderr。真正诊断从 stderr 出现时，不能因 stdout=0 写成消失。禁止把 CPU 的 E5RT=0 或 provider 请求字符串写成已修复。全 0 且无收益证明时标 `not_proven` / `undetermined`，不要写成 `zero => not_fixed`。生产 `detect` 基线全 0 时，标签保持 `not_proven`。不要改脚本把 0 写成 `fixed`。动态 `coreml-mlp` 的 MLP>0 仍是 `not_fixed`。

页 B 只对 CPU 同夹具做逐区域 idx/文本差分，不是金标准，不报 B 准确率。

## 计时口径

| 数字 | 含义 | 不能当成 |
| --- | --- | --- |
| baseline 页 `ms` | 固定 3+3 区上的 `detect` 墙钟，含 CoreML 首次特化 | 全页 / 30 行基准，或 canvas `infer_ms` |
| canvas `load.ms` | 三个会话 `commit_from_file` | 首页推理 |
| canvas `infer_ms` | 现有 `infer` 一次（本轮先 infer，再诊断 encoder） | baseline 首图 3.27s |
| canvas `enc_diag_ms` | 诊断 encoder，在 infer 之后 | 用户首次成本 |
| canvas `wall_ms` | 页墙钟，含诊断 | 生产 detect |

历史 baseline 冷载约 3.61s，页 A 首约 3.27s，峰值约 1.08GB。那是 detect 墙钟。

历史格式对照页 A 首次 `infer_ms` 曾报 MLP 141/133/131/132。那一轮在计时 infer **之前**先跑了诊断 encoder。整理后改为先 infer 再诊断，两轮不可直接互比。这里不写新的时延结论，也不要用历史 131–141ms 推出「2 倍 GUI 加速」。

## 缓存层次

「冷」只表示该进程 CWD 下 `models/cache`（ModelCacheDirectory）为空。系统级 CoreML 缓存是否全冷未知。新 inode 只证明副本不是硬链。

原始大日志在 `/tmp/image-translator-e5rt-20260914-0641/`，不进 git，模型不上传。
