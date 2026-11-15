# ADR 0002. 固定 Pairing Key 配对

## 状态

已接受。替代 ADR 0001 决定 5 里的配对部分（6 位码、30 秒窗口、`/pair/*`）。决定 5 的其余部分仍然有效：WebUI 页面本身不要求登录，除公开路由以外的 API 需要 Bearer。

## 背景

ADR 0001 的 Pairing 要操作者盯着 Daemon 控制台，在 30 秒内把 6 位数字输进设备。Daemon 常在后台运行，控制台看不到。iPhone、iPad、visionOS 和浏览器每台都要重复一遍。窗口、`/pair/request` 和为测试窗口过期而注入的时钟，都只为这个流程存在。

## 决定

1. Config 新增 `pairing_key`，默认 `sam01101`。旧 `config.json` 没有这个字段时取默认值。
2. `POST /pair`，body 是 `{"key": "..."}`。Key 与 Config 一致时返回 200 `{"token": "..."}`，否则返回 401 `{"ok": false, "error": "..."}`。
3. Key 用常量时间比较。空 Key 视为未设置，永远不匹配。`PUT /config` 拒绝空白的 Key。
4. 每次 `POST /pair` 都现读 Config，改 Key 对之后的 Pairing 立即生效。已签发的 Token 不受影响，继续有效。Token 存在 `tokens.json`，Daemon 重启后仍然有效。写入改为先写临时文件再改名，崩溃不会留下截断的文件而让所有设备失去配对。
5. 不限速。操作者明确不要。
6. 删除 `/pair/request`、`/pair/confirm`、6 位码、30 秒窗口、控制台的 `PAIRING <code>` 输出，以及只为窗口存在的 `Clock` 和 `FakeClock`。

## 后果

局域网内知道 Key 的人都能配对并取得 Token。默认值写在源码和文档里，操作者应在 Config 界面改掉。Token 没有单独吊销手段，需要时只能删除 `tokens.json`，这与 ADR 0001 一致。

`gui/index.html` 和 Swift Client 仍调用已删除的 `/pair/request` 与 `/pair/confirm`。在新界面替换它们之前，它们无法对新 Daemon 配对。

## 否决的方案

保留 6 位码并延长窗口。操作者仍要去看控制台。

改 Key 时吊销所有已签发的 Token。改 Key 是为了控制之后的新设备，不该让已配对的设备失效。

给 Token 加过期时间。操作者要求配对成功一次后永久保存。
