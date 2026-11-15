# client-ui

图片翻译的界面, 用 [NSH](https://github.com/longbridge/nsh) 的 TSX 写一次, 同时给:

- **Web:** `scripts/build-ui.sh web` 打包成静态站点 `gui/dist/`, 由 Daemon 托管。
- **App (iOS / iPadOS / visionOS):** `scripts/build-ui.sh native` 产出 `client-ui/dist/native/`, Swift 宿主用 `NshAppView` 加载; 宿主侧契约见 [HOST_PROTOCOL.md](HOST_PROTOCOL.md)。

页面: 配对 (Pairing Key)、任务列表 + 上传、单张查看 (原文/译文)、Config (结构化表单)、Engine 启停; PDF 阅读器只有路由占位 (`reader`)。

## 结构

```
App.tsx            入口: 配对页或已配对外壳; 配对后才开始轮询
nsh.json           id、capabilities (network、host)、主题、web.host
theme.json         语义 token (浅色/深色)
web/host.js        网页宿主: 选文件、拖放、上传、保存
src/domain/        纯函数与状态机, 不依赖 nsh; tests/ 里逐个测
  session.ts       Session = Discovering | Unpaired | Pairing | Paired{token}
  job.ts           Job 的生命周期 (queued/running/done/failed/cancelled) 与取消
  board.ts         本地 Item = Pending | Uploading | Job; 批量状态; 上传调度
  nav.ts           键盘高亮与查看器的移动
  config.ts        Config 表单的字段表、校验、整份文档的改写
  media.ts         降采样图片 URL (w=256 列表, w=2048 查看器)
src/api/           Daemon 的 HTTP 客户端; 外部 JSON 在 parse.ts 一次性解析
src/ports/host.ts  宿主协议的类型化封装
src/store/         signal: 每个 Item 一个 signal, 轮询只写变化的项
src/ui/            应用自己的组件层 (见 DESIGN.md)
src/pages/         页面
tests/             nsh test: 领域单测 + 整个界面的无头流程测试
```

## 数据怎么走

- 图片字节不进 JS (嵌入式 NSH 的 JS 堆上限 256 MB)。列表缩略图用 `<Image src=".../source|artifact?w=256&token=...">`, 查看器用 `w=2048`; 选文件、拖放、上传由宿主完成。
- 状态机在 `src/domain`: 配对、Job、列表 Board 都是纯函数 reducer, 返回新状态和要执行的命令; store 执行命令、写 signal。
- 轮询: 配对后每秒一次 `GET /jobs`, 只更新变化的行; 每 3 秒查一次 Engine。上一轮没结束不会开下一轮。
- 上传调度: 本地 Pending 按顺序提交, 在途 (上传中 + 排队 + 翻译中) 不超过 Daemon 队列容量 64, 同时上传不超过 3 个; 429 回到 Pending 重试, Engine 停止时暂停。
- 401: 任何受保护请求收到 401 就清掉 Token 回到配对页, 本地待上传的文件保留。

## 命令

```sh
scripts/build-ui.sh check    # TypeScript 类型检查 (nsh check)
scripts/build-ui.sh test     # nsh test: 领域单测与整个界面的无头流程测试
scripts/build-ui.sh web      # gui/dist
scripts/build-ui.sh native   # client-ui/dist/native
```

`nsh` CLI 来自 `scripts/nsh-visionos/build.sh` (固定 commit, 与 App 的 xcframework 同一份源码)。首次 `nsh check` 会把 TypeScript 7 下载到 `~/Library/Caches/nsh`。

## Daemon 托管 Web 版需要什么

把 `gui/dist/` 整个目录当静态站点, 在 `/` 提供:

- `/` 返回 `index.html` (`Cache-Control: no-cache`); `/manifest.webmanifest`。
- `gui/dist` 下的所有其他文件按原路径返回, **无需鉴权**: `app.<digest>.js`、`host.<digest>.js`、`nsh.<digest>.js`、`nsh.<digest>.wasm` (`Content-Type: application/wasm`)、`nsh-*.js`、`nsh-syntaxes.<digest>.bin`、`nsh-math.<digest>.bin`、目录 `browser.<digest>/`、`facades/`、`icons/`、`assets/`。文件名带内容摘要, 可以 `Cache-Control: public, max-age=31536000, immutable`。有 `.br` / `.gz` 预压缩副本时, 按 `Accept-Encoding` 返回。
- 这些路径与 Daemon 的 API 路径 (`/health`、`/pair`、`/jobs`、`/config`、`/engine/*`) 不重名。API 保持同源, 页面用 `location.origin` 当 Daemon 地址, 不需要 CORS。

CSP: 页面自己不要求 CSP, Daemon 不发 CSP 头最简单。要发的话, 下面这条在 Chrome 里实测够用 (配对、上传、查看、Config、Engine 启停全流程 21 步, 控制台没有任何 CSP 违规):

```
default-src 'self'; script-src 'self' 'wasm-unsafe-eval' 'sha256-<构建时打印的 hash>'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'
```

逐条去掉做过对照: 去掉 `script-src` 的 hash (页面里有一段内联 import map)、去掉 `wasm-unsafe-eval`、把 `connect-src` 设成 `'none'` (runtime 与 bundle 用 fetch 加载) 都会让页面起不来, 去掉 `style-src` 的 `'unsafe-inline'` 会有大量 inline style 被拦 (NSH 用 style 属性定位); `frame-src`、`worker-src`、`img-src` 的 `blob:` 不需要。`sha256-…` 随每次构建变化, `scripts/build-ui.sh web` 把 `script-src` 整行写进 `gui/dist/csp-script-src.txt`。

浏览器会请求 `/favicon.ico`, 页面声明的图标在 `icons/` 下, 所以这个请求在 Daemon 上是 404, 无害; 想去掉就给 `/favicon.ico` 返回 `icons/` 里的 32px 图标。
