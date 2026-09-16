#!/usr/bin/env bash
#
# PaneFM 一鍵安裝腳本 (macOS / Linux)
# 使用方式:
#   curl -fsSL https://raw.githubusercontent.com/winthropchang/panefm/main/install.sh | bash
#

set -euo pipefail

REPO="winthropchang/panefm"

# 1. 偵測作業系統
OS="$(uname -s)"
case "$OS" in
    Darwin)
        PLATFORM="macos"
        ;;
    Linux)
        PLATFORM="linux"
        ;;
    *)
        echo "❌ 不支援的作業系統: $OS"
        exit 1
        ;;
esac

# 2. 偵測硬體架構
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64)
        ARCH="x64"
        ;;
    arm64|aarch64)
        ARCH="arm64"
        ;;
    *)
        echo "❌ 不支援的架構: $ARCH"
        exit 1
        ;;
esac

# 3. 比對 Releases 資產檔名
ASSET_NAME="panefm-${PLATFORM}-${ARCH}"

if [ "$PLATFORM" = "linux" ]; then
    echo "⚠️ 目前 GitHub Releases 官方僅發布 macOS (arm64/x64) 與 Windows 二進位檔。"
    echo "Linux 使用者請透過 cargo 安裝："
    echo "  cargo install --git https://github.com/${REPO}.git"
    exit 1
fi

DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/${ASSET_NAME}"

echo "=========================================="
echo "🚀 正在安裝 PaneFM ($ASSET_NAME)..."
echo "=========================================="

# 4. 決定安裝路徑
if [ -w "/usr/local/bin" ]; then
    INSTALL_DIR="/usr/local/bin"
elif [ "$(id -u)" -eq 0 ]; then
    INSTALL_DIR="/usr/local/bin"
else
    INSTALL_DIR="$HOME/.local/bin"
fi

mkdir -p "$INSTALL_DIR"
TARGET_PATH="$INSTALL_DIR/panefm"
TMP_PATH="${TARGET_PATH}.tmp.$$"

echo "📥 下載中: $DOWNLOAD_URL"
if command -v curl >/dev/null 2>&1; then
    curl -# -fL "$DOWNLOAD_URL" -o "$TMP_PATH"
elif command -v wget >/dev/null 2>&1; then
    wget -q --show-progress -O "$TMP_PATH" "$DOWNLOAD_URL"
else
    echo "❌ 找不到 curl 或 wget 指令，請先安裝其一。"
    exit 1
fi

# 5. 賦予可執行權限
chmod 755 "$TMP_PATH"

# 6. macOS 專屬修復：解除 Gatekeeper 隔離屬性與 ad-hoc 簽名
if [ "$PLATFORM" = "macos" ]; then
    echo "🛡️ 解除 macOS Gatekeeper 隔離屬性並進行程式碼簽名..."
    xattr -d com.apple.quarantine "$TMP_PATH" 2>/dev/null || true
    xattr -c "$TMP_PATH" 2>/dev/null || true
    codesign -s - --force "$TMP_PATH" 2>/dev/null || true
fi

mv -f "$TMP_PATH" "$TARGET_PATH"

echo ""
echo "✅ PaneFM 安裝成功！安裝位置: $TARGET_PATH"

# 7. PATH 檢查
if ! echo ":$PATH:" | grep -q ":$INSTALL_DIR:"; then
    echo ""
    echo "⚠️ 注意：$INSTALL_DIR 目前尚未包含在您的 PATH 環境變數中。"
    echo "請將以下指令加入您的 shell 設定檔 (~/.zshrc 或 ~/.bashrc)："
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    echo ""
fi

echo ""
echo "🎉 現在您可以直接在終端機輸入 'panefm' 開始使用！"
