#!/usr/bin/env bash
# 构建 FoobarRS.app 并压缩为 zip。用法: VERSION=0.1.0 packaging/macos/package.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
version="${VERSION:-0.1.0}"
app="$root/dist/FoobarRS.app"
contents="$app/Contents"

cd "$root"
cargo build --release -p fb-app

rm -rf "$app"
mkdir -p "$contents/MacOS" "$contents/Resources"
install -m755 target/release/fb-app "$contents/MacOS/fb-app"
install -m644 assets/icons/rs.foobar_rs.svg "$contents/Resources/rs.foobar_rs.svg"

cat > "$contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>FoobarRS</string>
    <key>CFBundleDisplayName</key>
    <string>foobar-rs</string>
    <key>CFBundleIdentifier</key>
    <string>rs.foobar_rs</string>
    <key>CFBundleExecutable</key>
    <string>fb-app</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

zip="$root/dist/foobar-rs-$version-macos-$(uname -m).zip"
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
echo "$zip"

# 可选：签名/公证/dmg；未配置证书时脚本自行跳过。
VERSION="$version" bash packaging/macos/sign-and-notarize.sh "$app" "$zip"
