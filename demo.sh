#!/usr/bin/env bash
set -euo pipefail

if command -v python3 >/dev/null 2>&1; then
  exec python3 demo.py "$@"
fi

exec uvx --from python python demo.py "$@"
