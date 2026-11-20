#!/usr/bin/env bash
# 构建 Linux 发布目录、tar.gz 和 deb。用法: VERSION=0.1.0 packaging/linux/package.sh
# deb 手工用 ar/tar 组装，不依赖 dpkg-deb。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
version="${VERSION:-0.1.0}"
name="foobar-rs-$version-linux-x86_64"
stage="$root/dist/$name"

cd "$root"
cargo build --release -p fb-app

rm -rf "$stage"
mkdir -p "$stage/assets/icons" "$stage/assets/linux"
install -m755 target/release/fb-app "$stage/fb-app"
install -m644 README.md LICENSE "$stage/"
install -m644 assets/icons/io.github.wang1zhen.foobar-rs.svg assets/icons/io.github.wang1zhen.foobar-rs.png "$stage/assets/icons/"
install -m644 assets/linux/io.github.wang1zhen.foobar-rs.appdata.xml "$stage/assets/linux/"
install -m644 assets/linux/io.github.wang1zhen.foobar-rs.desktop "$stage/io.github.wang1zhen.foobar-rs.desktop"
install -m755 packaging/linux/install.sh "$stage/install.sh"

tarball="$root/dist/$name.tar.gz"
tar -C "$root/dist" -czf "$tarball" "$name"
echo "$tarball"

# ---- deb ----
case "$(uname -m)" in
    x86_64) deb_arch=amd64 ;;
    aarch64) deb_arch=arm64 ;;
    *) echo "不支持的架构: $(uname -m)" >&2; exit 1 ;;
esac
maintainer="${MAINTAINER:-foobar-rs maintainers <maintainers@example.invalid>}"

debroot="$root/dist/deb-root"
debctl="$root/dist/deb-control"
debwork="$root/dist/deb-ar"
deb="$root/dist/foobar-rs_${version}_${deb_arch}.deb"
rm -rf "$debroot" "$debctl" "$debwork"
rm -f "$deb"
mkdir -p "$debroot/usr/bin" \
         "$debroot/usr/share/applications" \
         "$debroot/usr/share/icons/hicolor/scalable/apps" \
         "$debroot/usr/share/icons/hicolor/256x256/apps" \
         "$debroot/usr/share/metainfo" \
         "$debroot/usr/share/doc/foobar-rs" \
         "$debctl" "$debwork"

install -m755 target/release/fb-app "$debroot/usr/bin/fb-app"
sed 's|^Exec=.*|Exec=/usr/bin/fb-app|' assets/linux/io.github.wang1zhen.foobar-rs.desktop \
    > "$debroot/usr/share/applications/io.github.wang1zhen.foobar-rs.desktop"
chmod 644 "$debroot/usr/share/applications/io.github.wang1zhen.foobar-rs.desktop"
install -m644 assets/icons/io.github.wang1zhen.foobar-rs.svg \
    "$debroot/usr/share/icons/hicolor/scalable/apps/io.github.wang1zhen.foobar-rs.svg"
install -m644 assets/icons/io.github.wang1zhen.foobar-rs.png \
    "$debroot/usr/share/icons/hicolor/256x256/apps/io.github.wang1zhen.foobar-rs.png"
install -m644 assets/linux/io.github.wang1zhen.foobar-rs.appdata.xml \
    "$debroot/usr/share/metainfo/io.github.wang1zhen.foobar-rs.appdata.xml"
install -m644 README.md "$debroot/usr/share/doc/foobar-rs/README"
install -m644 LICENSE "$debroot/usr/share/doc/foobar-rs/LICENSE"
install -m644 THIRD_PARTY.md "$debroot/usr/share/doc/foobar-rs/THIRD_PARTY"
# 仓库没有 CHANGELOG，按版本生成最小 Debian changelog。
cat > "$debroot/usr/share/doc/foobar-rs/changelog" <<EOF
foobar-rs ($version) unstable; urgency=medium

  * Release $version.

 -- $maintainer  $(date -R)
EOF
gzip -9n "$debroot/usr/share/doc/foobar-rs/changelog"

# x11/wayland 后端由 winit 运行时 dlopen，ldd 看不到；以下依赖需在目标发行版上核对。
# Ubuntu 24.04 起 libasound2 改名为 libasound2t64，用 alternatives 兼容。
depends="libasound2 | libasound2t64, libxkbcommon0, libwayland-client0 | libx11-6, libxcursor1, libxrandr2, libxi6, libxcb1, libgl1"
cat > "$debctl/control" <<EOF
Package: foobar-rs
Version: $version
Architecture: $deb_arch
Maintainer: $maintainer
Installed-Size: $(du -sk --apparent-size "$debroot" | cut -f1)
Section: sound
Priority: optional
Depends: $depends
Description: foobar2000 风格音乐播放器
 foobar-rs 基于 Rust + egui，支持本地媒体库、播放列表、
 Navidrome 远程源和 WASM 插件。
EOF
# dpkg-deb 会自动生成 md5sums，手工构建时补上。
(cd "$debroot" && find usr -type f -print0 | sort -z | xargs -0 md5sum) > "$debctl/md5sums"

printf '2.0\n' > "$debwork/debian-binary"
tar -C "$debctl" --sort=name --owner=0 --group=0 --numeric-owner -czf "$debwork/control.tar.gz" .
tar -C "$debroot" --sort=name --owner=0 --group=0 --numeric-owner -czf "$debwork/data.tar.gz" .
(cd "$debwork" && ar rcs "$deb" debian-binary control.tar.gz data.tar.gz)

# ---- 校验 ----
expected=$'debian-binary\ncontrol.tar.gz\ndata.tar.gz'
[[ "$(ar t "$deb")" == "$expected" ]] || { echo "deb 成员不匹配: $deb" >&2; exit 1; }
# tar 清单先落盘：grep -q 提前退出会让 tar 收到 SIGPIPE，pipefail 下误报失败。
tar tzf "$debwork/control.tar.gz" > "$debwork/control.list"
tar tzf "$debwork/data.tar.gz" > "$debwork/data.list"
for path in ./control ./md5sums; do
    grep -qx -- "$path" "$debwork/control.list" \
        || { echo "control.tar.gz 缺少 $path" >&2; exit 1; }
done
for path in ./usr/bin/fb-app \
            ./usr/share/applications/io.github.wang1zhen.foobar-rs.desktop \
            ./usr/share/icons/hicolor/scalable/apps/io.github.wang1zhen.foobar-rs.svg \
            ./usr/share/icons/hicolor/256x256/apps/io.github.wang1zhen.foobar-rs.png \
            ./usr/share/metainfo/io.github.wang1zhen.foobar-rs.appdata.xml \
            ./usr/share/doc/foobar-rs/README \
            ./usr/share/doc/foobar-rs/LICENSE \
            ./usr/share/doc/foobar-rs/THIRD_PARTY \
            ./usr/share/doc/foobar-rs/changelog.gz; do
    grep -qx -- "$path" "$debwork/data.list" \
        || { echo "data.tar.gz 缺少 $path" >&2; exit 1; }
done
grep -qx 'Package: foobar-rs' "$debctl/control"
grep -qx "Version: $version" "$debctl/control"
grep -qx "Architecture: $deb_arch" "$debctl/control"
grep -q '^Maintainer: .\+' "$debctl/control"
grep -q '^Depends: .\+' "$debctl/control"

rm -rf "$debroot" "$debctl" "$debwork"
# SHA256SUMS 只含本次 Linux 产物；appimage.sh 会追加 AppImage。
(cd "$root/dist" && sha256sum "$(basename "$tarball")" "$(basename "$deb")" > SHA256SUMS)
echo "$deb"
