#!/bin/bash
# Tạo chứng chỉ ký code tự ký "Chup Self-Signed" trong keychain đăng nhập (một lần trên máy dev).
# Chữ ký ổn định giữa các lần build nên macOS không hỏi lại quyền Ghi màn hình / Trợ năng sau mỗi bản mới.
# macOS sẽ hỏi mật khẩu đăng nhập để tin cậy chứng chỉ cho việc ký code.
set -euo pipefail

NAME="Chup Self-Signed"
if security find-identity -p codesigning | grep -q "\"$NAME\""; then
  echo "Đã có \"$NAME\"."
  exit 0
fi
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
cat > "$TMP/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CNF
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -config "$TMP/cert.cnf"
PASS=chup
openssl pkcs12 -export -legacy -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -name "$NAME" -out "$TMP/cert.p12" -passout "pass:$PASS"
KC="$HOME/Library/Keychains/login.keychain-db"
security import "$TMP/cert.p12" -k "$KC" -P "$PASS" -T /usr/bin/codesign
security add-trusted-cert -r trustRoot -p codeSign -k "$KC" "$TMP/cert.pem"
security find-identity -p codesigning | grep "$NAME"
echo "Xong. Build: scripts/macos-bundle.sh"
