# Manga Image Translator

*   [Docs/Install Guide](https://frederik-uni.github.io/manga-image-translator-rust/index.html)
*   [Config file example](example/example.json)
*   [Config file schema](example/schema.json)

## 启动 Daemon

在仓库根目录运行：

```
cargo run --release -p simple-runtime -- daemon
```

Daemon 默认绑定 `0.0.0.0:8080`。没有开机自启。需要时手动启动。

