#!/bin/bash
# Đóng gói Chup cho macOS (SPEC 11, thiết kế docs/plans/2026-11-05-macos-design.md mục 5):
# build release -> Chup.app (Info.plist, icon) -> ký -> (tuỳ chọn) .dmg.
#
#   scripts/macos-bundle.sh            # Chup.app, ký bằng $CHUP_SIGN_ID (mặc định "Chup Self-Signed", không có thì ad-hoc)
#   scripts/macos-bundle.sh --dmg      # thêm Chup-<version>.dmg
#   CHUP_SIGN_ID=- scripts/macos-bundle.sh   # ép ký ad-hoc (CI)
#
# Ký ad-hoc: mỗi lần build lại macOS có thể hỏi lại quyền Ghi màn hình (quyền gắn với chữ ký).
# Chứng chỉ tự ký (scripts/macos-make-cert.sh) giữ quyền qua các lần build. Đổi sang Developer ID: đặt CHUP_SIGN_ID
# là tên chứng chỉ Developer ID Application rồi notarize .dmg bằng `xcrun notarytool`.
set -euo pipefail

cd "$(dirname "$0")/.."
DMG=0
[[ "${1:-}" == "--dmg" ]] && DMG=1

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
OUT=target/macos
APP="$OUT/Chup.app"

cargo build --release -p chup

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/chup "$APP/Contents/MacOS/chup"
cp crates/app/icons/icon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>app.chup.desktop</string>
  <key>CFBundleName</key><string>Chup</string>
  <key>CFBundleDisplayName</key><string>Chup</string>
  <key>CFBundleExecutable</key><string>chup</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>26.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>Chup</string>
</dict>
</plist>
PLIST

SIGN_ID="${CHUP_SIGN_ID:-Chup Self-Signed}"
if [[ "$SIGN_ID" != "-" ]] && ! security find-identity -p codesigning 2>/dev/null | grep -q "\"$SIGN_ID\""; then
  echo "Không thấy chứng chỉ \"$SIGN_ID\" (chạy scripts/macos-make-cert.sh để tạo): ký ad-hoc."
  SIGN_ID="-"
fi
codesign --force --options runtime --timestamp=none --sign "$SIGN_ID" "$APP"
codesign --verify --strict "$APP"
echo "Đã ký $APP bằng \"$SIGN_ID\""

if [[ $DMG == 1 ]]; then
  STAGE="$OUT/dmg"
  rm -rf "$STAGE" && mkdir -p "$STAGE"
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  DMG_PATH="$OUT/Chup-${VERSION}.dmg"
  rm -f "$DMG_PATH"
  hdiutil create -volname "Chup" -srcfolder "$STAGE" -ov -format UDZO "$DMG_PATH" >/dev/null
  rm -rf "$STAGE"
  echo "Đã tạo $DMG_PATH"
fi
