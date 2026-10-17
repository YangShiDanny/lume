#!/bin/bash
# Install lume to ~/.local/bin and check PATH / clang

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$SCRIPT_DIR/lume"
TARGET_DIR="$HOME/.local/bin"

if [ ! -f "$BIN" ]; then
    echo "Error: lume not found in $SCRIPT_DIR" >&2
    exit 1
fi

mkdir -p "$TARGET_DIR"
cp "$BIN" "$TARGET_DIR/lume"
chmod +x "$TARGET_DIR/lume"

echo "Installed to $TARGET_DIR/lume"

case ":$PATH:" in
    *":$TARGET_DIR:"*)
        echo "$TARGET_DIR is already in PATH"
        ;;
    *)
        echo ""
        echo "Add this line to ~/.bashrc or ~/.zshrc:"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac

if ! command -v clang >/dev/null 2>&1; then
    echo ""
    echo "Warning: clang not found. lume needs it to compile C."
    echo "  Ubuntu/Debian: sudo apt install clang"
    echo "  Fedora:        sudo dnf install clang"
    echo "  macOS:         xcode-select --install"
else
    echo "clang found: $(command -v clang)"
fi

echo ""
echo "Done. Open a new terminal, then run:"
echo "  lume run hello.lume"