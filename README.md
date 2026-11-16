# Manga Image Translator

*   [Docs/Install Guide](https://frederik-uni.github.io/manga-image-translator-rust/index.html)
*   [Config file example](example/example.json)
*   [Config file schema](example/schema.json)

## 启动 Daemon

在仓库根目录运行。Daemon 把 Web 版编进二进制，所以先生成 `gui/dist`：

```
scripts/build-ui.sh web
cargo run --release -p simple-runtime -- daemon
```

Daemon 默认绑定 `0.0.0.0:8080`。没有开机自启。需要时手动启动。

浏览器打开 Daemon 地址即可使用 WebUI，首次用 Pairing Key 配对（默认 `sam01101`）。如果本机有其他程序监听 `127.0.0.1:8080`（例如 Cursor），访问 `localhost:8080` 会连到那个程序，此时用 `--port` 换端口。

iOS / iPadOS / visionOS Client 的构建见 [app/README.md](app/README.md)。
