#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOST="${HOST:-127.0.0.1}"
PORT="${PORT:-8765}"
PUBLIC_DIR="${PUBLIC_DIR:-$ROOT_DIR/public}"
BIN="$ROOT_DIR/target/release/clinpatch"

cd "$ROOT_DIR"

if [[ ! -x "$BIN" ]]; then
  echo "Building release binary..."
  cargo build --release
fi

if [[ ! -d "$PUBLIC_DIR" ]]; then
  echo "Missing public directory: $PUBLIC_DIR" >&2
  exit 1
fi

for file in \
  "$PUBLIC_DIR/index.html" \
  "$PUBLIC_DIR/app.js" \
  "$PUBLIC_DIR/styles.css" \
  "$PUBLIC_DIR/clinvar.GRCh38.sample.vcf" \
  "$PUBLIC_DIR/clinvar.GRCh38.sample.vcf.rows.json" \
  "$PUBLIC_DIR/clinvar.GRCh38.sample.vcf.ids.json" \
  "$PUBLIC_DIR/clinvar.GRCh38.sample.vcf.positions.json"
do
  if [[ ! -f "$file" ]]; then
    echo "Missing demo file: $file" >&2
    exit 1
  fi
done

echo "Serving $PUBLIC_DIR"
echo "Open http://$HOST:$PORT/"
exec "$BIN" serve --root "$PUBLIC_DIR" --bind "$HOST:$PORT"
