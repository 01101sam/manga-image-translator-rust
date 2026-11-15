#!/bin/zsh
# 构建 NativeShell.xcframework (ios / ios-sim / xros / xros-sim, 可选 catalyst) 与 nsh CLI。
# 用法: scripts/nsh-visionos/build.sh [nsh 源码目录]
# 环境变量:
#   NSH_DIR             同第一个参数, 默认 ~/.cache/image-translator/nsh
#   IOS_MIN / XROS_MIN  部署目标, 默认 17.0 / 1.0 (与 app 工程一致)
#   NSH_CATALYST=1      额外构建 Mac Catalyst 切片
#   NSH_SKIP_CLI=1      不构建 nsh CLI
set -euo pipefail

HERE=${0:A:h}
REPO=${HERE:h:h}
NSH_DIR=${1:-${NSH_DIR:-$HOME/.cache/image-translator/nsh}}
IOS_MIN=${IOS_MIN:-17.0}
XROS_MIN=${XROS_MIN:-1.0}
PATCH=$HERE/visionos-cfg.patch
source $HERE/common.sh
OUT=$REPO/app/Frameworks/NativeShell.xcframework
CLI_OUT=$HERE/bin/nsh
WORK=$NSH_DIR/target/image-translator

# 一个切片: rust 目标 -> 一个 framework。
# 参数: 切片名 rust目标 sdk swift目标 Info.plist平台名 最低版本 [versioned]
build_slice() {
  local id=$1 rust=$2 sdk=$3 triple=$4 plat=$5 min=$6 versioned=${7:-}
  local crate=$NSH_DIR/crates/nsh-platform-ios
  local w=$WORK/$id sdkp mod
  sdkp=$(xcrun --sdk $sdk --show-sdk-path)
  rm -rf $w; mkdir -p $w/swift
  log "[$id] cargo rustc $rust"
  (cd $NSH_DIR && IPHONEOS_DEPLOYMENT_TARGET=$IOS_MIN XROS_DEPLOYMENT_TARGET=$XROS_MIN \
    cargo rustc --release -p nsh-platform-ios --target $rust --features embed --crate-type staticlib)

  # swiftc 按目标给模块文件命名 (去掉版本号), 与 xtask 的 compile_swift 一致。
  mod=$(print -r -- $triple | sed -E 's/(ios|xros)[0-9.]+/\1/')
  local -a swiftc=(xcrun swiftc -parse-as-library -module-name NativeShell -target $triple -sdk $sdkp
    -swift-version 5 -enable-library-evolution -O -whole-module-optimization)
  if [[ -n $versioned ]]; then
    # Mac Catalyst 的 UIKit 在 macOS SDK 的 iOSSupport 下。
    swiftc+=(-Fsystem $sdkp/System/iOSSupport/System/Library/Frameworks -I $sdkp/System/iOSSupport/usr/lib/swift)
  fi
  $swiftc -emit-library -static -emit-module-path $w/swift/$mod.swiftmodule \
    -emit-module-interface-path $w/swift/$mod.swiftinterface -I $crate/include \
    $crate/swift/NativeShell.swift -o $w/swift/libNativeShellSwift.a

  # 只导出 C ABI, 其余符号变 local, 免得与 app 里其它 Rust 静态库冲突。
  print -r -- '_nsh_*' > $w/nsh.exports
  xcrun --sdk $sdk clang -r -nostdlib -target $triple -isysroot $sdkp \
    -Wl,-force_load,$NSH_DIR/target/$rust/release/libnsh_platform_ios.a \
    -Wl,-exported_symbols_list,$w/nsh.exports -o $w/nsh-runtime.o
  xcrun libtool -static -no_warning_for_no_symbols -o $w/NativeShell $w/nsh-runtime.o $w/swift/libNativeShellSwift.a

  local fw=$w/NativeShell.framework contents res f
  if [[ -n $versioned ]]; then contents=$fw/Versions/A; res=$contents/Resources; else contents=$fw; res=$fw; fi
  mkdir -p $res $contents/Modules/NativeShell.swiftmodule
  cp $w/NativeShell $contents/NativeShell
  for f in $w/swift/*.(swiftinterface|swiftmodule|swiftdoc); do
    [[ $f == *.private.swiftinterface ]] || cp $f $contents/Modules/NativeShell.swiftmodule/
  done
  cat > $res/Info.plist <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>NativeShell</string>
<key>CFBundleIdentifier</key><string>com.longbridge.nsh.NativeShell</string>
<key>CFBundleName</key><string>NativeShell</string>
<key>CFBundlePackageType</key><string>FMWK</string>
<key>CFBundleShortVersionString</key><string>$NSH_VERSION</string>
<key>CFBundleVersion</key><string>$NSH_VERSION</string>
<key>CFBundleSupportedPlatforms</key><array><string>$plat</string></array>
<key>MinimumOSVersion</key><string>$min</string>
</dict></plist>
EOF
  if [[ -n $versioned ]]; then
    ln -s A $fw/Versions/Current
    ln -s Versions/Current/NativeShell $fw/NativeShell
    ln -s Versions/Current/Modules $fw/Modules
    ln -s Versions/Current/Resources $fw/Resources
  fi
  FRAMEWORKS+=($fw)
}

command -v xcrun >/dev/null || { log "需要 Xcode (xcrun)"; exit 1 }
sync_nsh

# 视觉 OS 的 rust 目标不在 rust-toolchain.toml 里, 缺了就停下, 安装由人确认。
installed=${(f)"$(cd $NSH_DIR && rustup target list --installed)"}
for t in aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-visionos aarch64-apple-visionos-sim ${NSH_CATALYST:+aarch64-apple-ios-macabi}; do
  (( ${installed[(I)$t]} )) || { log "缺 rust 目标: rustup target add $t"; exit 1 }
done

NSH_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' $NSH_DIR/Cargo.toml | head -1)
typeset -a FRAMEWORKS=()
build_slice ios-arm64             aarch64-apple-ios          iphoneos        arm64-apple-ios$IOS_MIN             iPhoneOS        $IOS_MIN
build_slice ios-arm64-simulator   aarch64-apple-ios-sim      iphonesimulator arm64-apple-ios$IOS_MIN-simulator   iPhoneSimulator $IOS_MIN
build_slice xros-arm64            aarch64-apple-visionos     xros            arm64-apple-xros$XROS_MIN           XROS            $XROS_MIN
build_slice xros-arm64-simulator  aarch64-apple-visionos-sim xrsimulator     arm64-apple-xros$XROS_MIN-simulator XRSimulator     $XROS_MIN
if [[ -n ${NSH_CATALYST:-} ]]; then
  build_slice ios-arm64-maccatalyst aarch64-apple-ios-macabi macosx arm64-apple-ios$IOS_MIN-macabi MacOSX $IOS_MIN versioned
fi

stage=$WORK/NativeShell.xcframework
rm -rf $stage
args=()
for fw in $FRAMEWORKS; do args+=(-framework $fw); done
xcodebuild -create-xcframework $args -output $stage
mkdir -p ${OUT:h}
rm -rf $OUT
mv $stage $OUT
log "xcframework: $OUT"

if [[ -z ${NSH_SKIP_CLI:-} ]]; then
  log "cargo build nsh CLI"
  (cd $NSH_DIR && cargo build --release -p nsh-cli)
  mkdir -p ${CLI_OUT:h}
  cp $NSH_DIR/target/release/nsh $CLI_OUT
  log "nsh CLI: $CLI_OUT"
fi
