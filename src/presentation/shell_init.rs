pub fn zsh_init_script() -> &'static str {
    r##"# reout zsh integration
# Install with: eval "$(reout init zsh)"
#
# This ZLE widget routes accepted commands through reout.
# It is local-only and skips commands beginning with `reout`.

function _reout_accept_line() {
  emulate -L zsh
  local cmd="$BUFFER"

  if [[ -n "$REOUT_DISABLED" || -z "${cmd//[[:space:]]/}" || "$cmd" == reout(|[[:space:]]*) ]]; then
    zle .accept-line
    return
  fi

  if _reout_should_skip "$cmd"; then
    zle .accept-line
    return
  fi

  print -s -- "$cmd"
  if _reout_should_capture_in_current_shell "$cmd"; then
    BUFFER="_reout_capture_current_shell ${(q)cmd}"
  else
    BUFFER="reout capture -- ${(q)cmd}"
  fi
  zle .accept-line
}

function _reout_should_capture_in_current_shell() {
  emulate -L zsh
  local cmd="${1#"${1%%[![:space:]]*}"}"
  local first="${cmd%%[[:space:];|&()<>]*}"

  (( $+aliases[$first] || $+functions[$first] )) && return 0

  return 1
}

function _reout_capture_current_shell() {
  emulate -L zsh
  local cmd="$1"
  local cwd="$PWD"
  local tmp
  tmp="$(mktemp "${TMPDIR:-/tmp}/reout.XXXXXX")" || return 1

  { eval "$cmd" } > >(tee "$tmp") 2>&1
  local exit_code=$?

  reout import --command "$cmd" --cwd "$cwd" --exit-code "$exit_code" < "$tmp"
  local import_code=$?
  command rm -f "$tmp"

  (( import_code == 0 )) || return "$import_code"
  return "$exit_code"
}

function _reout_should_skip() {
  emulate -L zsh
  local cmd="${1#"${1%%[![:space:]]*}"}"
  local first="${cmd%%[[:space:];|&()<>]*}"

  [[ "$cmd" == *$'\n'* ]] && return 0
  [[ "$cmd" == *'&' ]] && return 0
  [[ "$cmd" == *=* && "$cmd" != *[[:space:]]* ]] && return 0
  [[ "$cmd" == function[[:space:]]* || "$cmd" == *'()'*'{'* ]] && return 0

  case "$first" in
    cd|pushd|popd|dirs|pwd|export|unset|set|setopt|unsetopt|typeset|local|readonly|alias|unalias|source|.|eval|exec|exit|logout|jobs|fg|bg|wait|disown|bindkey|zle|autoload|hash|rehash)
      return 0
      ;;
    vi|vim|nvim|view|less|more|most|top|htop|btop|ssh|sftp|ftp|fzf|watch|man|tmux|screen)
      return 0
      ;;
  esac

  return 1
}

zle -N accept-line _reout_accept_line
"##
}

pub fn bash_init_script() -> &'static str {
    r##"# reout bash integration is not implemented yet.
# zsh is the supported transparent shell integration in this release.
echo "reout: bash integration is not implemented yet" >&2
"##
}

pub fn fish_init_script() -> &'static str {
    r##"# reout fish integration is not implemented yet.
# zsh is the supported transparent shell integration in this release.
echo "reout: fish integration is not implemented yet" >&2
"##
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zsh_init_contains_capture_paths_and_safety_skips() {
        let script = zsh_init_script();

        assert!(script.contains("_reout_accept_line"));
        assert!(script.contains("reout capture --"));
        assert!(script.contains("reout import --command"));
        assert!(script.contains("REOUT_DISABLED"));
        assert!(script.contains("cd|pushd|popd"));
        assert!(script.contains("vi|vim|nvim"));
    }

    #[test]
    fn bash_and_fish_init_are_explicitly_not_implemented() {
        assert!(bash_init_script().contains("bash integration is not implemented yet"));
        assert!(fish_init_script().contains("fish integration is not implemented yet"));
    }
}
