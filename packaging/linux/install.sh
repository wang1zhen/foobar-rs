#!/usr/bin/env bash
# 从解压后的发布目录安装。用法: prefix=/usr/local ./install.sh（默认 ~/.local）
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
prefix="${prefix:-$HOME/.local}"
bin_dir="$prefix/bin"
apps_dir="$prefix/share/applications"
icon_dir="$prefix/share/icons/hicolor/scalable/apps"
png_icon_dir="$prefix/share/icons/hicolor/256x256/apps"
metainfo_dir="$prefix/share/metainfo"

mkdir -p "$bin_dir" "$apps_dir" "$icon_dir" "$png_icon_dir" "$metainfo_dir"
install -m755 "$here/fb-app" "$bin_dir/fb-app"
install -m644 "$here/assets/icons/io.github.wang1zhen.foobar-rs.svg" "$icon_dir/io.github.wang1zhen.foobar-rs.svg"
install -m644 "$here/assets/icons/io.github.wang1zhen.foobar-rs.png" "$png_icon_dir/io.github.wang1zhen.foobar-rs.png"
install -m644 "$here/assets/linux/io.github.wang1zhen.foobar-rs.appdata.xml" \
    "$metainfo_dir/io.github.wang1zhen.foobar-rs.appdata.xml"
# 菜单不依赖 PATH：Exec 写死为安装后的绝对路径。
sed "s|^Exec=.*|Exec=$bin_dir/fb-app|" \
    "$here/io.github.wang1zhen.foobar-rs.desktop" > "$apps_dir/io.github.wang1zhen.foobar-rs.desktop"
chmod 644 "$apps_dir/io.github.wang1zhen.foobar-rs.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$apps_dir" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$prefix/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "已安装: $bin_dir/fb-app"
echo "桌面项: $apps_dir/io.github.wang1zhen.foobar-rs.desktop"
