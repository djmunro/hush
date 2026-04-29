#!/bin/bash
# Pull base models and register an Ollama model for each Modelfile in model-files/*.md.
#
# Requires: ollama on PATH, network for `ollama pull`.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODELFILES_DIR="$ROOT/model-files"

if ! command -v ollama >/dev/null 2>&1; then
  echo "ollama not found on PATH" >&2
  exit 1
fi

shopt -s nullglob
modfiles=("$MODELFILES_DIR"/*.md)
if [[ ${#modfiles[@]} -eq 0 ]]; then
  echo "No .md Modelfiles in $MODELFILES_DIR" >&2
  exit 1
fi

for MODFILE in "${modfiles[@]}"; do
  MODEL_NAME="$(basename "$MODFILE" .md)"
  BASE_MODEL="$(grep -m1 '^FROM ' "$MODFILE" | awk '{print $2}' | tr -d '\r')"
  if [[ -z "$BASE_MODEL" ]]; then
    echo "Could not parse FROM in $MODFILE" >&2
    exit 1
  fi
  echo "→ pulling base model $BASE_MODEL (for $MODEL_NAME)"
  ollama pull "$BASE_MODEL"
  echo "→ creating Ollama model $MODEL_NAME from $MODFILE"
  ollama create "$MODEL_NAME" -f "$MODFILE"
  echo "✓ $MODEL_NAME ready"
done

echo "✓ all models (ollama list)"
