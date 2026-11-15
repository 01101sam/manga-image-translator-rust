#!/bin/zsh
# build.sh 与 scripts/build-ui.sh 共用: 固定的 NSH 版本与「把源码目录整理成固定 commit 加补丁」的步骤。
# 调用方需要先设置 HERE (本目录)、NSH_DIR、PATCH。

NSH_URL=https://github.com/longbridge/nsh
# 必须是完整哈希: GitHub 不接受按短哈希 fetch。
NSH_COMMIT=e2649500263700af33c08afad8a11fd76e44748e

log() { print -r -- "[nsh-visionos] $*" }

# 保证 $NSH_DIR 是 NSH_COMMIT 加补丁的干净工作树; 状态未变时不碰文件, 免得 cargo 因 mtime 重编。
sync_nsh() {
  if [[ ! -d $NSH_DIR/.git ]]; then
    [[ ! -e $NSH_DIR || -z $(ls -A $NSH_DIR) ]] || { log "$NSH_DIR 非空且不是 git 仓库, 拒绝使用"; exit 1 }
    mkdir -p $NSH_DIR
    git -C $NSH_DIR init -q
    git -C $NSH_DIR remote add origin $NSH_URL
  fi
  [[ $(git -C $NSH_DIR remote get-url origin) == $NSH_URL ]] || { log "$NSH_DIR 的 origin 不是 $NSH_URL, 拒绝使用"; exit 1 }
  git -C $NSH_DIR cat-file -e $NSH_COMMIT^{commit} 2>/dev/null || {
    log "fetch $NSH_COMMIT"
    git -C $NSH_DIR fetch -q --depth 1 origin $NSH_COMMIT
  }
  local want="$NSH_COMMIT $(shasum $PATCH | cut -d' ' -f1)"
  local stamp=$NSH_DIR/.git/image-translator-patch-state
  local have=
  if [[ -f $stamp && $(git -C $NSH_DIR rev-parse HEAD) == $NSH_COMMIT ]]; then
    # 工作树被改过 (diff 哈希对不上) 也要重来。
    [[ $(<$stamp) == "$want $(git -C $NSH_DIR diff HEAD | shasum | cut -d' ' -f1)" ]] && have=1
  fi
  if [[ -z $have ]]; then
    log "reset 到 ${NSH_COMMIT[1,7]} 并应用补丁"
    git -C $NSH_DIR checkout -q -f --detach $NSH_COMMIT
    git -C $NSH_DIR reset -q --hard $NSH_COMMIT
    git -C $NSH_DIR clean -fdq
    git -C $NSH_DIR apply $PATCH
    print -r -- "$want $(git -C $NSH_DIR diff HEAD | shasum | cut -d' ' -f1)" > $stamp
  else
    log "NSH 源码已是 ${NSH_COMMIT[1,7]} + 补丁, 跳过"
  fi
}
