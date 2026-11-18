#!/usr/bin/env bash
# 可选签名/公证 macOS 产物。用法: VERSION=0.1.0 packaging/macos/sign-and-notarize.sh [FoobarRS.app] [zip]
# 未设置 APPLE_SIGNING_IDENTITY / APPLE_* 时打印跳过并退出 0。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
version="${VERSION:-0.1.0}"
arch="$(uname -m)"
app="${1:-$root/dist/FoobarRS.app}"
zip_in="${2:-}"

identity="${APPLE_SIGNING_IDENTITY:-}"
notarize=0
if [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
    notarize=1
fi

[[ -d "$app" ]] || { echo "找不到 $app" >&2; exit 1; }

if [[ -z "$identity" && $notarize -eq 0 ]] && ! command -v hdiutil >/dev/null 2>&1; then
    echo "跳过签名/公证：未设置 APPLE_SIGNING_IDENTITY 或 APPLE_* 凭据，且无 hdiutil" >&2
    exit 0
fi

# --deep 便于一次性签 .app；正式分发建议逐个嵌套二进制显式签名。
if [[ -n "$identity" ]]; then
    echo "签名 $app"
    codesign --force --deep --options runtime --timestamp --sign "$identity" "$app"
    codesign --verify --deep --strict --verbose=2 "$app"
else
    echo "跳过签名：未设置 APPLE_SIGNING_IDENTITY" >&2
fi

dmg=""
if command -v hdiutil >/dev/null 2>&1; then
    dmg="$root/dist/foobar-rs-$version-macos-$arch.dmg"
    rm -f "$dmg"
    hdiutil create -volname "foobar-rs" -srcfolder "$app" -ov -format UDZO "$dmg" >/dev/null
    if [[ -n "$identity" ]]; then
        codesign --force --timestamp --sign "$identity" "$dmg"
    fi
    echo "$dmg"
fi

if [[ $notarize -eq 1 ]]; then
    # notarytool 只接受 dmg/zip/pkg；优先提交 dmg，否则用 package.sh 生成的 zip。
    target="$dmg"
    [[ -n "$target" ]] || target="$zip_in"
    [[ -n "$target" && -e "$target" ]] \
        || { echo "公证需要 dmg 或 zip，但两者都不存在" >&2; exit 1; }
    xcrun notarytool submit "$target" \
        --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" --wait
    xcrun stapler staple "$target"
    # zip 公证时票据只能订到 .app 上；dmg 公证时 dmg 已订票。
    [[ "$target" == "$dmg" ]] || xcrun stapler staple "$app"
    spctl --assess --type execute --verbose=2 "$app" || true
else
    echo "跳过公证：未设置 APPLE_ID/APPLE_TEAM_ID/APPLE_APP_PASSWORD" >&2
fi
