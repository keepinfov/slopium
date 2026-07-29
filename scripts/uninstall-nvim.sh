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

remove_link() {
  local source="$1"
  local target="$2"
  if [[ ! -L "${target}" ]]; then
    return
  fi

  local current
  current="$(readlink -- "${target}")"
  if [[ "${current}" != "${source}" ]]; then
    echo "refusing to remove foreign symlink: ${target} -> ${current}" >&2
    exit 1
  fi

  unlink -- "${target}"
  echo "removed: ${target}"
}

remove_link "${source_dir}" "${pack_target}"
remove_link "${lazy_source}" "${lazy_target}"
