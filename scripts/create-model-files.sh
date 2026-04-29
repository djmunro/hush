#!/bin/bash
# Pull the base model referenced by the Modelfile and register the custom
# Ollama model used for Hush post-processing (transcribe-editor-dev).
#
# Expects model-files as a sibling of this repo:
#   Development/hush/scripts/create-model-files.sh
#   Development/model-files/transcribe-editor-dev.md
#
# Requires: ollama on PATH, network for `ollama pull`.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODELFILES_DIR="$(cd "$ROOT/.." && pwd)/model-files"
MODFILE="$MODELFILES_DIR/transcribe-editor-dev.md"
MODEL_NAME="transcribe-editor-dev"

if ! command -v ollama >/dev/null 2>&1; then
  echo "ollama not found on PATH" >&2
  exit 1
fi

if [[ ! -f "$MODFILE" ]]; then
  echo "Modelfile not found: $MODFILE" >&2
  exit 1
fi

BASE_MODEL="$(grep -m1 '^FROM ' "$MODFILE" | awk '{print $2}' | tr -d '\r')"
if [[ -z "$BASE_MODEL" ]]; then
  echo "Could not parse FROM in $MODFILE" >&2
  exit 1
fi

echo "→ pulling base model $BASE_MODEL"
ollama pull "$BASE_MODEL"

echo "→ creating Ollama model $MODEL_NAME from $MODFILE"
ollama create "$MODEL_NAME" -f "$MODFILE"

echo "✓ $MODEL_NAME ready (ollama list)"
