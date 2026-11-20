#!/bin/sh
set -e

export PATH="$HOME/.cargo/bin:$PATH"
PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_BUNDLE="$HOME/Applications/CoCo Job Monitor.app"
PLUGIN_DIR="$HOME/.snowflake/cortex/plugins/coco-job-monitor"
PLUGIN_BIN_DIR="$PLUGIN_DIR/bin"
ICONS_DIR="$HOME/.snowflake/cortex/job-monitor/icons"

cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml"

BIN_SRC="$PROJECT_DIR/target/release/coco-job-monitor"

mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"
mkdir -p "$PLUGIN_BIN_DIR" "$PLUGIN_DIR/.cortex-plugin" "$PLUGIN_DIR/scripts" "$PLUGIN_DIR/skills/job-monitor"
mkdir -p "$ICONS_DIR"

cp "$BIN_SRC" "$APP_BUNDLE/Contents/MacOS/coco-job-monitor"
cp "$BIN_SRC" "$PLUGIN_BIN_DIR/coco-job-monitor"

# Install default menu bar GIF icons without overwriting user customizations
for f in "$PROJECT_DIR"/assets/icons/*.gif; do
    base="$(basename "$f")"
    if [ ! -f "$ICONS_DIR/$base" ]; then
        cp "$f" "$ICONS_DIR/$base"
    fi
done

# Install plugin scripts & skill
cp "$PROJECT_DIR/plugin/scripts/notify.sh" "$PLUGIN_DIR/scripts/notify.sh"
cp "$PROJECT_DIR/plugin/scripts/sync-hooks.py" "$PLUGIN_DIR/scripts/sync-hooks.py"
cp "$PROJECT_DIR/plugin/skills/job-monitor/SKILL.md" "$PLUGIN_DIR/skills/job-monitor/SKILL.md"

sed "s|__HOME__|$HOME|g" "$PROJECT_DIR/plugin/.cortex-plugin/plugin.json.in" > "$PLUGIN_DIR/.cortex-plugin/plugin.json"
cp "$PLUGIN_DIR/.cortex-plugin/plugin.json" "$PLUGIN_DIR/.hooks.json"

cat > "$APP_BUNDLE/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDisplayName</key>
    <string>CoCo Job Monitor</string>
    <key>CFBundleExecutable</key>
    <string>coco-job-monitor</string>
    <key>CFBundleIdentifier</key>
    <string>dev.coco.job-monitor</string>
    <key>CFBundleName</key>
    <string>CoCo Job Monitor</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>14.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP_BUNDLE" >/dev/null 2>&1 || true
python3 "$PLUGIN_DIR/scripts/sync-hooks.py"

echo "Built app: $APP_BUNDLE"
echo "Icons dir: $ICONS_DIR"
