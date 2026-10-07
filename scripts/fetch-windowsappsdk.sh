#!/usr/bin/env bash
# Same as fetch-windowsappsdk.ps1 but for Git Bash / MSYS2 dev shells.
# Usage: bash scripts/fetch-windowsappsdk.sh [version]
set -euo pipefail

VERSION="${1:-1.6.250108002}"
WEBVIEW2_VERSION="${2:-1.0.2957.106}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/Frameworks/WindowsAppSDK"
TMP="$(mktemp -d)"

# unzip → bsdtar → python3/python -m zipfile, whichever works. GNU tar
# cannot read zip archives so it is deliberately not a candidate. Note
# `bash` on Windows PATH may resolve to WSL's bash, which has a different
# tool set than Git Bash — hence the fall-through on failure.
extract() {
    if command -v unzip >/dev/null 2>&1 && unzip -o -q "$1" -d "$2"; then
        return 0
    fi
    if command -v bsdtar >/dev/null 2>&1 && bsdtar -xf "$1" -C "$2"; then
        return 0
    fi
    if command -v python3 >/dev/null 2>&1 && python3 -m zipfile -e "$1" "$2"; then
        return 0
    fi
    if command -v python >/dev/null 2>&1 && python -m zipfile -e "$1" "$2"; then
        return 0
    fi
    echo "no working zip extractor found (need unzip/bsdtar/python)" >&2
    return 1
}

echo "Downloading Microsoft.WindowsAppSDK $VERSION ..."
curl -sSfL -o "$TMP/wasdk.nupkg" \
    "https://www.nuget.org/api/v2/package/Microsoft.WindowsAppSDK/$VERSION"
mkdir -p "$TMP/wasdk"
extract "$TMP/wasdk.nupkg" "$TMP/wasdk"

echo "Downloading Microsoft.Web.WebView2 $WEBVIEW2_VERSION ..."
curl -sSfL -o "$TMP/webview2.nupkg" \
    "https://www.nuget.org/api/v2/package/Microsoft.Web.WebView2/$WEBVIEW2_VERSION"
mkdir -p "$TMP/webview2"
extract "$TMP/webview2.nupkg" "$TMP/webview2"

mkdir -p "$DEST"
for rel in \
    "lib/uap10.0/Microsoft.UI.Xaml.winmd" \
    "lib/uap10.0/Microsoft.UI.Text.winmd" \
    "lib/uap10.0/Microsoft.Windows.AppLifecycle.winmd" \
    "lib/uap10.0/Microsoft.Windows.ApplicationModel.DynamicDependency.winmd" \
    "lib/uap10.0/Microsoft.Windows.ApplicationModel.Resources.winmd" \
    "lib/uap10.0.18362/Microsoft.Foundation.winmd" \
    "lib/uap10.0.18362/Microsoft.Graphics.winmd" \
    "lib/uap10.0.18362/Microsoft.UI.winmd" \
    "runtimes/win-x64/native/Microsoft.WindowsAppRuntime.Bootstrap.dll"; do
    cp "$TMP/wasdk/$rel" "$DEST/"
done
# Microsoft.UI.Xaml.Controls.WebView2 references this winmd.
cp "$TMP/webview2/lib/Microsoft.Web.WebView2.Core.winmd" "$DEST/"
rm -rf "$TMP"
echo "Windows App SDK metadata ready under $DEST"
