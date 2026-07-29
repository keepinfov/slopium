#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "${script_dir}/.." && pwd)"
source_dir="${repo_dir}/editors/nvim"
data_dir="${XDG_DATA_HOME:-${HOME}/.local/share}"
config_dir="${XDG_CONFIG_HOME:-${HOME}/.config}"
pack_target="${data_dir}/nvim/site/pack/slopium/start/slopium.nvim"
lazy_source="${source_dir}/lazy-spec.lua"
lazy_target="${config_dir}/nvim/lua/plugins/slopium.lua"

install_link() {
  local source="$1"
  local target="$2"
  if [[ -L "${target}" ]]; then
    local current
    current="$(readlink -- "${target}")"
    if [[ "${current}" == "${source}" ]]; then
      echo "already installed: ${target}"
      return
    fi
    if [[ ! -e "${target}" ]]; then
      ln -sfn -- "${source}" "${target}"
      echo "repaired stale link: ${target} -> ${source}"
      return
    fi
    echo "refusing to replace existing symlink: ${target} -> ${current}" >&2
    exit 1
  fi

  if [[ -e "${target}" ]]; then
    echo "refusing to replace existing path: ${target}" >&2
    exit 1
  fi

  mkdir -p -- "$(dirname -- "${target}")"
  ln -s -- "${source}" "${target}"
  echo "installed: ${target} -> ${source}"
}

install_link "${source_dir}" "${pack_target}"

if [[ -f "${config_dir}/nvim/lazy-lock.json" ]]; then
  install_link "${lazy_source}" "${lazy_target}"
  echo "lazy.nvim detected; local plugin spec installed"
fi
