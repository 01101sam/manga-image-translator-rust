# ADR 0001. 单 Daemon 与 Server 持有的 Config

## 状态

已接受。

## 背景

原先是 Bun `gui/server.ts` 监督一个 Rust `api` 子进程。Bun 负责 WebUI、日志环、stderr 解析，以及启动和停止那个子进程。翻译请求走同步 `POST /translate`。配置散落在请求体、环境变量和本机 LaunchAgent。

这套结构把 Engine 的生命周期绑在操作系统进程上。关掉翻译能力等于杀掉 Daemon。Client 也无法共用同一份 Config 与 Artifact。

## 决定

1. 一个 actix-web Daemon 承接全部服务端职责。删除 Bun 层。
2. Engine 的状态是 `Stopped | Starting | Running | Draining`。停止 Engine 会拒绝新 Job、取消排队中的 Job、等正在运行的 Job 结束，然后释放 Models。Daemon 进程继续运行。
3. Config 由 Daemon 持有，写在平台配置目录的 `imagetranslator/config.json`。WebUI 是改 Config 的入口。Client 不保存翻译设置。
4. Job 改为异步句柄。`POST /jobs` 立即返回 `job_id`。队列容量 64。
5. Client 通过 Pairing 拿到 Token。WebUI 页面本身不要求登录。除 `/pair/*`、`/`、`/health` 以外的 API 需要 Bearer。
6. Daemon 在运行期间用 Bonjour 广播 `_imagetranslator._tcp`。
7. 没有开机自启。用 `cargo run --release -p simple-runtime -- daemon` 手动启动。

## 后果

操作者要自己启动 Daemon。WebUI 仍要先完成一次 Pairing，才能调用 `/jobs`、`/engine/*` 和 `/config`。这是为了让未带 Token 的 API 调用返回 401，同时页面本身可以匿名打开。

DeepSeek KEY 出现在 Config JSON 里，并会下发给同一局域网里打开 WebUI 的浏览器。这是操作者接受的个人使用取舍。

`workers` 默认是 2。性能车道会按实测改这个默认值，上限 4。

## 否决的方案

保留 Bun 层。它的五项职责都能在 Rust 里完成，再留一层进程监督没有收益。

把 actix-web 换成 axum。现有服务器已经在 actix-web 上，换框架只是重写。

Server 端用 pdfium 渲染 PDF。PDF Reader 的交互要求 Client 用 PDFKit 在本地翻页。Server 不接触 PDF。

用 SSE 推 Job 状态。1 秒轮询对 Client 更简单。

按会话拆 Artifact 目录。改成一份共享临时目录，Daemon 启动和关闭时清空。

给 WebUI 做登录。操作者明确只要页面匿名可开。

再加一个监督进程。Engine 的启动、停止、重启放在同一个 Daemon 里。

保留同步翻译 API。异步 Job 句柄才能排队、取消、以及在模型加载时不阻塞 HTTP。
