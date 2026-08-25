pub fn zsh_init_script() -> &'static str {
    r#"# reout zsh integration
# Install with: eval "$(reout init zsh)"
#
# This ZLE widget runs the accepted command through `reout capture`.
# It is local-only and skips commands beginning with `reout`.

function _reout_accept_line() {
  emulate -L zsh
  local cmd="$BUFFER"

  if [[ -z "${cmd//[[:space:]]/}" || "$cmd" == reout(|[[:space:]]*) ]]; then
    zle .accept-line
    return
  fi

  print -s -- "$cmd"
  BUFFER="reout capture -- ${(q)cmd}"
  zle .accept-line
}

zle -N accept-line _reout_accept_line
"#
}
