# App (iOS / iPadOS / visionOS)

一个很薄的 Swift 宿主, 用 `NshAppView` 加载 `client-ui` 的 TSX 界面。界面和逻辑都在 TSX 里 (和 Web 版是同一份代码); Swift 只做 TSX 做不了的事: Bonjour 发现、Keychain 存 Token、拖放和文件选择、流式上传、保存文件、PDFKit。宿主协议见 [client-ui/HOST_PROTOCOL.md](../client-ui/HOST_PROTOCOL.md)。

## 构建

构建前要有两样东西, 都不入库:

1. `app/Frameworks/NativeShell.xcframework`: 运行 `scripts/nsh-visionos/build.sh` 生成 (固定 NSH commit, 含 iOS、iOS 模拟器、visionOS、visionOS 模拟器四个切片, 都只有 arm64)。
2. `client-ui/dist/native/bundle.nshc`: Xcode 的 "Embed NSH UI bundle" 构建阶段调用 `scripts/embed-ui.sh`, 在 bundle 不存在或 `client-ui` 源码比它新时用 `scripts/build-ui.sh native` 重建, 再把 `bundle.nshc`、`app.json`、`capabilities.json` 拷进 App 的 `nsh/` 目录。别人正在改 `client-ui` 时可以设 `NSH_UI_SKIP_BUILD=1`, 只拷现成的 bundle。

```sh
cd app
xcodebuild build -project ImageTranslator.xcodeproj -scheme App -destination 'platform=iOS Simulator,name=iPad Pro 13-inch (M5)'
xcodebuild build -project ImageTranslator.xcodeproj -scheme App -destination 'platform=visionOS Simulator,name=Apple Vision Pro'
```

xcframework 没有 x86_64 切片, 工程已对 iOS 与 visionOS 模拟器排除 x86_64。

网络: `Info.plist` 只开 `NSAllowsLocalNetworking` (局域网 IP 和 `.local`), 没有 `NSAllowsArbitraryLoads`; NSH 的 `Image` 和宿主的 `URLSession` 都用 http 直连局域网 Daemon。NSH 的 capabilities 在 `client-ui/nsh.json` 里声明 (`network.hosts: ["*"]` 和 `host: session.* files.* upload.* artifact.* pdf.*`), 宿主不再额外收窄。

## 宿主做什么

| 文件 | 职责 |
|---|---|
| `Host/NshHostViewController.swift` | 持有 `NshAppView`, 注册 HOST_PROTOCOL 的全部调用, 接 `UIDropInteraction`; bundle 加载失败或 JS 虚拟机报错时显示错误和「重新载入」 |
| `Host/HostSession.swift` | `session.*`: Bonjour 发现的 Daemon 加 Keychain 里的 Token, Token 永久保存 |
| `Host/HostFiles.swift` | `ManagedFiles`: 文件 id 到沙盒文件。图片在 tmp, 启动时整个清掉; PDF 在 Application Support, id 是文件名里的 uuid, 超过 30 天没打开才在启动时清掉 |
| `Host/HostIngest.swift` | 拖放和选择的文件进沙盒。每个 `NSItemProvider` 只读一种表示, 一次只读一个, 目录递归并按自然序排 |
| `Host/HostPickers.swift` | 照片 (PHPicker)、文件、文件夹三种选择器, iPad 与 visionOS 用居中的 popover |
| `Host/HostUpload.swift` | `upload.start`: multipart 先分块写进临时文件, 再 `URLSession.upload(fromFile:)`, 字节不进 JS 也不在内存里拼 `Data` |
| `Host/HostArtifact.swift` | `artifact.save`: 下载到临时文件, 弹系统分享面板 |
| `Host/HostPdf.swift` | 原生组件 `it.pdf` (PDFView, 单页, 只渲染当前页) 和 `pdf.renderPage` (CGContext 画到长边, 直接写 PNG 文件) |

旧的 SwiftUI 界面 (`RootView`、`JobListView`、`PDFReaderView`、`AppSession` 等) 还留在 target 里但不再被引用, 步骤 7 删除。

## 内存

- 图片字节不进 JS (嵌入式 NSH 的 JS 堆上限 256 MB); 列表和查看器显示 Daemon 降采样后的 JPEG (`?w=`)。
- 上传、拖放、选择都走文件, 宿主不把图片读进内存。`NSItemProvider` 的文件表示由系统在回调里给一个临时文件, 宿主在回调里把它搬进沙盒。
- NSH 的图片缓存在磁盘上限 256 MB。
- 拖放摄入一次只读一个 provider。iPad 模拟器里用进程内 `NSItemProvider(contentsOf:)` 读 30 张 8000x12000 的 PNG (每张 37 MB): 并发 2 或 4 时 RSS 峰值 1.37 GB (4 次实测 1364 到 1374 MB), 并发 1 时 332、366、357 MB (3 次); 文件选择器的 URL 路径 (`FileManager.copyItem`, APFS 克隆) 是 295 MB。三种情况的 phys_footprint 都是 23 到 24 MB, 所以涨的是读过的文件页, 不是 jetsam 看的内存。`loadInPlaceFileRepresentation` 在并发 4 和 1 时的峰值与 `loadFileRepresentation` 一样, 没有采用。200 张 1.7 MB 的 JPEG 并发 1 和 4 的耗时都是 0.9 秒。真实跨应用拖放走系统的进程间拷贝, 没有测过, 并发 1 只是让进程内路径不虚高, 在跨进程路径上会不会慢是推测。

## 测试钩子

靠启动参数触发, UI 测试和手动验证用, 见 `TestHooks.swift`。

| 参数 | 作用 |
|---|---|
| `-daemon-host H -daemon-port P` | 把这个 Daemon 播种进发现列表, 不启动 Bonjour (启动后 `BonjourBrowser` 会清掉不在浏览结果里的条目) |
| `-paired-test H P TOKEN` | 内存里的已配对状态, 不碰 Keychain |
| `-forget-pairing` | 启动时先清掉 Keychain 里的配对 |
| `-pick-folder DIR` / `-pick-pdf FILE` | `files.pick` / `pdf.pick` 不弹选择器, 直接从这里取文件 |
| `-auto-import-folder DIR` | TSX 起来后模拟一次拖放: 克隆文件, 包成 `NSItemProvider` 走真实的读取路径, 再发 `files.added`; 加 `-auto-import-urls` 改走文件选择器的 URL 路径 |
| `-debug-drag-source DIR` | 窗口左下角放一个可拖的小方块, UI 测试用它发起真正的拖放 |
| `-section jobs\|reader\|config` | 起始分区 |
| `-bundle FILE.js` | 加载这个 bundle 而不是内置的。字节码 (`.nshc`) 只能从 App 自己的资源加载, 外部文件要用 `bundle.js` |

`ImageTranslatorUITests/NshHostUITests.swift` 需要一个在跑的测试 Daemon, 用环境变量 `TEST_RUNNER_HOST_DAEMON_HOST`、`TEST_RUNNER_HOST_DAEMON_PORT`、`TEST_RUNNER_HOST_DAEMON_KEY`、`TEST_RUNNER_HOST_PICK_FOLDER` 传进来。Bonjour 那个用例要先在 Mac 上 `dns-sd -R <名字> _imagetranslator._tcp local <端口>` 公布一个实例, 并设 `TEST_RUNNER_HOST_BONJOUR=1`。测试 Daemon 一律用独立的 HOME、TMPDIR 和端口, 并加 `--no-bonjour`。
