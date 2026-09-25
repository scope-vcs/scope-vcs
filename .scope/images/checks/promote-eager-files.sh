#!/bin/bash
# Rewrites files into the current image layer. The SOCI publisher fetches
# layers under its size cutoff before the task starts, so the rewritten copies
# are available without lazy fetches.
#
# Usage: promote-eager-files.sh [--libraries-of BINARY | DIRECTORY | FILE]...
# Symlinks resolve to their targets. A directory promotes every regular file
# below it. --libraries-of promotes BINARY and every shared library it loads.
set -euo pipefail

declare -A promoted=()

add() {
  local file
  file="$(realpath -e "$1")"
  [[ -f "$file" ]] || { echo "not a regular file: $1" >&2; exit 1; }
  promoted["$file"]=1
}

while (($#)); do
  case "$1" in
    --libraries-of)
      binary="$2"
      shift 2
      libraries="$(ldd "$binary")"
      if grep --quiet 'not found' <<<"$libraries"; then
        echo "unresolved library for $binary:" >&2
        echo "$libraries" >&2
        exit 1
      fi
      add "$binary"
      while IFS= read -r library; do
        add "$library"
      done < <(awk '$3 ~ /^\// {print $3} $1 ~ /^\// {print $1}' <<<"$libraries")
      ;;
    *)
      if [[ -d "$1" ]]; then
        listing="$(mktemp)"
        find "$1" -type f -print0 >"$listing"
        mapfile -d '' -t files <"$listing"
        rm "$listing"
        ((${#files[@]})) || { echo "no files in $1" >&2; exit 1; }
        for file in "${files[@]}"; do
          add "$file"
        done
      else
        add "$1"
      fi
      shift
      ;;
  esac
done

((${#promoted[@]})) || { echo "no files to promote" >&2; exit 1; }

for file in "${!promoted[@]}"; do
  cp -p "$file" "$file.eager"
  mv "$file.eager" "$file"
  # Some BuildKit differs skip files whose metadata is unchanged.
  touch "$file"
done
