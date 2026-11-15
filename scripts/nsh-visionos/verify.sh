#!/bin/zsh
# 检查 NativeShell.xcframework 的每个切片: 平台与 minos 是否整齐, 以及最小 swiftc 程序能否链接。
# 用法: scripts/nsh-visionos/verify.sh [xcframework 路径]
set -euo pipefail

HERE=${0:A:h}
REPO=${HERE:h:h}
XCF=${1:-$REPO/app/Frameworks/NativeShell.xcframework}
IOS_MIN=${IOS_MIN:-17.0}
XROS_MIN=${XROS_MIN:-1.0}

# 切片名 -> "LC_BUILD_VERSION 平台号 minos swift目标 sdk"
typeset -A EXPECT=(
  ios-arm64                "2 $IOS_MIN arm64-apple-ios$IOS_MIN iphoneos"
  ios-arm64-simulator      "7 $IOS_MIN arm64-apple-ios$IOS_MIN-simulator iphonesimulator"
  xros-arm64               "11 $XROS_MIN arm64-apple-xros$XROS_MIN xros"
  xros-arm64-simulator     "12 $XROS_MIN arm64-apple-xros$XROS_MIN-simulator xrsimulator"
  ios-arm64-maccatalyst    "6 $IOS_MIN arm64-apple-ios$IOS_MIN-macabi macosx"
)

tmp=$(mktemp -d)
trap 'rm -rf $tmp' EXIT
cat > $tmp/main.swift <<'EOF'
import UIKit
import NativeShell

@MainActor func make() -> UIView { NshAppView(frame: .zero) }
// 永远不会执行, 只让链接器必须解析 NativeShell 的符号。
if CommandLine.arguments.count > 99 { MainActor.assumeIsolated { _ = make() } }
EOF

fail=0
for dir in $XCF/*(/N); do
  id=${dir:t}
  [[ -n ${EXPECT[$id]:-} ]] || { print "FAIL $id: 未知切片"; fail=1; continue }
  want=(${=EXPECT[$id]})
  bin=$dir/NativeShell.framework/NativeShell
  # 归档里每个目标文件各有一份 LC_BUILD_VERSION; 去重后应只剩一对, 且与期望一致。
  pairs=$(xcrun otool -l $bin | awk '/^ +platform /{p=$2} /^ +minos /{print p, $2}' | sort -u)
  archs=$(xcrun lipo -archs $bin)
  if [[ $pairs == "$want[1] $want[2]" ]]; then
    print "OK   $id: archs=$archs platform=$want[1] minos=$want[2]"
  else
    print "FAIL $id: archs=$archs 期望 '$want[1] $want[2]', 实际:"; print -r -- $pairs | sed 's/^/       /'; fail=1
    continue
  fi
  [[ $id == *maccatalyst ]] && continue
  sdkp=$(xcrun --sdk $want[4] --show-sdk-path)
  # 链接器的 minos 不一致只是 warning, 退出码为 0, 所以要单独查 stderr。
  if xcrun swiftc -target $want[3] -sdk $sdkp -F $dir -framework NativeShell $tmp/main.swift -o $tmp/link-$id 2> $tmp/link-$id.err && ! grep -q 'ld: warning' $tmp/link-$id.err; then
    print "OK   $id: swiftc 链接通过, $(( $(stat -f%z $tmp/link-$id) / 1048576 )) MiB, $(xcrun vtool -show-build $tmp/link-$id | awk '/platform|minos/{printf "%s ", $1 " " $2}')"
  else
    print "FAIL $id: swiftc 链接失败"; head -20 $tmp/link-$id.err | sed 's/^/       /'; fail=1
  fi
done
exit $fail
