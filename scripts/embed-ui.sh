#!/bin/zsh
# Xcode 的 Run Script 阶段调用: 把 client-ui 的原生 bundle 放进 App 的 nsh/ 目录, NshAppView 从那里加载。
# bundle 不存在或 client-ui 源码比它新时, 先用 scripts/build-ui.sh native 重建 (需要 nsh CLI: scripts/nsh-visionos/build.sh)。
# NSH_UI_SKIP_BUILD=1: 只拷现成的 bundle, 不检查是否过期 (别人正在改 client-ui 时用)。
set -euo pipefail

HERE=${0:A:h}
REPO=${HERE:h}
UI=$REPO/client-ui
DIST=$UI/dist/native

die() { print -r -- "error: [embed-ui] $*" >&2; exit 1 }

stale() {
  [[ -f $DIST/bundle.nshc ]] || return 0
  [[ -n "$(find $UI/App.tsx $UI/nsh.json $UI/theme.json $UI/src $UI/web -type f -newer $DIST/bundle.nshc 2>/dev/null | head -1)" ]]
}

if [[ ${NSH_UI_SKIP_BUILD:-0} != 1 ]] && stale; then
  [[ -x $HERE/nsh-visionos/bin/nsh ]] || die "client-ui 的 bundle 需要重建, 但没有 nsh CLI: 先运行 scripts/nsh-visionos/build.sh"
  # Xcode 给 Run Script 的环境变量 (SDKROOT 等) 会干扰 nsh, 用干净环境。
  env -i HOME=$HOME PATH=/usr/bin:/bin:/usr/sbin:/sbin $HERE/build-ui.sh native
fi
[[ -f $DIST/bundle.nshc ]] || die "没有 $DIST/bundle.nshc, 先运行 scripts/build-ui.sh native"

DEST=$TARGET_BUILD_DIR/$UNLOCALIZED_RESOURCES_FOLDER_PATH/nsh
rm -rf $DEST
mkdir -p $DEST
for name in bundle.nshc app.json capabilities.json; do
  [[ -e $DIST/$name ]] && cp $DIST/$name $DEST/
done
[[ -d $DIST/assets ]] && cp -R $DIST/assets $DEST/
print -r -- "[embed-ui] bundle.nshc sha256 $(shasum -a 256 $DEST/bundle.nshc | cut -c1-16) -> $DEST"
