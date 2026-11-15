#!/bin/zsh
# 把 pdfjs-dist 的运行时文件拷进 client-ui/web/pdfjs (入库, 构建时不联网)。
# 用法: scripts/vendor-pdfjs.sh <pdfjs-dist 目录>   例如 npm pack pdfjs-dist@6.5.438 解开后的 package/
#
# 为什么改成 .js: Daemon 按扩展名给 Content-Type, 不认识 .mjs 会发 application/octet-stream,
# 浏览器拒绝把它当模块脚本或模块 Worker 加载。
set -euo pipefail

SRC=${1:?用法: $0 <pdfjs-dist 目录>}
DEST=${0:A:h:h}/client-ui/web/pdfjs
VERSION=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' $SRC/package.json | head -1)
[[ -n $VERSION && -f $SRC/build/pdf.min.mjs ]] || { print -r -- "$SRC 不是 pdfjs-dist 目录" >&2; exit 1 }

rm -rf $DEST && mkdir -p $DEST/wasm
cp $SRC/build/pdf.min.mjs $DEST/pdf.js
cp $SRC/build/pdf.worker.min.mjs $DEST/pdf.worker.js
# 只带解码用的 wasm: 扫描件的 JPX、JBIG2 和带 ICC 的颜色; quickjs-eval 是 PDF 内嵌脚本用的, 阅读器不开脚本。
cp $SRC/wasm/{jbig2,openjpeg,qcms_bg}.wasm $SRC/wasm/LICENSE_* $DEST/wasm/
cp -R $SRC/cmaps $SRC/standard_fonts $SRC/iccs $DEST/
cp $SRC/LICENSE $DEST/LICENSE
print -r -- "pdfjs-dist $VERSION" > $DEST/VERSION
print -r -- "pdfjs-dist $VERSION -> $DEST ($(du -sh $DEST | cut -f1))"
