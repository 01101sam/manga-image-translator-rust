#!/bin/zsh
# 构建 client-ui: 同一套 TSX 界面, Web 站点 (gui/dist, 由 Daemon 托管) 与 App 用的原生 bundle。
# 用法: scripts/build-ui.sh [web|native|check|test|all]    默认 web
#   web     NSH web runtime (固定 commit 的源码编成 wasm) + nsh package --target web -> gui/dist, 再把 web/pdfjs 拷成 gui/dist/pdfjs
#   native  nsh build --release -> client-ui/dist/native (bundle.nshc、app.json、capabilities.json), 供 Swift 宿主加载
#   check   nsh check (TypeScript 类型检查)
#   test    nsh test (领域逻辑与整个界面的无头测试)
#   all     check + test + web + native
# 环境变量:
#   NSH_DIR   固定 commit 的 NSH 源码目录, 默认 ~/.cache/image-translator/nsh (由 scripts/nsh-visionos/build.sh 准备)
#   NSH_UI_RUNTIME  web runtime 的产物目录, 默认 ~/.cache/image-translator/nsh-runtime
set -euo pipefail

HERE=${0:A:h}
REPO=${HERE:h}
NSH_DIR=${NSH_DIR:-$HOME/.cache/image-translator/nsh}
RUNTIME=${NSH_UI_RUNTIME:-$HOME/.cache/image-translator/nsh-runtime}
PATCH=$HERE/nsh-visionos/visionos-cfg.patch
NSH=$HERE/nsh-visionos/bin/nsh
UI=$REPO/client-ui
source $HERE/nsh-visionos/common.sh

die() { print -r -- "[build-ui] $*" >&2; exit 1 }
say() { print -r -- "[build-ui] $*" }

[[ -x $NSH ]] || die "没有 nsh CLI: 先运行 scripts/nsh-visionos/build.sh (它会从固定 commit 构建 CLI)"

# 与 xcframework 同一份源码: 必须已经是固定 commit 加补丁, 否则让 build.sh 来整理, 这里不动 NSH 源码。
require_nsh_tree() {
  [[ -d $NSH_DIR/.git ]] || die "没有 NSH 源码: 先运行 scripts/nsh-visionos/build.sh"
  [[ $(git -C $NSH_DIR rev-parse HEAD) == $NSH_COMMIT ]] || die "$NSH_DIR 不在固定 commit ${NSH_COMMIT[1,7]}: 先运行 scripts/nsh-visionos/build.sh"
}

# web runtime: nsh-platform-web 编成 wasm。需要 wasm32 目标和带 wasm 后端的 clang (Apple 自带的 clang 没有)。
build_web_runtime() {
  require_nsh_tree
  local out=$RUNTIME/web
  local stamp="$NSH_COMMIT $(shasum $PATCH | cut -d' ' -f1)"
  if [[ -f $out/.stamp && $(<$out/.stamp) == $stamp && -f $out/nsh-browser.wasm ]]; then
    say "web runtime 已是 ${NSH_COMMIT[1,7]}, 跳过"
    return
  fi
  local clang=${NSH_WASM_CLANG:-/opt/homebrew/opt/llvm/bin/clang}
  local ar=${NSH_WASM_AR:-/opt/homebrew/opt/llvm/bin/llvm-ar}
  [[ -x $clang && -x $ar ]] || die "缺带 wasm 后端的 clang/llvm-ar: brew install llvm, 或用 NSH_WASM_CLANG / NSH_WASM_AR 指定"
  (cd $NSH_DIR && rustup target list --installed | grep -qx wasm32-unknown-unknown) \
    || die "缺 rust 目标: rustup target add wasm32-unknown-unknown"
  say "cargo build nsh-platform-web (wasm32)"
  (cd $NSH_DIR && CC_wasm32_unknown_unknown=$clang AR_wasm32_unknown_unknown=$ar \
    cargo build --profile web -p nsh-platform-web --no-default-features --features browser,linkable --target wasm32-unknown-unknown)
  local syntaxes=(${(f)"$(ls -d $HOME/.cargo/registry/src/*/two-face-*/generated/syntaxes-fancy-newlines.bin 2>/dev/null)"})
  [[ -n ${syntaxes[1]:-} ]] || die "找不到 two-face 的 syntaxes-fancy-newlines.bin (应在 cargo 注册表里)"
  rm -rf $out && mkdir -p $out/browser
  cp $NSH_DIR/target/wasm32-unknown-unknown/web/nsh_platform_web.wasm $out/nsh-browser.wasm
  cp $NSH_DIR/crates/nsh-platform-web/web/nsh.js $out/
  cp $NSH_DIR/crates/nsh-platform-web/web/browser/*.js $out/browser/
  cp $NSH_DIR/crates/nsh-platform-web/web/browser/*.txt $out/browser/ 2>/dev/null || true
  cp ${syntaxes[1]} $out/nsh-syntaxes.bin
  print -r -- $stamp > $out/.stamp
  say "web runtime: $out"
}

build_web() {
  build_web_runtime
  local tmp=$(mktemp -d)
  trap "rm -rf $tmp" EXIT
  say "nsh package --target web"
  local log=$tmp/package.log
  (cd $UI && NSH_RUNTIME=$RUNTIME $NSH package App.tsx --target web -o $tmp/out) 2>&1 | tee $log
  local site=(${tmp}/out/*(/N))
  (( ${#site} == 1 )) || die "nsh package 没有产出站点目录"
  rm -rf $REPO/gui/dist && mkdir -p $REPO/gui/dist
  cp -R ${site[1]}/. $REPO/gui/dist/
  # pdf.js 与它的 worker、wasm、字体: PDF 阅读器的 Web 宿主 (web/host.js) 按相对路径 pdfjs/ 加载, 版本见 web/pdfjs/VERSION。
  cp -R $UI/web/pdfjs $REPO/gui/dist/pdfjs
  # 页面自带内联脚本, 在严格的 script-src 策略下要加这个 hash; 每次构建都会变。
  sed -n 's/.*it needs \(script-src .*\)$/\1/p' $log > $REPO/gui/dist/csp-script-src.txt
  say "gui/dist: $(du -sh $REPO/gui/dist | cut -f1), CSP: $(<$REPO/gui/dist/csp-script-src.txt)"
}

build_native() {
  say "nsh build --release (原生 bundle)"
  rm -rf $UI/dist/native && mkdir -p $UI/dist
  (cd $UI && $NSH build App.tsx --release -o $UI/dist/native)
  say "client-ui/dist/native: $(ls $UI/dist/native | tr '\n' ' ')"
}

# 嵌入 iOS 的 NativeShell 运行时默认只注册 nsh/base 的 core 部分 (没有 Select、Menu、Popover、Sheet、ToastStack、
# Combobox、Tooltip), 按名字 import 它们会让整个 bundle 在 iOS 上加载失败。这里挡在构建前。
# 允许表 = core 的组件, 加上实测在 iPad 与 visionOS 模拟器上有导出的 Input、Dialog、NavStack、NavPage、createNavStack。
check_embed_parts() {
  local allowed=(Accordion AccordionHeader AccordionItem AccordionPanel AccordionTrigger Avatar AvatarGroup Button Link Checkbox
    Collapsible Pagination Radio RadioGroup Slider Scrollbar Switch Progress Tab Tabs Toggle ToggleGroup Toolbar ToolbarGroup
    ToolbarSeparator AlertDialog Input Dialog NavStack NavPage createNavStack)
  local used name bad=()
  used=(${(f)"$(cd $UI && perl -0777 -ne 'while (/import\s*\{([^}]*)\}\s*from\s*"nsh\/base"/g) { for (split /,/, $1) { s/^\s+|\s+$//g; s/\s+as\s+\w+$//; next if /^type\s/ || $_ eq ""; print "$_\n" } }' App.tsx $(find src -name '*.ts' -o -name '*.tsx') | sort -u)"})
  for name in $used; do (( ${allowed[(I)$name]} )) || bad+=($name); done
  (( ${#bad} == 0 )) || die "client-ui 用了嵌入 iOS 运行时没有的 nsh/base 组件: ${bad[*]} (见 HOST_PROTOCOL.md「嵌入 iOS 时可用的 nsh/base 组件」)"
  say "nsh/base 组件都在 core 部分内: ${used[*]}"
}

case ${1:-web} in
  web) build_web ;;
  native) build_native ;;
  check)
    check_embed_parts
    (cd $UI && $NSH check)
    ;;
  test) (cd $UI && $NSH test) ;;
  all)
    check_embed_parts
    (cd $UI && $NSH check)
    (cd $UI && $NSH test)
    build_web
    build_native
    ;;
  *) die "用法: $0 [web|native|check|test|all]" ;;
esac
