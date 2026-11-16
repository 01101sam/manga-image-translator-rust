# 宿主协议

`client-ui` 的 TSX 只通过 NSH 的 `host` 通道和外界打交道: 调用 `host.call(name, args)`, 发送 `host.post(name, args)`, 监听 `host.on(event, listener)`。同一套 TSX 有两个宿主实现, 协议完全相同:

| 宿主 | 实现 | 说明 |
|---|---|---|
| Web | `web/host.js` (`nsh.json` 的 `web.host`) | 跑在页面里, 文件是浏览器的 `File` 对象, 上传用浏览器的 `fetch` + `FormData` |
| App (iOS / iPadOS / visionOS) | Swift, 嵌入 `NshAppView` | 步骤 5 实现。文件留在沙盒里, 用 `URLSession.uploadTask(fromFile:)` 流式上传 |

**设计前提:** 图片字节永远不进 JS。嵌入式 NSH 的 JS 堆上限 256 MB, 超限整个 VM 停止。所以选文件、拖放、上传、保存都由宿主完成, TSX 只持有宿主发来的文件引用 (`id` + 名字 + 大小)。显示图片则由 `<Image src="http://…/jobs/{id}/artifact?w=256&token=…">` 直接加载降采样的 JPEG。

约定:

- 参数和返回值都是 JSON。调用成功 resolve 返回值; 失败时 reject, 消息是人读的中文或英文。
- 名字只含 `A-Z a-z 0-9 _ . : -`。`nsh.json` 的 `capabilities.host` 授权 `session.*`、`files.*`、`upload.*`、`artifact.*`、`pdf.*`; 宿主新增别的前缀时要同步加。事件不需要授权。
- 宿主**在 TSX 注册监听之前发的事件不会重放**。所以 TSX 启动时先挂监听, 再调用 `session.snapshot` 读初始状态; 宿主把初始状态放在 snapshot 里, 之后的变化才用事件推送。宿主因此把 `session.snapshot` 当作「监听已经挂好」的信号: 在它被调用之前要发的 `files.added`、`pdf.added`、`host.notice` (冷启动时从分享扩展的收件箱搬来的文件、「打开方式」带来的文件) 先攒着, 调用之后按原顺序补发; 界面重新载入时重新等这个信号。
- `files.pick` 和 `upload.start` 由 TSX 以 `timeout: 0` 调用 (不限时): 用户可能在选择器里停很久, 大图上传可能很慢。其余调用用默认的 30 秒。
- TSX 自己限制并发: 同时在途的 `upload.start` 不超过 3 个, 在途 Job (排队 + 翻译中 + 上传中) 不超过 Daemon 队列容量 64。宿主不需要再排队, 但也不要假设一次只会来一个。

## TSX 调用宿主

### `session.snapshot`

启动时调用一次, 读宿主已知的配对状态。

```jsonc
// 参数: 无
// 返回:
{
  "daemons": [{ "name": "studio", "url": "http://192.168.1.5:8080" }],   // 此刻发现的 Daemon, url 不带结尾斜杠
  "paired": { "daemon": { "name": "studio", "url": "http://192.168.1.5:8080" }, "token": "…" }, // 已保存的配对; 没有则 null
  "imports": ["photos", "files", "folder", "pdf", "paste"]                                     // 宿主支持的导入入口
}
```

- `imports`: 导入菜单只显示宿主声明的入口, 顺序固定为 `photos` (从相册, `files.pick` 的 `source: "photos"`)、`files` (从文件, `source: "files"`)、`folder` (选择文件夹, `source: "folder"`)、`pdf` (打开 PDF, `pdf.pick`)、`paste` (粘贴, `files.paste`; 声明了它 TSX 才绑 ⌘V)。没有这个字段 (老宿主) 当作 `["files", "pdf"]`。App 声明 `photos`、`files`、`pdf`、`paste` (文件夹含在「从文件」的文档选择器里); Web 声明 `files`、`pdf`, 浏览器支持选目录 (`webkitdirectory`) 时再加 `folder`。

- App: `daemons` 来自 Bonjour (`BonjourBrowser`), `paired` 来自 Keychain。`paired` 的 Daemon 当前没被发现也照样返回 (Token 永久有效, 不依赖 Bonjour 在线)。url 里的 IPv6 zone (`%en0`) 要先去掉。
- Web: `daemons` 只有一项, 就是页面自己的来源 `{ "name": location.host, "url": location.origin }`; `paired` 恒为 `null`, Token 由 TSX 存在 NSH `Storage` (浏览器 localStorage), 键 `session.token:<url>`。
- 只发现一个 Daemon 且没有 `paired` 时, TSX 直接显示 Pairing Key 输入框, 不再让用户点选。

### `session.saveToken` / `session.clearToken`

```jsonc
// session.saveToken 参数
{ "daemon": { "name": "studio", "url": "http://…" }, "token": "…" }
// session.clearToken 参数
{ "daemon": { "name": "studio", "url": "http://…" } }
// 返回: null
```

- 配对成功时调用一次 `saveToken`, 宿主写入 Keychain 并永久保存; 之后每次启动由 `session.snapshot` 带回。
- `clearToken` 在两种情况调用: Daemon 对这个 Token 回了 401 (TSX 回到未配对), 用户点了「取消配对」。
- Web 宿主不会收到这两个调用 (TSX 直接用 Storage), 实现成返回 `null` 即可。

### `files.pick`

打开系统的选择器让用户选图片, 由导入菜单的「从相册」「从文件」触发。

```jsonc
// 参数
{ "limit": 200, "source": "photos" }   // limit: 最多返回多少个; source: "photos" | "files" | "folder", 缺省 "files"
// 返回 (用户取消则 files 为空数组)
{ "files": [{ "id": "…", "name": "page01.png", "size": 123456, "type": "image/png" }] }
```

- `source`: `photos` 是相册 (App: PHPicker, 只出图片); `files` 是文件选择器 (App: 文档选择器, 图片、PDF 和文件夹都能选, 文件夹递归并按自然序; Web: 浏览器的文件对话框); `folder` 只有 Web 声明, 弹浏览器的目录选择 (`<input webkitdirectory>`), 取出里面的图片和 PDF 并按相对路径自然序排 (page2 在 page10 前, 与 App 的文件夹导入一致)。
- 选出来的 **PDF 不放进 `files`**, 与拖放一致走事件 `pdf.added`: 只取第一个, 其余宿主立刻放掉并发一条 `host.notice`。

- `id` 由宿主生成, 只在宿主进程内有意义。宿主把 `id` 映射到文件引用 (App: 沙盒内的临时副本或安全作用域 URL; Web: `File` 对象)。**不要把文件读进内存再回传。**
- 非图片、非 PDF 的文件不要返回, 数一下发一条 `host.notice`。

### `files.paste`

读系统剪贴板里的图片。导入菜单的「粘贴」和 ⌘V (`keymap` 的 `mod+v`) 调用; 只有声明了 `paste` 的宿主 (App) 才会收到, 也只有它们 TSX 才绑 ⌘V。网页不声明: 浏览器自己发 `paste` 事件, `web/host.js` 取 `clipboardData.files` 走 `files.added`, TSX 若绑了 ⌘V 反而会吃掉这个按键。

```jsonc
// 参数: 无 (TSX 以 timeout: 0 调用)
// 返回
{ "files": [{ "id": "…", "name": "image.png", "size": 123456, "type": "image/png" }] }
```

- 图片和 PDF 都按文件表示读 (`NSItemProvider` 的文件表示, 与拖放同一条路径), 不解码大图; 复制的文件引用 (file URL) 也认。PDF 不放进返回值, 走 `pdf.added`。
- 剪贴板里没有图片、PDF 或文件时返回空数组, 并发一条 `host.notice` (「剪贴板里没有可以导入的图片或 PDF」)。
- iOS 在内容来自别的 App 时会问一次「允许粘贴」。宿主先用 `hasImages`、`hasURLs` 这类不触发提示的查询看有没有可读的类型, 没有就不去读内容; 用户可以在系统设置里给这个 App 选「允许」, 之后不再问。
- 图片一到 TSX 就回到任务页 (⌘V 在哪个分区都能按); 文本框有焦点时 ⌘V 归文本框。

### `files.release` (post, 无返回)

```jsonc
{ "ids": ["…", "…"] }
```

用户移除了还没上传的文件、或文件被 Daemon 拒绝后, TSX 通知宿主释放它们 (删临时副本、放掉安全作用域)。上传成功后宿主**自己**释放对应文件, TSX 不会再发 release。

### `upload.start`

把一个文件流式上传给 Daemon 的 `POST /jobs`。

```jsonc
// 参数
{
  "fileId": "…",
  "url": "http://192.168.1.5:8080/jobs",
  "token": "…",
  "overrides": { "target_lang": "chs", "detector": "DBNet", "translator": "Llm" }   // 可为 {} ; 键只会是这三个
}
// 返回 (永远 resolve 一个结果对象, 业务失败不要 reject)
{ "ok": true, "jobId": "…" }
{ "ok": false, "kind": "queue-full" | "engine-stopped" | "unauthorized" | "rejected" | "network", "message": "给人看的原因" }
```

请求要求:

- `multipart/form-data`, 两个字段: `image` (文件, 带文件名和类型) 和 `overrides` (上面那个对象的 JSON 字符串)。头 `Authorization: Bearer <token>`。
- **流式**: App 先把 multipart 的头、文件内容、尾依次写进一个临时文件, 再 `uploadTask(with:fromFile:)`; 不能 `Data(contentsOf:)`。Web 用 `fetch` 的 `FormData` 体, 浏览器自己流式处理。

状态码到 `kind` 的映射 (host.js 和 Swift 一致):

| Daemon 应答 | `kind` | TSX 的反应 |
|---|---|---|
| 200 且有 `job_id` | (ok) | 该项变成 Job, 宿主释放文件 |
| 401 | `unauthorized` | 回到未配对, 文件仍留在队列里 |
| 429 | `queue-full` | 文件回到 Pending, 1.5 秒后重试 |
| 503 | `engine-stopped` | 文件回到 Pending, 等 Engine 启动 |
| 400 / 413 / 415 / 422 | `rejected` | 文件留在列表里标红, 不再自动重试, 用户可重试或移除, 宿主应保留文件直到收到 `files.release` |
| 5xx、连不上、超时 | `network` | 文件回到 Pending, 3 秒后重试 |

`queue-full`、`engine-stopped`、`unauthorized`、`network` 时宿主必须**保留**文件, TSX 会再次用同一个 `fileId` 调用。

### `artifact.save`

```jsonc
{ "url": "http://…/jobs/{id}/artifact?token=…", "filename": "page01-translated" }   // 返回 null
```

让用户保存译图 (或原图)。`url` 是不带 `w` 的原始文件地址 (带 `?token=`), `filename` 不含扩展名。宿主自己下载, 按应答的 `Content-Type` 补扩展名 (`image/png` 为 `.png`, `image/jpeg` 为 `.jpg`, `image/webp` 为 `.webp`, `text/html` 为 `.html`, 其他 `.bin`; 原图常是 jpeg 或 webp, 不能落成 `.bin`): App 下载到临时文件后弹分享面板 (ShareLink), Web 用 `fetch` 取回再交给 `<a download>`。失败 reject, TSX 用 toast 显示原因。

## 宿主发给 TSX 的事件

| 事件 | 载荷 | 说明 |
|---|---|---|
| `session.daemons` | `[{ name, url }]` | Bonjour 发现的 Daemon 列表变化 (全量, 不是增量) |
| `files.added` | `[{ id, name, size, type }]` | 用户拖放、粘贴或从分享入口带进来的图片, 与 `files.pick` 同样的文件引用。Web 的 ⌘V / Ctrl+V 由 `web/host.js` 监听浏览器的 `paste` 事件 (取 `clipboardData.files`), 走这个事件; 拖入的文件夹同样按自然序 |
| `files.dragState` | `{ "active": true \| false }` | 拖拽进入 / 离开窗口, TSX 据此显示「松开以添加」。Web 发; App 可选 |
| `host.notice` | `{ "text": "…" }` | 一条给用户看的提示 (比如「已忽略 3 个不是图片的文件」), TSX 用 toast 显示 |

拖放的规则 (App 侧, 步骤 5): 每个 `NSItemProvider` 只读一种表示; 并发读取有上限, 不要同时展开几百个; 目录要递归。TSX 不关心来源, 只收 `files.added`。

## 启动参数

宿主在加载 bundle 之前可以设置 `host.params` (Swift: `appView.params = [...]`; Web: `createPlayer` 的参数):

| 参数 | 值 | 说明 |
|---|---|---|
| `section` | `"jobs"` (默认) \| `"reader"` \| `"config"` | 起始分区, UI 测试和截图用 |

## 嵌入 iOS 时可用的 nsh/base 组件

NativeShell.xcframework 的默认运行时只注册 `nsh/base` 的 **core** 部分, 其余部分 (`Select`、`Menu`、`Popover`、`Sheet`、`ToastStack`、`Combobox`、`Tooltip` 等) 不在, 按名字 import 它们会让整个 bundle 加载失败 (`Could not find export 'Select' in module 'nsh/base'`); 嵌入运行时没有 Swift 侧的开关可以加。

实测 (iPad 模拟器与 visionOS 模拟器上, 探针 bundle 用 `import * as base from "nsh/base"` 看 `typeof`): 有 `Button`、`Input`、`Switch`、`Tabs`、`Radio`、`Progress`、`Dialog`、`NavStack`、`NavPage`、`createNavStack`、`AlertDialog`; 没有 `Select`、`Menu`、`Popover`、`Sheet`、`ToastStack`、`Combobox`、`Tooltip`。`Dialog` 与 `NavStack` 的导出存在, 但没有在这两个平台上渲染过, 步骤 6 要用时先实测。`client-ui` 目前只用 `Button`、`Input`、`Switch`、`Tab`、`Tabs`、`Progress`、`Radio`、`RadioGroup`, `scripts/build-ui.sh check` 会拦住 core 以外的导入。

## PDF 阅读器

Server 不接触 PDF, 来源只有用户本地文件。App 宿主把 PDF 存进沙盒, Web 的 `web/host.js` 把浏览器的 `File` 管成 object URL。分区路由 `reader` 承载阅读器; `pdf.*` 前缀已在 `capabilities.host` 里授权 (`pdf.open` 与 `pdf.export` 也在其中)。

### `pdf.pick`

```jsonc
// 参数: 无 (TSX 以 timeout: 0 调用)
// 返回; 用户取消则 file 为 null
{ "file": { "id": "…", "name": "book.pdf", "size": 1234567, "type": "application/pdf", "pageCount": 120 } }
```

- App: 复制到 `Application Support/…/pdfs/<uuid>.pdf` (不放 tmp), `id` 就是这个 uuid, 重启后仍能解析; 只有 `pdf.release` 或超过 30 天没有被打开才会删。`pageCount` 由宿主读出; Web 同形返回, 读不到时省略。TSX 以组件的 `loaded` 事件为准。
- 拖放进来的 PDF 不走 `files.added` (那里只有图片), 而是事件 `pdf.added`, 载荷是上面 `file` 的数组。一次拖放里既有图片又有 PDF 时, 两个事件各发一次。

### `pdf.open`

重启后恢复上次打开的 PDF: TSX 在 NSH Storage 里记着上次的 `fileId` 和页码 (键 `reader.last`), 启动时用它问宿主文件还在不在。

```jsonc
// 参数
{ "fileId": "…" }
// 返回; 宿主已经没有这个文件时 file 为 null, TSX 随即忘掉这条记录
{ "file": { "id": "…", "name": "book.pdf", "size": 1234567, "type": "application/pdf", "pageCount": 120 } }
```

- App: PDF 在 `Application Support/…/pdfs` 里留 30 天 (每次打开刷新), 所以重启后通常还在; 只恢复文档和页码, 各页的翻译进度属于 Daemon 的 Job, 不恢复。
- Web: 恒为 `{ "file": null }` (object URL 刷新页面就失效)。

### `pdf.export`

把已翻译的页换进原 PDF, 拼成新 PDF 再交给系统分享面板。只有 App 宿主实现; Web 宿主 reject 一句「网页版不支持导出 PDF」, TSX 在网页上不显示导出按钮。

```jsonc
// 参数 (TSX 以 timeout: 0 调用: 逐页下载译文, 页多时要很久)
{
  "fileId": "…",
  "pages": [{ "page": 1, "artifactUrl": "http://…/jobs/{id}/artifact?token=…" }]   // 页码从 0 起; 没列出的页用原页
}
// 返回 (用户关掉分享面板之前就返回)
{ "translated": 3, "missing": 0 }
```

- 宿主在后台线程**逐页**处理: 下载这一页的译文到临时文件、用 ImageIO 降采样到页面点尺寸的 2 倍 (JPEG 编码后嵌入, 文件小)、写进输出 PDF、删临时文件, 再处理下一页, 不同时持有多页位图。没列出的页把原页 (矢量) 原样画进去, 页面框取原页的显示尺寸。
- `missing` 是给了地址却取不到或不是图片的页数 (比如 Daemon 重启后 Artifact 没了), 这几页用原页; TSX 据此提示。
- `fileId` 不存在、PDF 有密码、写文件失败时 reject。输出文件名 `<原名>-translated.pdf`, 分享面板关闭后宿主删掉它。

### `pdf.release` (post, 无返回)

```jsonc
{ "fileId": "…" }
```

用户关闭这个 PDF 后调用, 宿主删掉沙盒副本。

### 原生组件 `it.pdf`

```ts
nativeComponent<
  { fileId: string; page: number; hidden?: boolean },
  { loaded: { pageCount: number }; pageChanged: { page: number } }
>("it.pdf", { props: ["fileId", "page", "hidden"], events: ["loaded", "pageChanged"] });
```

- `fileId`: `pdf.pick` / `pdf.added` 给的 id。变化时重新加载文档。
- `page`: 从 0 开始。收到和当前页不同的值就跳过去。用户在 PDFView 里翻页时宿主发 `pageChanged`, TSX 把值写回 `page`。
- `hidden`: 为真时隐藏 PDFView (译文叠加显示时用), 文档和当前页保留。
- ref 命令: `goto` (参数 `{ "page": n }`, 裸数字也收), `next`, `prev`; 越界忽略。
- 事件: 文档打开后发一次 `loaded` 再发一次 `pageChanged { page: 0 }`; 之后每次当前页变化发 `pageChanged`。
- PDFView 用 `singlePage` + `autoScales`, 只渲染当前页, 宿主不把整页光栅化成图, 也不预渲染相邻页。

### `pdf.renderPage`

把某一页送去翻译前, 先让宿主渲染成 PNG 文件, 再走和图片一样的 `upload.start`。

```jsonc
// 参数 (page 从 0 开始; longEdge 缺省 3072, 宿主限制在 256 到 6144)
{ "fileId": "…", "page": 0, "longEdge": 3072 }
// 返回
{ "fileRef": { "id": "…", "name": "book-p001.png", "size": 2345678, "type": "image/png" } }
```

- `fileRef` 和 `files.pick` 返回的文件引用同形, `id` 直接当 `upload.start` 的 `fileId`, 上传和释放规则完全一样 (成功后宿主自己释放; 被拒绝或放弃时 TSX 发 `files.release`)。渲染出的 PNG 在宿主沙盒里, 不进 JS。
- 页码越界、`fileId` 已不存在、文件不是 PDF 时 reject。
- `longEdge` 一般取 Config 的 `downscale_long_edge` (没有就 3072)。宿主把页面画到长边等于 `longEdge` (矢量页会放大, 扫描页受原图分辨率限制); 上限 6144 是因为渲染要整页位图, 再大 iPad 会吃不消。
- 页 → Job 的映射、译文叠加和空格切换是 TSX 的事 (`PageMachine`); 宿主只管上面这几个调用和组件。

### Web: pdf.js 覆盖层 (`pdf.show` / `pdf.dispose`)

NSH 的 Web 版没有注册的原生组件, `it.pdf` 在网页上是个空 View。所以 `PdfSurface` 在 `platform.os === "web"` 时改放一个 `testID="pdf-slot"` 的占位 View, 它在页面里是带 `data-testid="pdf-slot"` 的 DOM 节点, `web/host.js` 把 pdf.js 的 canvas 放进这个节点里。位置、裁剪、窗口缩放和层叠都交给浏览器; 译文 `<Image>` 是占位节点后面的兄弟节点, 自然盖在 canvas 上面。

```jsonc
// pdf.show: 幂等的「设成这个状态」。TSX 在 fileId / page / hidden 任何一个变化时调用, 连续调用时宿主只画最新的一次
{ "fileId": "…", "page": 3, "hidden": false }
// 返回 (所有排在前面的调用也拿到同一个结果)
{ "pageCount": 212 }
// pdf.dispose (post, 无参数): 占位节点要离开屏幕 (切到别的分区、关掉 PDF), 拆掉 canvas、放掉渲染缓冲; 文档保持打开
```

- 只渲染当前页: canvas 的像素尺寸是占位节点的显示尺寸乘 `devicePixelRatio`, 长边最多 3000 像素。新页画好之后才换掉旧 canvas, 所以翻页不闪白, 任何时刻占位节点里只有一个 canvas。`hidden` 为 true 时不渲染也不换页, 变回 false 时补画。占位节点改变大小 (窗口缩放、横竖屏) 时宿主用 `ResizeObserver` 自己重画。
- `pdf.show` 打不开文档就 reject; TSX 在页面上显示这条原因。
- 文档只在覆盖层显示它、或正在渲染页图时保持打开 (打开后连同 worker 和读入的数据可以占到几百 MB)。覆盖层离开屏幕或页图渲染完 20 秒后, 宿主销毁 pdf.js 的加载任务, 回来时重新打开; `pdf.release` 立即销毁并 revoke object URL。
- 解码: `getDocument` 传了 `isImageDecoderSupported: false` 和 `isOffscreenCanvasSupported: false`, 嵌在 PDF 里的 JPEG 由 worker 里的 JS 解码。默认走浏览器的 ImageDecoder 时, 解出的位图一直留在渲染进程里: 100 页的图片 PDF 翻完渲染进程约 1.1 GB, 24 秒后仍是 1.1 GB; 关掉后翻完约 0.6 GB, 24 秒后回落到约 0.5 GB (footprint 从约 825 MB 降到约 250 MB)。代价是每页重绘慢一些, 一页重画的耗时约 56 ms 变成约 185 ms (Chrome headless, 3 次)。要换回去就删掉这两个选项。
- pdf.js、它的 worker 和解码用的 wasm、字体、CMap 都是同源静态文件, 在站点的 `pdfjs/` 下 (`pdf.js`、`pdf.worker.js`, 改成 `.js` 是因为 Daemon 按扩展名发 Content-Type, 不认识 `.mjs`); 版本见 `web/pdfjs/VERSION`, 来源与更新方法见 `scripts/vendor-pdfjs.sh`。`scripts/build-ui.sh web` 把它们拷进 `gui/dist/pdfjs/`。
- PDF 用 object URL 交给 pdf.js, 所以页面的 CSP 若限制了 `connect-src`, 要加上 `blob:`。
- `pdf.renderPage` 在 Web 上用同一个 pdf.js 文档渲染到一块临时 canvas, `toBlob` 成 PNG 后立刻清零 canvas; 文件名是 `<PDF 名>-p001.png` 这种三位页号。

### TSX 一侧的行为

- 页与 Job 的对应: 页图作为普通本地文件进任务列表, 上传成功后 `jobId` 记在这一页上; 之后每页的状态跟着轮询回来的 Job 状态走。页图出现在任务列表里, 与图片一样受 Pending Upload 分批 (在途不超过 Daemon 报告的队列容量、同时上传不超过 3 个)。
- 渲染节奏: 同一时间只有一个 `pdf.renderPage` 在途, 已渲染但还没上传的页图最多 5 张 (上传跟不上, 比如队列满或 Engine 停止, 就先不渲染下一页), 「翻译全部」不会一次渲染整本。
- `pageChanged` 的回声: PDFView 对每次换页都发 `pageChanged`, 包括 TSX 改 `page` 属性引起的, 以及每次回到阅读器时重新挂载的组件先落到第 0 页再跳到 TSX 要的页。TSX 只在宿主上一次报告的页码等于自己的当前页时 (两边本来同步) 才把新报告当成用户自己翻的页 (滑动、点 PDF 里的链接); 刚挂载还没有报告过时一律不跟。所以连按方向键时滞后的回声、重新挂载时的 `0, 7, 7` 都不会改动页码。
- 键盘 (方向键翻页, 空格翻译本页或切换原文与译文) 由 TSX 的 `keymap` 处理, 不依赖宿主的按键事件。

## 测试

`tests/app.test.tsx` 和 `tests/reader-app.test.tsx` 用 `nsh test` 的 `open(…, { host: { handlers } })` 扮演宿主, 用 `network` 扮演 Daemon, 覆盖本协议的每个调用 (`it.pdf` 的事件无头测试发不出来, 靠 `tests/reader.test.ts` 里的纯函数测试和宿主一侧的测试)。改协议时先改这些测试。
