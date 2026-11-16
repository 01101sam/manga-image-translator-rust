# client-ui

图片翻译的界面, 用 [NSH](https://github.com/longbridge/nsh) 的 TSX 写一次, 同时给:

- **Web:** `scripts/build-ui.sh web` 打包成静态站点 `gui/dist/`, 由 Daemon 托管。
- **App (iOS / iPadOS / visionOS):** `scripts/build-ui.sh native` 产出 `client-ui/dist/native/`, Swift 宿主用 `NshAppView` 加载; 宿主侧契约见 [HOST_PROTOCOL.md](HOST_PROTOCOL.md)。

页面: 配对 (Pairing Key; App 预填默认 Key 并记住配对成功的那个, 网页的输入框是空的, 也不存输入的 Key)、任务列表 + 上传 (导入菜单: 相册、文件、打开 PDF, 项目由宿主在 `session.snapshot` 里声明; 排序菜单: 名称 / 日期 / 大小各升降序, 一次性; 条目菜单: 移到最前 / 最后; 长按或鼠标按住拖动重排)、单张查看 (原文/译文)、Config (结构化表单)、Engine 启停、PDF 阅读器 (`reader`: 本地显示 PDF, 方向键翻页, 空格把当前页送去翻译或切换原文/译文; 有译文的页默认显示译文, 切回原文后整个文档保持原文直到再切回来, 这个偏好随 `reader.last` 保存)。

## 结构

```
App.tsx            入口: 配对页或已配对外壳; 配对后才开始轮询
nsh.json           id、capabilities (network、host)、主题、web.host
theme.json         语义 token (浅色/深色)
web/host.js        网页宿主: 选文件、拖放、上传、保存、PDF (pdf.js 覆盖层与页图渲染)
web/pdfjs/         pdf.js 6.x 的运行时文件 (入库, 版本见 VERSION; scripts/vendor-pdfjs.sh 更新)
src/domain/        纯函数与状态机, 不依赖 nsh; tests/ 里逐个测
  session.ts       Session = Discovering | Unpaired | Pairing | Paired{token}
  job.ts           Job 的生命周期 (queued/running/done/failed/cancelled) 与取消
  board.ts         本地 Item = Pending | Uploading | Job; 批量状态; 上传调度。两个顺序: submitOrder (加入顺序, Pending 按它提交) 与 displayOrder (界面顺序)
  arrange.ts       一次性排序 (名称自然序、日期、大小, 缺值排末尾) 与单项移动, 只改 displayOrder
  natural.ts       文件名的自然序比较: 数字段按数值, 不依赖 Intl.Collator (嵌入 App 的引擎里没有)
  reorder.ts       拖动重排的几何: 每个单元格的位置、落点、指示线、边缘自动滚动的速度
  saved.ts         随列表保存的行的读写: Job (id、名字、大小、修改时间) 和还没上传的文件 (`local:<fileId>`, 另存 type 和被拒绝的原因); 有指纹的行另存指纹和宽高比
  similar.ts       相似图片: 256 位 dHash 的汉明距离 ≤ 20 且宽高比相差不到 3% (空白图的哈希几乎全 0, 不参与); 一批文件对列表里已有的 (不分状态) 分出「照常加入」和「重复跳过」
  nav.ts           键盘高亮与查看器的移动
  reader.ts        PDF 阅读器每页的状态机 (移植自旧 Client 的 PageMachine) 与翻页、渲染节奏的纯函数; 看译文还是原文是整个文档的偏好 (store 的 `viewing`), 不在页上
  config.ts        Config 表单的字段表、校验、整份文档的改写
  media.ts         降采样图片 URL (w=256 列表, w=2048 查看器)
src/api/           Daemon 的 HTTP 客户端; 外部 JSON 在 parse.ts 一次性解析
src/ports/host.ts  宿主协议的类型化封装; ports/pdf.ts 是其中 PDF 的部分
src/store/         signal: 每个 Item 一个 signal, 轮询只写变化的项; importer.ts 是用户导入图片的入口 (选择、粘贴、拖放都经过它, 跳过重复的图片并提供「仍然添加」); reorder.ts 是拖动重排的状态 (影子、指示线、自动滚动), menu.ts 是任务页上盖着的菜单 (导入、排序、条目)
src/ui/            应用自己的组件层 (见 DESIGN.md)
src/pages/         页面
tests/             nsh test: 领域单测 + 整个界面的无头流程测试
```

## 数据怎么走

- 图片字节不进 JS (嵌入式 NSH 的 JS 堆上限 256 MB)。列表缩略图用 `<Image src=".../source|artifact?w=256&token=...">`, 查看器用 `w=2048`; 选文件、拖放、上传由宿主完成。
- 状态机在 `src/domain`: 配对、Job、列表 Board 都是纯函数 reducer, 返回新状态和要执行的命令; store 执行命令、写 signal。
- 轮询: 配对后每秒一次 `GET /jobs`, 只更新变化的行; 每 3 秒查一次 Engine。上一轮没结束不会开下一轮。
- 显示顺序: 排序和拖动只改 `displayOrder`, 不改 `submitOrder`, 所以不影响 Pending 提交给 Daemon 的先后, 也不影响 Daemon 的队列; 新加入的条目追加在末尾, 不会自动维持排序。`displayOrder` 随 Job 的 id、名字、大小、修改时间存进 NSH Storage (`jobs:<daemon url>`), 重启或刷新后按它恢复。还没上传的文件 (待上传、上传中、被拒绝) 也在这份存档里 (宿主的 fileId、名字、大小、type、被拒绝的原因; 缩略图不存, 宿主自己留着); 重启后先不等 Daemon, 向宿主 `files.resolve` 找回文件放回列表, Job 等 Daemon 应答后再放回、各自回到存档里的位置, 上传接着做。
- 上传调度: 本地 Pending 按加入顺序提交, 在途 (上传中 + 排队 + 翻译中) 不超过 Daemon 队列容量 64, 同时上传不超过 3 个; 429 回到 Pending 重试, Engine 停止时暂停。
- PDF 不上传: 当前页由宿主显示 (App 是 PDFKit 的原生组件 `it.pdf`, Web 是 pdf.js 的 canvas), 要翻译的页由宿主渲染成 PNG 文件, 再走和图片同一条 `upload.start`, 成为任务列表里的普通 Job。页图一页一页地渲染, 已渲染未上传的最多 5 张, 所以「翻译全部」不会一次把整本变成 PNG。
- 401: 任何受保护请求收到 401 就清掉 Token 回到配对页, 本地待上传的文件保留。

## 命令

```sh
scripts/build-ui.sh check    # TypeScript 类型检查 (nsh check)
scripts/build-ui.sh test     # nsh test: 领域单测与整个界面的无头流程测试
scripts/build-ui.sh web      # gui/dist
scripts/build-ui.sh native   # client-ui/dist/native
scripts/vendor-pdfjs.sh <pdfjs-dist 目录>   # 更新 web/pdfjs (npm pack pdfjs-dist@<版本> 解开后的 package/)
```

`nsh` CLI 来自 `scripts/nsh-visionos/build.sh` (固定 commit, 与 App 的 xcframework 同一份源码)。首次 `nsh check` 会把 TypeScript 7 下载到 `~/Library/Caches/nsh`。

## Daemon 托管 Web 版需要什么

把 `gui/dist/` 整个目录当静态站点, 在 `/` 提供:

- `/` 返回 `index.html` (`Cache-Control: no-cache`); `/manifest.webmanifest`。
- `gui/dist` 下的所有其他文件按原路径返回, **无需鉴权**: `app.<digest>.js`、`host.<digest>.js`、`nsh.<digest>.js`、`nsh.<digest>.wasm` (`Content-Type: application/wasm`)、`nsh-*.js`、`nsh-syntaxes.<digest>.bin`、`nsh-math.<digest>.bin`、目录 `browser.<digest>/`、`facades/`、`icons/`、`assets/`、`pdfjs/` (pdf.js、worker、wasm、字体, PDF 阅读器用, 文件名没有摘要, 按 `no-cache` 处理即可)。文件名带内容摘要, 可以 `Cache-Control: public, max-age=31536000, immutable`。有 `.br` / `.gz` 预压缩副本时, 按 `Accept-Encoding` 返回。
- 这些路径与 Daemon 的 API 路径 (`/health`、`/pair`、`/jobs`、`/config`、`/engine/*`) 不重名。API 保持同源, 页面用 `location.origin` 当 Daemon 地址, 不需要 CORS。

CSP: 页面自己不要求 CSP, Daemon 不发 CSP 头最简单。要发的话, 下面这条在 Chrome 里实测够用 (配对、上传、查看、Config、Engine 启停全流程 21 步, 控制台没有任何 CSP 违规):

```
default-src 'self'; script-src 'self' 'wasm-unsafe-eval' 'sha256-<构建时打印的 hash>'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'
```

要用 PDF 阅读器的话, `connect-src` 要改成 `'self' blob:`: pdf.js 读的是浏览器里选中文件的 object URL (实测上面这条策略会拦下 `blob:`, 加上之后打开 PDF、翻页、worker 与 wasm 解码都没有别的违规)。逐条去掉做过对照: 去掉 `script-src` 的 hash (页面里有一段内联 import map)、去掉 `wasm-unsafe-eval`、把 `connect-src` 设成 `'none'` (runtime 与 bundle 用 fetch 加载) 都会让页面起不来, 去掉 `style-src` 的 `'unsafe-inline'` 会有大量 inline style 被拦 (NSH 用 style 属性定位); `frame-src`、`worker-src`、`img-src` 的 `blob:` 不需要。`sha256-…` 随每次构建变化, `scripts/build-ui.sh web` 把 `script-src` 整行写进 `gui/dist/csp-script-src.txt`。

浏览器会请求 `/favicon.ico`, 页面声明的图标在 `icons/` 下, 所以这个请求在 Daemon 上是 404, 无害; 想去掉就给 `/favicon.ico` 返回 `icons/` 里的 32px 图标。