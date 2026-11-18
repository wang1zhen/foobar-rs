#!/usr/bin/env bash
# 安装桌面入口与图标，供应用菜单和 Wayland 任务栏使用。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
icon_dir="$data/icons/hicolor/scalable/apps"
apps_dir="$data/applications"

bin=""
for candidate in "$root/target/release/fb-app" "$root/target/debug/fb-app"; do
    if [[ -x "$candidate" ]]; then
        bin="$candidate"
        break
    fi
done
if [[ -z "$bin" ]]; then
    echo "未找到 fb-app 二进制，请先运行: cargo build -p fb-app" >&2
    exit 1
fi

mkdir -p "$icon_dir" "$apps_dir"
cp "$root/assets/icons/rs.foobar_rs.svg" "$icon_dir/rs.foobar_rs.svg"

# 菜单不依赖 PATH：把 Exec 换成本次构建的绝对路径。
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
while IFS= read -r line; do
    printf '%s\n' "${line/#Exec=*/Exec=$bin}"
done < "$root/assets/linux/rs.foobar_rs.desktop" > "$tmp"
install -m644 "$tmp" "$apps_dir/rs.foobar_rs.desktop"

if command -v gtk-update-icon-cache >/dev/null 2>&1 && [[ -w "$data/icons/hicolor" ]]; then
    gtk-update-icon-cache -f -t "$data/icons/hicolor" >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$apps_dir"
fi

echo "图标:   $icon_dir/rs.foobar_rs.svg"
echo "桌面项: $apps_dir/rs.foobar_rs.desktop"
echo "可执行: $bin"
echo "从应用菜单启动 foobar-rs；若菜单未刷新，注销并重新登录。"
