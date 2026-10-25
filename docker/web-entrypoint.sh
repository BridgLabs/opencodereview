#!/bin/sh
set -eu
TOKEN="${VITE_OCR_API_TOKEN:-}"
if [ -z "$TOKEN" ]; then
  for _ in $(seq 1 30); do
    [ -s /data/api-token ] && break
    sleep 1
  done
  if [ ! -s /data/api-token ]; then
    echo "ERROR: /data/api-token was not created; refusing to serve assets with placeholder token" >&2
    exit 1
  fi
  TOKEN="$(cat /data/api-token)"
fi
ESCAPED_TOKEN="$(printf '%s' "$TOKEN" | sed 's/[\\&|]/\\&/g')"
find /usr/share/nginx/html -type f -name '*.js' -exec sed -i "s|__OCR_API_TOKEN__|$ESCAPED_TOKEN|g" {} +
