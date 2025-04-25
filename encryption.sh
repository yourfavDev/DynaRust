#!/usr/bin/env bash
set -euo pipefail

KEY_FILE="encryption.key"
KEY_SIZE=32  # 32 bytes = 256 bits

if [[ -e "$KEY_FILE" ]]; then
  echo "Error: $KEY_FILE already exists. Remove or rename it and retry."
  exit 1
fi

# Method 1: Using /dev/urandom
head -c "$KEY_SIZE" /dev/urandom > "$KEY_FILE"

# Alternatively, with OpenSSL:
# openssl rand -out "$KEY_FILE" "$KEY_SIZE"

# Restrict file perms so only the owner can read/write
chmod 600 "$KEY_FILE"

echo "Generated a $KEY_SIZE-byte key at $KEY_FILE (permissions 600)."
