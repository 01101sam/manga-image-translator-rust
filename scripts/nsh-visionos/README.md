# NSH 构建链 (iOS / visionOS)

把固定版本的 [NSH](https://github.com/longbridge/nsh) 构建成 app 要链接的 `NativeShell.xcframework`, 并同一 commit 构建 `nsh` CLI。

## 为什么要补丁

上游 `e264950` 的 UIKit 后端 (`nsh-platform-ios`、`nsh-platform-apple`、`nsh-core` 的图像预算与字体目录) 只认 `target_os = "ios"`, rustc 的 visionOS 目标是 `target_os = "visionos"`, 不打补丁时 `nsh-platform-ios` 在 visionOS 目标上整个 crate 被 cfg 掉, 产出的静态库没有 `_nsh_*` 符号。`visionos-cfg.patch` 把 `visionos` 并进 `ios` 的 cfg 分支, 另外修了运行时字符串匹配:

- `nsh-engine/src/storage.rs` 的数据目录与缓存目录: visionOS 与 iOS 一样用 `~/Library/Application Support` 和 `~/Library/Caches`, 否则落到 Linux 分支的 `~/.local/share`。
- `nsh-updater/src/lib.rs` 的 `platform()`: visionOS 取发布清单里的 `ios` 条目。
- `nsh-core/src/runtime/os.rs` 的 `PLATFORM` (`os.platform()` 与 `process.platform`): rustc 的 `std::env::consts::OS` 在 visionOS 上是 `"visionos"`, 补丁让它和 `platform.os` (UIKit 后端写死 `Os::Ios`) 一致, 都是 `"ios"`。实测 (visionOS 模拟器, 探针 bundle): 未打这一处补丁时 `platform.os=ios`、`process.platform=visionos`; 打了以后两个都是 `ios`。`ios:` 类名与 `platform.select({ ios })` 本来就按 `platform.os` 走, 不受这个补丁影响; 受影响的只是读 `process.platform` 或 `os.platform()` 的代码。

## 运行

```bash
scripts/nsh-visionos/build.sh [nsh 源码目录]   # 默认 ~/.cache/image-translator/nsh
scripts/nsh-visionos/verify.sh                 # 检查每个切片的平台、minos, 并各做一次 swiftc 链接
```

- 首次运行会 shallow fetch 固定 commit (约 20 MB) 并下载 crates, 冷编译实测四个切片约 29 分钟 (含下载), CLI 约 10 分钟; 之后重跑只在补丁或源码变化时重编。
- 共用的版本号与「整理成固定 commit 加补丁」在 `common.sh`, `scripts/build-ui.sh` 也读它, 保证 CLI、xcframework 与 Web runtime 出自同一份源码。脚本幂等: 源码目录已是固定 commit 加补丁时不动文件; 否则 reset、clean 后重新打补丁。目录必须是空的, 或 origin 为 NSH 仓库。
- 需要 Xcode 和 rustup 目标 `aarch64-apple-ios`、`-ios-sim`、`-visionos`、`-visionos-sim`。缺了脚本会停下并打印 `rustup target add` 命令, 不会自动安装。
- 可选环境变量: `IOS_MIN` (默认 17.0)、`XROS_MIN` (默认 1.0), 与 `app/ImageTranslator.xcodeproj` 的部署目标一致; `NSH_CATALYST=1` 多构建一个 Mac Catalyst 切片 (app 目前不支持 macOS, 默认不构建); `NSH_SKIP_CLI=1` 跳过 CLI。

## 产物

| 产物 | 位置 |
|---|---|
| `NativeShell.xcframework` (ios-arm64、ios-arm64-simulator、xros-arm64、xros-arm64-simulator) | `app/Frameworks/` (已 gitignore) |
| `nsh` CLI | `scripts/nsh-visionos/bin/nsh` (已 gitignore) |

静态 framework, Swift 模块自带 autolink, 链接时不用额外的 `-framework` 或 `-l` 参数。升级 NSH 时改 `build.sh` 里的 `NSH_COMMIT` (完整哈希) 并重新生成补丁。
