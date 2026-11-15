# ADR 0003. Daemon 内存：排队只存路径，实例目录，Media 路由的 query token

## 状态

已接受。补充 ADR 0001 决定 4 与「Artifact 放在一份共享临时目录」的部分，不推翻它们。

## 背景

8000x12000 的 PNG 在旧 Daemon 里有三处放大内存。`POST /jobs` 在 async handler 里同步读文件并解码，排队的 Job 一直持有整张解码后的 `DynamicImage`。6 张排队时 RSS 从 64 MB 涨到 1.8 GB，随队列线性增长。Artifact 目录是全局的，每个 Daemon 启动时整个删掉，同机再起一个 Daemon 会清掉正在运行的那个。NSH 的 Image 和浏览器的 `<img>` 发不出 Authorization 头，而 Artifact 被整份读进内存再返回。

## 决定

1. 排队的 Job 只存源文件路径。上传的 multipart 临时文件直接建在实例目录里，校验通过后改名成源文件。Worker 取走 Job 时才在阻塞线程里解码。
2. `POST /jobs` 只读图片文件头，像素数超过 200MP 或单边超过 65535 时返回 413 和中文错误，不解码。解码上限放宽到 2 GiB，避免合法的大图在 Worker 里不明原因地失败。
3. Config 新增 `downscale_long_edge`，默认 4096，`null` 关闭。长边超过它的图在 pipeline 入口等比缩小。它放在 `DaemonConfig` 而不是 `Settings`，因为 `Settings` 同时是 CLI 的输入和 `docs/schema.json` 的来源。
4. 每个 Daemon 实例使用 `<base>/<PID>-<随机后缀>/`。启动时只回收 PID 已不存在的实例目录，退出时（Ctrl-C 和 SIGTERM）只删自己的。旧版直接放在 `<base>` 下的平铺文件，只有在进程表里找不到可能是旧版 Daemon 的进程时才删，`ps` 失败时不删。
5. `GET /jobs/{id}/artifact` 改为流式返回，新增 `GET /jobs/{id}/source` 返回原图。这两个 GET 路由除 Bearer 外接受 `?token=`，其他路由不接受。
6. `/index.html` 与 `/assets/*` 和 `/` 一样免鉴权，为之后嵌入的 NSH web bundle 预留。`/assets/*` 在 bundle 嵌入前一律 404。
7. 单次上传上限从 64 MB 放宽到 256 MB，超限返回 413 和 JSON 错误。图片落盘不占内存，上限只防磁盘被灌满。
8. 源文件保留到 Job 被取消、被丢弃或 Daemon 退出，因为 `/source` 在 Job 完成后还要读它。
9. `/source` 与 `/artifact` 接受 `?w=256|512|1024|2048`，返回长边不超过 w 的 JPEG 预览，不放大，透明处铺白。预览缓存在实例目录的 `preview/` 下，键是 Job id、来源和 w。同时最多生成 2 个预览，其余请求排队。NSH 的 Image 会把整个编码文件下载进内存再降采样，列表缩略图必须由 Daemon 先缩小。

## 后果

排队长度不再放大内存，代价是 Worker 取走 Job 时多一次磁盘读取和解码。损坏的图片在上传时只检查文件头，真正的解码失败要到 Worker 取走 Job 后才出现在 Job 的 `error` 里。

RSS 不回落的主要原因不在图片像素数，也不在 Rust 的分配器。实测（Apple 硅 Mac，`Original` 翻译器，1 个 Worker）：

- 1487x2048 的小图跑 1 个 Job，峰值 RSS 已经是 9.7 GB。缩到 4096 的 8000x12000 图是 10.0 GB，不缩放是 14.1 GB。像素相关的部分只占一小块。
- 每个 Worker 各自持有一份模型，2 个 Worker 各跑 1 个 Job 是 19.0 GB，约为 1 个 Worker 的两倍。
- 同一个 Worker 连续跑第 2 个 Job 还会再涨约 5 GB（9.7 GB 到 14.8 GB），跑完不归还。把 ORT 的 `memory pattern` 关掉后，连跑 3 个 Job 稳定在 9.9 GB。这一行在 `base-util` 的 `new_session_`，不在本 ADR 的范围内。
- 保留下来的大块内存，分配栈都在 onnxruntime 里（推理中间张量、BFC arena、CoreML 输出）。mimalloc 作为全局分配器管不到它们，实测峰值反而高约 2%。

旧版 Daemon 还在运行时启动新版 Daemon，旧版退出时会整个删掉 `<base>`，包括新版的实例目录。升级时先停旧的再启新的。

## 否决的方案

用 `actix-files` 返回文件。仓库之前刻意去掉过它，流式返回只需要 `tokio-util`。

在内存里缓存缩略图。预览放磁盘，Daemon 的 RSS 不随预览数增长。

让 Client 用 `?token=` 直接加载原图，由 NSH Image 降采样。它要先把整个编码文件读进内存，27 张 54 MB 的原图并发时瞬时峰值 1.3 GB。

用 mimalloc 解决 RSS 不回落。保留的内存来自 onnxruntime 自己的分配，Rust 的全局分配器管不到它，实测没有收益。
