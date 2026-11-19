#!/usr/bin/env bash
# 可选构建 AppImage。用法: VERSION=0.1.0 packaging/linux/appimage.sh
# 找不到 appimagetool 时直接跳过（退出 0），由 CI 预先下载工具后重跑。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
version="${VERSION:-0.1.0}"
case "$(uname -m)" in
    x86_64) arch=x86_64 ;;
    aarch64) arch=aarch64 ;;
    *) echo "不支持的架构: $(uname -m)" >&2; exit 1 ;;
esac
out="$root/dist/foobar-rs-$version-linux-$arch.AppImage"

if ! command -v appimagetool >/dev/null 2>&1; then
    echo "跳过 AppImage：未找到 appimagetool" >&2
    exit 0
fi

cd "$root"
cargo build --release -p fb-app

appdir="$root/dist/foobar-rs.AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" \
         "$appdir/usr/share/applications" \
         "$appdir/usr/share/icons/hicolor/scalable/apps" \
         "$appdir/usr/share/icons/hicolor/256x256/apps" \
         "$appdir/usr/share/metainfo"
install -m755 target/release/fb-app "$appdir/usr/bin/fb-app"
sed 's|^Exec=.*|Exec=fb-app|' assets/linux/rs.foobar_rs.desktop \
    > "$appdir/usr/share/applications/rs.foobar_rs.desktop"
chmod 644 "$appdir/usr/share/applications/rs.foobar_rs.desktop"
install -m644 assets/icons/rs.foobar_rs.svg \
    "$appdir/usr/share/icons/hicolor/scalable/apps/rs.foobar_rs.svg"
install -m644 assets/icons/rs.foobar_rs.png \
    "$appdir/usr/share/icons/hicolor/256x256/apps/rs.foobar_rs.png"
install -m644 assets/linux/io.github.wang1zhen.foobar-rs.appdata.xml \
    "$appdir/usr/share/metainfo/io.github.wang1zhen.foobar-rs.appdata.xml"
# appimagetool 要求在 AppDir 根目录放 .desktop 和与之同名的图标。
cp "$appdir/usr/share/applications/rs.foobar_rs.desktop" "$appdir/rs.foobar_rs.desktop"
cp "$appdir/usr/share/icons/hicolor/256x256/apps/rs.foobar_rs.png" "$appdir/rs.foobar_rs.png"
# .DirIcon 必须是有效图片，PNG 比 SVG 兼容性更好。
cp "$appdir/rs.foobar_rs.png" "$appdir/.DirIcon"
cp "$appdir/usr/share/icons/hicolor/scalable/apps/rs.foobar_rs.svg" "$appdir/rs.foobar_rs.svg"
cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
exec "$here/usr/bin/fb-app" "$@"
EOF
chmod 755 "$appdir/AppRun"

# CI 常无 FUSE；APPIMAGE_EXTRACT_AND_RUN 让 appimagetool 自身解包运行，普通二进制会忽略该变量。
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" appimagetool "$appdir" "$out"
rm -rf "$appdir"

echo "$out"
(cd "$root/dist" && sha256sum "$(basename "$out")" >> SHA256SUMS)
