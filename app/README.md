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
| `Host/HostFiles.swift` | `ManagedFiles`: 文件 id 到沙盒文件。图片在 tmp, 启动时整个清掉; PDF 在 Application Support, id 是文件名里的 uuid, 超过 30 天没打开才在启动时清掉 (`pdf.open` 凭上次的 id 找回); 交给分享面板的文件放在各自的子目录里, 面板上显示的是干净的文件名 |
| `Host/HostIngest.swift` | 拖放和选择的文件进沙盒。每个 `NSItemProvider` 只读一种表示, 一次只读一个, 目录递归并按自然序排; 只带一个网页链接的拖放会下载, 按应答类型或文件头判断是图片还是 PDF (`Info.plist` 只开了 `NSAllowsLocalNetworking`, 所以 http 链接只有局域网地址能下载, https 不受限) |
| `Host/HostPickers.swift` | 照片 (PHPicker) 和文档选择器 (图片、PDF、文件夹)。选哪个由 TSX 的导入菜单决定 (`files.pick` 的 `source`), 宿主不再弹自己的选择菜单 |
| `Host/HostUpload.swift` | `upload.start`: multipart 先分块写进临时文件, 再 `URLSession.upload(fromFile:)`, 字节不进 JS 也不在内存里拼 `Data` |
| `Host/HostArtifact.swift` | `artifact.save`: 下载到临时文件, 弹系统分享面板 |
| `ShareExtension/` | 分享扩展 (bundle id `com.wisdomworldworkshop.imagetranslator.share`, iOS 与 visionOS): 分享面板里接受图片 (含多张)、PDF、图片网页链接, 只把文件复制进 App Group (`group.com.wisdomworldworkshop.imagetranslator`) 的 `inbox/`, 不解码也不上传, 然后显示「已加入，打开 App 后开始翻译」就结束。App 在启动和回到前台时把收件箱搬进沙盒 |
| `Shared/ShareInbox.swift` | App 与扩展共用的收件箱约定, 和扩展的拷贝逻辑 `ShareDepositor`: 串行、一次一个文件, 用 `copyItem` 按文件拷 (不把内容读进内存, 所以扩展约 120 MB 的内存上限与文件大小无关); 先拷进 `.staging/`, 拷完整个目录原子改名成条目, App 不会搬走拷了一半的文件; 每个条目一个目录, 目录名 (毫秒时间戳、序号、uuid) 的字典序就是分享顺序; 网页链接存成 `link.weblink`, 内容是 URL, 由 App 导入时下载 |
| `Host/HostOpenURL.swift` | 「打开方式」: `Info.plist` 声明 `CFBundleDocumentTypes` (`public.image`、`com.adobe.pdf`) 和 `LSSupportsOpeningDocumentsInPlace = NO` (系统先把文件拷进 App 自己的 `Documents/Inbox`), SwiftUI 的 `onOpenURL` 把 URL 交给 `OpenURLRouter`, 它攒着冷启动时到的 URL、把前后到的合成一批 (多个 PDF 才能「只开第一个」), 再交给宿主走和拖放一样的导入。在自己容器里的文件 (`Documents/Inbox` 的副本) 搬进沙盒并删掉, 容器外的只拷不搬, 不动用户的原文件 |
| `Host/HostPdf.swift` | 原生组件 `it.pdf` (PDFView, 单页, 只渲染当前页)、`pdf.renderPage` (CGContext 画到长边, 直接写 PNG 文件) 和 `pdf.export` (译文页换进原 PDF, 逐页下载逐页写) |

旧的 SwiftUI 界面 (`RootView`、`JobListView`、`PDFReaderView`、`AppSession`、`ConfigWebView` 等) 和它们的测试已经删除, 旧功能在新实现里的位置见文末「旧功能对照」。

## 内存

- 图片字节不进 JS (嵌入式 NSH 的 JS 堆上限 256 MB); 列表和查看器显示 Daemon 降采样后的 JPEG (`?w=`)。
- 上传、拖放、选择都走文件, 宿主不把图片读进内存。`NSItemProvider` 的文件表示由系统在回调里给一个临时文件, 宿主在回调里把它搬进沙盒。
- NSH 的图片缓存在磁盘上限 256 MB。
- 拖放摄入一次只读一个 provider。iPad 模拟器里用进程内 `NSItemProvider(contentsOf:)` 读 30 张 8000x12000 的 PNG (每张 37 MB): 并发 2 或 4 时 RSS 峰值 1.37 GB (4 次实测 1364 到 1374 MB), 并发 1 时 332、366、357 MB (3 次); 文件选择器的 URL 路径 (`FileManager.copyItem`, APFS 克隆) 是 295 MB。三种情况的 phys_footprint 都是 23 到 24 MB, 所以 jetsam 看的内存没变。并发 2 时用 `vmmap` 看, 多出来的常驻内存在只读的系统库区域 (`__LINKEDIT` 约 778 MB、`__TEXT` 约 493 MB), 不在文件映射里 (`mapped file` 只有 11 MB 常驻), 所以机制没查清, 也只在模拟器里见过; `HostFilesTests.testProviderIngestOfBigFilesKeepsResidentMemoryNearOneFile` 断言的就是这个模拟器里的 RSS 现象。`loadInPlaceFileRepresentation` 在并发 4 和 1 时的峰值与 `loadFileRepresentation` 一样, 没有采用。200 张 1.7 MB 的 JPEG 并发 1 和 4 的耗时都是 0.9 秒。真实跨应用拖放走系统的进程间拷贝, 没有测过, 并发 1 在那条路径上会不会慢是推测。限流要罩住整个 `load`, 连类型判断也在内 (只限住读文件那一步时, 同样的测试 RSS 又涨回 240 MB); 所以网页链接的下载也占这一个名额, 一个慢链接会拖住后面的拖放和选择, 这是已知的限制。
- PDF 导出 (`pdf.export`) 的内存随被拷贝的原页数线性增长, 没有上限。实测 (iPad 模拟器, 单元测试进程): 100 页、131 MB 的图片 PDF 换 10 页译文, 5 秒, 峰值 footprint 364 MB (起点 40 MB, 约每页 3 MB), 输出 118 MB; 24 页每页一张不同的 2400x3200 噪声译文图时 RSS +101 MB。CoreGraphics 的 PDF 上下文在 `closePDF` 之前一直持有已写页的资源。试过两种替代, 都没有更好, 所以保留最简单的写法: 每 20 页换一个上下文再用 PDFKit 合并, 写的时候 footprint 在 60 到 130 MB, 合并时峰值 429 MB; 用 PDFKit 整本换页, 峰值 350 MB。几百页以上的大图片 PDF 导出可能占到 GB 级, 这是已知的限制。手动复现: 设 `TEST_RUNNER_HOST_MANGA_PDF` 跑 `HostFilesTests.testExportOfARealImagePDFReportsItsMemory`。

## 测试钩子

靠启动参数触发, UI 测试和手动验证用, 见 `TestHooks.swift`。

| 参数 | 作用 |
|---|---|
| `-daemon-host H -daemon-port P` | 把这个 Daemon 播种进发现列表, 不启动 Bonjour (启动后 `BonjourBrowser` 会清掉不在浏览结果里的条目) |
| `-paired-test H P TOKEN` | 内存里的已配对状态, 不碰 Keychain |
| `-forget-pairing` | 启动时先清掉 Keychain 里的配对 |
| `-pick-folder DIR` / `-pick-pdf FILE` | `files.pick` / `pdf.pick` 不弹选择器, 直接从这里取文件 |
| `-inbox-dir DIR` | 分享扩展的收件箱换成这个目录 (不碰 App Group 容器里用户自己的东西), 里面按 `ShareInbox` 的格式放条目; UI 测试用 |
| `-seed-pasteboard FILE\|DIR` | 启动时 App 自己把这个文件 (目录则取里面的图片和 PDF) 放进系统剪贴板; 是自己写的, 读的时候系统不弹「允许粘贴」, UI 测试靠它验证粘贴 |
| `-auto-import-folder DIR` | TSX 起来后模拟一次拖放: 克隆文件, 包成 `NSItemProvider` 走真实的读取路径, 再发 `files.added`; 加 `-auto-import-urls` 改走文件选择器的 URL 路径 |
| `-debug-drag-source DIR` | 窗口左下角放一个可拖的小方块, UI 测试用它发起真正的拖放 |
| `-section jobs\|reader\|config` | 起始分区 |
| `-bundle FILE.js` | 加载这个 bundle 而不是内置的。字节码 (`.nshc`) 只能从 App 自己的资源加载, 外部文件要用 `bundle.js` |

`ImageTranslatorUITests/NshHostUITests.swift` 需要一个在跑的测试 Daemon (Engine 要是 running, 翻译用例才会完成), 用环境变量 `TEST_RUNNER_HOST_DAEMON_HOST`、`TEST_RUNNER_HOST_DAEMON_PORT`、`TEST_RUNNER_HOST_DAEMON_KEY`、`TEST_RUNNER_HOST_PICK_FOLDER` 传进来; 阅读器、配置页和分区切换的用例还要 `TEST_RUNNER_HOST_TOKEN` (先 `POST /pair` 取得) 和 `TEST_RUNNER_HOST_PICK_PDF`。Daemon 上可能留着之前的 Job, 用例按增量数, 不假设列表是空的。Bonjour 那个用例要先在 Mac 上 `dns-sd -R <名字> _imagetranslator._tcp local <端口>` 公布一个实例, 并设 `TEST_RUNNER_HOST_BONJOUR=1`。测试 Daemon 一律用独立的 HOME、TMPDIR 和端口, 并加 `--no-bonjour`。
## 分享扩展与收件箱

扩展只管把文件放进 App Group 的 `inbox/`, 其余都在 App 里 (`Host/HostInbox.swift` 的 `InboxDrainer`):

- **何时搬:** App 启动和每次回到前台 (`didBecomeActive`)。TSX 还没读 `session.snapshot` 时 (冷启动) 先攒着事件, 读了之后补发 (见 HOST_PROTOCOL.md)。
- **怎么搬:** 按条目名排序, 每个条目 `importSync(move: true)` 搬进沙盒 (同一个卷上是 rename), 图片走 `files.added` 进待上传并切到任务页, PDF 走 `pdf.added` 打开阅读器 (多个只开第一个, 其余宿主放掉并发 `host.notice`)。网页链接条目 (`link.weblink`) 走拖放链接的同一条下载路径。进程里只有一个 `InboxDrainer` (actor), 多个窗口不会同时搬同一个条目。
- **幂等:** 条目只处理一次, 处理完或处理不了 (不是图片或 PDF) 整个目录都删。搬文件要么还在收件箱要么已在沙盒, 没有中间态; 搬完到删目录之间崩溃留下的是空目录, 下次直接删, 不会重复导入。`.staging/` 里超过一小时的是被杀掉的扩展留下的, 启动时删。
- **没保证到的:** 进了沙盒的图片和拖放的一样, 待上传列表只在 TSX 的内存里, App 在上传前被系统杀掉, 这些图片就没了 (沙盒里的图片启动时清理)。所以「不丢」只到「从收件箱进沙盒」这一步。分享时还没配对也一样: 图片留在待上传, 配对后才上传。
- **HEIC:** 分享给扩展的照片里, 扩展优先挑 PNG、JPEG、WebP 这类 Daemon 认的表示; 照片库只给 HEIC 时, Daemon 会拒绝这张图 (在列表里标红), 目前没有在宿主里转码。
- **测试:** `ShareInboxTests`、`InboxDrainerTests` 是单测; `NshHostUITests` 里用 `-inbox-dir` 测冷启动和回前台的导入; 整条链 (照片 App 分享, 扩展, 收件箱, 打开 App) 的两个用例要 `xcrun simctl addmedia` 给模拟器放照片并设 `TEST_RUNNER_HOST_PHOTOS_SHARE=1`, 因为照片 App 的界面随系统版本变。

## 旧功能对照

步骤 7 删除旧的 SwiftUI 界面前, 逐条对过旧版的 UI 测试和视图。「已补」是对照时发现新实现缺的, 当场补上; 「不再需要」写明原因。旧文件在 git 历史里, 删除它们的提交的上一个提交里能看到。

| 旧功能 | 新实现 | 测试 |
|---|---|---|
| Bonjour 发现 Daemon | `BonjourBrowser.swift` 保留, `Host/HostSession.swift` 推送 `session.daemons`; TSX `PairingScreen` | `NshHostUITests.testBonjourDiscoversTheDaemonThenPairs`、`EndpointTests` |
| 6 位配对码 (`pairing-code`) | 不再需要: Pairing Key 固定 (默认 sam01101, 存在 Config), 一次成功永久保存 Token, 见 `docs/adr/0002-pairing-key.md` | `app.test.tsx` 的 pairing、`session.test.ts` (旧 `PairingMachineTests` 的选择、确认、重试、401 都对应在这里) |
| Token 跨重启 (Keychain) | `TokenStore.swift` 保留, `HostSession` 读写; Web 存 NSH Storage | `NshHostUITests.testPairingIsRememberedAcrossRelaunch`、`HostUploadTests.testTokenRoundTripsThroughSaveAndSnapshot` |
| 更换 Daemon、401 清 Token | 配置页「取消配对」(`changeDaemon`)、401 触发 `session.clearToken` | `app.test.tsx` 的 "a revoked Token…" 和 "forgetting the Daemon…" |
| 添加图片: 相册、文件、文件夹 | TSX 的导入菜单 (`ImportMenu`) + `files.pick` 的 `source` + `Host/HostPickers.swift`; 文件夹含在「从文件」里, 递归并按自然序 | `HostFilesTests.testFolderIngest…`、`NshHostUITests.testPickedImagesUploadFinishAndOpenInViewer` (旧的 `testFolderBatchImport` 也并在这里)、`testImportMenuOpensListsTheHostsSourcesAndClosesOnATapOutside` |
| 拖放文件、照片、PDF | `Host/HostIngest.swift`, 每个 provider 只读一种表示, 一次只读一个 | `HostFilesTests` 的 provider 摄入、类型过滤、限流用例、`NshHostUITests.testDraggingFilesOntoTheWindowAddsThem` (旧 `DropRoutingTests` 的路由判断随之换成 provider 的类型判断) |
| 打开方式 (旧版没有) | 新增: `Info.plist` 的文档类型加 `Host/HostOpenURL.swift`。实测 (iPad 模拟器): `xcrun simctl openurl` 只会交给默认的「预览」, 文件 App 的「打开方式」子菜单里也只有「预览」(把 `LSSupportsOpeningDocumentsInPlace` 改成 YES 也一样), 所以模拟器里没法从系统菜单选到本 App; 用 `XCUIApplication.open(url)` 让系统把 URL 交给 App, 验证 `onOpenURL` 到导入这一段。真机上是否出现在系统菜单里没有测过 | `OpenURLTests`、`NshHostUITests.testAFileOpenedWithTheAppIsImportedAndTheOriginalIsLeftAlone` |
| 分享扩展 (旧版没有) | 新增: `ShareExtension/` 与 `Shared/ShareInbox.swift`, App 一侧是 `Host/HostInbox.swift` | `ShareInboxTests`、`InboxDrainerTests`、`NshHostUITests.testSharedImagesInTheInboxAreImportedAtLaunchAndAgainWhenTheAppComesBack`、`testAPhotoSharedFromPhotosShowsUpInTheAppAsAPendingUpload` |
| 粘贴 (旧版没有) | 新增: 导入菜单的「粘贴」和 ⌘V → `files.paste` (`NshHostViewController.paste`), 剪贴板里的图片和 PDF 按文件表示读, 不解码; 内容来自别的 App 时系统会问一次「允许粘贴」 | `PasteboardIngestTests`、`NshHostUITests.testPasteFromTheMenuAndWithCommandVAddsTheClipboard`、`app.test.tsx` 的 import menu |
| 拖放网页链接 (`testWebURLToImage`、`testWebURLToPDF`) | 已补: `HostIngest.loadWebLink` 下载后按应答类型或文件头分图片和 PDF | `HostFilesTests.testDroppedWebLinksAreDownloadedAndRoutedByWhatTheyAre`、`testWebType…` |
| 导入结果横幅 | `host.notice` 事件, TSX 用 toast 显示「已忽略 N 个不是图片或 PDF 的文件」; 不再列出无法识别的类型标识 | `HostFilesTests.testProviderIngestIgnoresKindsTheCallerDidNotAllow` |
| Job 列表: 状态、排队位置、错误、批量提交、Pending Upload、队列容量 | `JobsPage`、`JobRow`、`JobTile`、`store/jobs.ts`、`domain/board.ts` (虚拟化 List, 批量轮询, iPad 与 visionOS 是网格) | `app.test.tsx` 的 jobs 与 grid、`board.test.ts`、`job.test.ts` |
| 取消排队中的 Job (`testCancelQueuedJob`) | 行上的「取消」 (`cancelItem`) | `app.test.tsx` 的 "a queued job can be cancelled…"。不再做设备上的用例: 它要把 workers 设成 0 再重启 Engine, 取消只走 Daemon API, 对宿主没有增量 |
| 删除、重排 (`EditButton`、`onDelete`、`onMove`) | 删除: 行上「移除」、「清除已结束」、Backspace。重排不再需要: 只改本地显示顺序, 与 Daemon 的队列无关 | `app.test.tsx` 的 "a queued job can be cancelled…" 与 "Backspace…" |
| 键盘移动、回车预览 | `keymap` 与 `domain/nav.ts` (网格里上下一行, 左右一格) | `nav.test.ts`、`app.test.tsx` 的 viewer and keyboard 与 grid |
| 预览译文与原文、上一张下一张、关闭 (`testPreviewClose`) | `Viewer.tsx`, 加载 w=2048 的降采样图 | `app.test.tsx` 的 viewer and keyboard、`NshHostUITests.testPickedImagesUploadFinishAndOpenInViewer` (已补关闭) |
| 分享译图 (`ShareLink`) | `artifact.save` → `Host/HostArtifact.swift`, 存原始大小的文件 | `HostUploadTests.testArtifactExtensionFollowsContentType`、`app.test.tsx` 的 "saving asks the host for the full-size file…" |
| 缩略图、把 Artifact 整个读进内存 | 不再需要: Daemon 的 `?w=256` 与 `?w=512` 预览加 NSH `Image` 的降采样缓存, 字节不进 JS | `media.test.ts` |
| Engine 提示条 | `Banner` (任务页和阅读器) 与配置页的 Engine 卡片 | `app.test.tsx` 的 "a stopped Engine holds uploads…" |
| 打开 PDF、PDFView 单页 | `pdf.pick` / `pdf.added`; App 用 `it.pdf` (`PdfComponent`), Web 用 pdf.js | `NshHostUITests.testReaderOpensAPdfTurnsPagesAndTranslatesOne`、`reader-store.test.ts` |
| 方向键翻页、空格翻译或切换 | `ReaderPage.tsx` 的 `keymap`; `domain/reader.ts` 是页状态机 | `reader.test.ts` (前六个用例逐个移植自 `PageMachineTests`)、`reader-app.test.tsx` |
| 把页送去翻译: 整页 300 dpi JPEG 读进内存 | `pdf.renderPage`: PNG, 长边取 Config 的 `downscale_long_edge`, 直接写文件 | `HostFilesTests.testRenderPage…` (包括新加的整页铺满像素检查) |
| 翻译全部 | `store/reader.translateAll`: 一次渲染一页, 已渲染未上传最多 5 页 | `reader-app.test.tsx` 的 "translate all renders only a few pages ahead…" |
| 导出 PDF (`assembleExport`) | 已补: `pdf.export`, 后台线程逐页下载译文、降采样到页面点尺寸的 2 倍、写进 PDF, 再弹分享面板。Web 不支持导出 (没有不加依赖的低成本办法), 不显示按钮 | `HostFilesTests.testExport…`、`reader-app.test.tsx`、`NshHostUITests.testReaderOpens…` (确认分享面板弹出) |
| 存图到相册 | 已补: 阅读器页脚「保存译图」走 `artifact.save`, 分享面板里有「存储图像」 | `reader-app.test.tsx` 的 "the translated page can be saved…" |
| 重启后回到上次的 PDF | 已补 (旧版也没有): `pdf.open` 加 Storage 的 `reader.last` | `reader-app.test.tsx` 的 export and restoring the last PDF |
| 配置页 (WKWebView 载入 Daemon 的网页) | `ConfigPage.tsx`: TSX 表单, 与 Web 同一份, 含 Pairing Key 和 Engine 启停 | `app.test.tsx` 的 config、`config.test.ts`、`NshHostUITests.testConfigPageShowsTheDaemonSettings` (已补) |
| 分区切换压力 (`testTabSwitchStress`, 50 轮) | 已补: 30 轮 | `NshHostUITests.testSwitchingSectionsRepeatedlyStaysResponsive` |
| visionOS 窗口横竖屏 | `WindowSettings.swift` 与 `ImageTranslatorApp.swift` 保留 | `WindowSettingsTests` |
| 测试钩子 `-open-test-pdf`、`-auto-submit-image` 与打进 App 的 sample.jpg、three-pages.pdf、batch | 不再需要: 新测试用 `-pick-folder`、`-pick-pdf` 取绝对路径的文件; 钩子和资源已删 | `TestHooksTests` |
| `DaemonClient`、`MockURLProtocol`、`Domain` 里的状态机与拖放路由 | 不再需要: HTTP 全在 TSX (`api/daemon.ts`), 测试用 NSH 的 `network` 桩 (`tests/fake-daemon.ts`); `Domain.swift` 只留 `DaemonEndpoint`、`Token`、`PairedDaemon` | 同上各行 |
