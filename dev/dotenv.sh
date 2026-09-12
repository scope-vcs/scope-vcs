# Sourced by dev/check and dev/scope-dev. Reads one KEY=VALUE entry from a
# dotenv file, trimming whitespace and one layer of matching quotes.
read_dotenv_value() {
  local path="$1"
  local key="$2"
  [[ -f "$path" ]] || return 0

  awk -v key="$key" '
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*$/ { next }
    {
      split($0, parts, "=")
      name = parts[1]
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", name)
      if (name != key) {
        next
      }
      value = substr($0, index($0, "=") + 1)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
      if ((substr(value, 1, 1) == "\"" && substr(value, length(value), 1) == "\"") ||
          (substr(value, 1, 1) == "'"'"'" && substr(value, length(value), 1) == "'"'"'")) {
        value = substr(value, 2, length(value) - 2)
      }
      print value
      exit
    }
  ' "$path"
}
