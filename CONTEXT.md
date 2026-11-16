# Manga Image Translator

漫画图片翻译系统。单个 Rust Daemon 承载全部服务端能力,iPhone / iPad / visionOS 原生 Client 与浏览器中的 WebUI 通过局域网使用它。

## Language

**Daemon**:
唯一的服务器进程,承载 HTTP API、WebUI、配置、队列与 Worker 池。手动启动,无开机自启。
_Avoid_: server、service、webui-server

**Engine**:
Daemon 内部加载模型、执行翻译 pipeline 的核心。WebUI 的启动/关闭/重启作用于 Engine,Daemon 本身始终存活。
_Avoid_: worker pool、backend

**Worker**:
Engine 内一个并发翻译执行槽,独占一份模型实例。数量按本机资源实测确定,可配置。
_Avoid_: thread、process

**Job**:
一张图片或 PDF 一页的翻译任务,是队列与调度的最小单位。生命周期为 queued → running → done/failed,queued 状态下可被 Client 取消为 cancelled。
_Avoid_: task、request

**Pending Upload**:
Client 已接收、尚未提交给 Daemon 的图片。Queue 满时图片停在这里,按顺序成为 Job。它不属于 Job 的生命周期,取消它不涉及 Daemon。
_Avoid_: queued、draft

**Inbox**:
Client 的 Share Extension 放文件的地方 (App Group 里的一个目录)。App 在启动和每次回到前台时把它搬进自己的沙盒: 图片成为 Pending Upload, PDF 在 PDF Reader 里打开。扩展只放文件, 不解码、不上传。
_Avoid_: 队列、草稿箱

**Queue**:
Job 的有界等待队列。Worker 全忙时 Job 排队,队列满时拒绝新 Job。
_Avoid_: buffer、backlog

**Pairing**:
Client 或浏览器与 Daemon 建立信任的流程。提交的 Pairing Key 与 Daemon 持有的一致即完成配对,每台设备只需成功一次。
_Avoid_: login、registration

**Pairing Key**:
Daemon 持有的配对密钥,属于 Config,可在 Config 界面修改。修改它只影响之后的 Pairing,已签发的 Token 继续有效。
_Avoid_: password、pairing code

**Token**:
Pairing 成功后 Daemon 签发的凭证。Client 或浏览器永久保存,之后每个请求携带;未配对的请求被拒绝。
_Avoid_: password、session id

**Artifact**:
Job 的输出文件,存放于 Daemon 所在机器的共享临时文件夹,所有 Client 共享,Daemon 关闭时清理。
_Avoid_: output、result file

**Config**:
Daemon 持有的全局配置,含 DeepSeek API URL/KEY 与 Pairing Key。只能经 Config 界面修改,Client 不持有。
_Avoid_: settings file、preferences

**WebUI**:
Daemon 向浏览器提供的界面。它与 Client 共用同一套界面代码,Config 界面在两处相同。
_Avoid_: admin page、dashboard

**Client**:
iPhone / iPad / visionOS 原生 App。通过 Bonjour 自动发现 Daemon,不配置 URL。
_Avoid_: app、frontend

**PDF Reader**:
Client 与 WebUI 内置的 PDF 阅读器。PDF 只在本地打开和渲染,方向键翻页,空格键切换译文/原文或把未翻译页加入 Queue。Client 可把译文页导出为新 PDF。Daemon 不接触 PDF,只接收渲染好的单页图片。
_Avoid_: viewer