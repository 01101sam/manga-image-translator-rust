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
- 宿主**在 TSX 注册监听之前发的事件不会重放**。所以 TSX 启动时先挂监听, 再调用 `session.snapshot` 读初始状态; 宿主把初始状态放在 snapshot 里, 之后的变化才用事件推送。
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
  "paired": { "daemon": { "name": "studio", "url": "http://192.168.1.5:8080" }, "token": "…" } // 已保存的配对; 没有则 null
}
```

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

打开系统的文件选择器让用户选图片。

```jsonc
// 参数
{ "limit": 200 }                 // 最多返回多少个
// 返回 (用户取消则 files 为空数组)
{ "files": [{ "id": "…", "name": "page01.png", "size": 123456, "type": "image/png" }] }
```

- `id` 由宿主生成, 只在宿主进程内有意义。宿主把 `id` 映射到文件引用 (App: 沙盒内的临时副本或安全作用域 URL; Web: `File` 对象)。**不要把文件读进内存再回传。**
- App 用 PhotosPicker 和 fileImporter 两种来源, 文件夹要递归展开成图片文件。
- 非图片文件不要返回。

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

让用户保存译图 (或原图)。`url` 是不带 `w` 的原始文件地址 (带 `?token=`), `filename` 不含扩展名。宿主自己下载, 按应答的 `Content-Type` 补扩展名 (`image/png` 为 `.png`, `text/html` 为 `.html`, 其他 `.bin`): App 下载到临时文件后弹分享面板 (ShareLink), Web 用 `fetch` 取回再交给 `<a download>`。失败 reject, TSX 用 toast 显示原因。

## 宿主发给 TSX 的事件

| 事件 | 载荷 | 说明 |
|---|---|---|
| `session.daemons` | `[{ name, url }]` | Bonjour 发现的 Daemon 列表变化 (全量, 不是增量) |
| `files.added` | `[{ id, name, size, type }]` | 用户拖放、粘贴或从分享入口带进来的图片, 与 `files.pick` 同样的文件引用 |
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

## 为 PDF 阅读器预留

`pdf.*` 前缀已在 `capabilities.host` 里授权, 调用和事件由步骤 6 定义 (App 用 PDFKit 的 Native Component, Web 用 pdf.js 覆盖层)。分区路由 `reader` 已有占位页。

## 测试

`tests/app.test.tsx` 用 `nsh test` 的 `open(…, { host: { handlers } })` 扮演宿主, 用 `network` 扮演 Daemon, 覆盖本协议的每个调用。改协议时先改这些测试。
