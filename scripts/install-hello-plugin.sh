#!/usr/bin/env bash
# 把示例插件 hello 复制到数据目录，避免依赖当前工作目录。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data="${FOOBAR_RS_DATA_DIR:-$HOME/.local/share/foobar-rs}"
target="$data/plugins/hello"

mkdir -p "$data/plugins"
rm -rf "$target"
cp -r "$root/examples/plugins/hello" "$target"

for file in plugin.json plugin.wat; do
    if [ ! -f "$target/$file" ]; then
        echo "安装失败：缺少 $target/$file" >&2
        exit 1
    fi
done

echo "插件已安装到 $target"
echo "启动: FOOBAR_RS_DATA_DIR=$data cargo run -p fb-app"
