# Manga Image Translator

漫画图片翻译系统。单个 Rust Daemon 承载全部服务端能力,iPhone / iPad / visionOS 原生 Client 通过局域网使用它。

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

**Queue**:
Job 的有界等待队列。Worker 全忙时 Job 排队,队列满时拒绝新 Job。
_Avoid_: buffer、backlog

**Pairing**:
Client 与 Daemon 建立信任的流程。Daemon 控制台显示 6 位数字,30 秒内在 Client 中输入即完成配对。
_Avoid_: login、registration

**Token**:
Pairing 成功后 Daemon 签发的凭证。Client 持久保存,之后每个请求携带;未配对的请求被拒绝。
_Avoid_: password、session id

**Artifact**:
Job 的输出文件,存放于 Daemon 所在机器的共享临时文件夹,所有 Client 共享,Daemon 关闭时清理。
_Avoid_: output、result file

**Config**:
Daemon 持有的全局配置,含 DeepSeek API URL/KEY。只能经 WebUI 修改,Client 不持有。
_Avoid_: settings file、preferences

**WebUI**:
Daemon 内嵌的 Web 界面,是修改 Config 的唯一入口。iOS Client 以 WebView 承载它。
_Avoid_: admin page、dashboard

**Client**:
iPhone / iPad / visionOS 原生 App。通过 Bonjour 自动发现 Daemon,不配置 URL。
_Avoid_: app、frontend

**PDF Reader**:
Client 内置的 PDF 阅读器。PDFKit 在本地渲染原文,方向键翻页,空格键切换译文/原文或把未翻译页加入 Queue。Server 不接触 PDF。
_Avoid_: viewer
