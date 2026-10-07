# shellcheck shell=bash
# Activate mise for bash when the first-start apps step installed it (own file, not root).
if [ -n "${BASH_VERSION:-}" ] && [ "$(id -u)" -ne 0 ] && [ -O "$HOME/.local/bin/mise" ]; then
    eval "$("$HOME/.local/bin/mise" activate bash)"
fi
