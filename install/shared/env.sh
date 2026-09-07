# Shared env helpers for greenpng install/release scripts (GR_* naming only).
# greenpng 1.0.0 is a clean break — legacy GV*_* alias reads were removed.

# gr_env NAME [default] — GR_<NAME> if set, else default/empty.
gr_env() {
  local v
  v="${GR_$1:-}"
  if [[ -n "$v" ]]; then printf '%s' "$v"; return 0; fi
  if [[ $# -ge 2 ]]; then printf '%s' "$2"; fi
}
