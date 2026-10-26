#!/usr/bin/env bash

set -euo pipefail

readonly primary_mirror="azure.archive.ubuntu.com"
readonly fallback_mirror="archive.ubuntu.com"
current_mirror="$primary_mirror"
readonly -a apt_options=(
  -o Acquire::Retries=3
  -o Acquire::http::Timeout=20
  -o Acquire::https::Timeout=20
  -o DPkg::Lock::Timeout=60
)

run_apt() {
  local limit="$1"
  shift
  sudo DEBIAN_FRONTEND=noninteractive timeout \
    --signal=TERM --kill-after=30s "$limit" \
    apt-get "${apt_options[@]}" "$@"
}

switch_to_fallback() {
  local sources
  for sources in /etc/apt/sources.list /etc/apt/sources.list.d/ubuntu.sources; do
    if [[ -f "$sources" ]]; then
      sudo sed -i "s|${primary_mirror}|${fallback_mirror}|g" "$sources"
    fi
  done
  current_mirror="$fallback_mirror"
}

run_once_with_fallback() {
  local label="$1"
  local limit="$2"
  shift 2

  set +e
  run_apt "$limit" "$@"
  local status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    return 0
  fi
  if [[ "$status" -ne 124 ]]; then
    return "$status"
  fi
  if [[ "$current_mirror" != "$primary_mirror" ]]; then
    echo "::error::${label} timed out using fallback mirror ${fallback_mirror}"
    return 124
  fi

  echo "::error::${label} timed out using slow mirror ${primary_mirror}; retrying once with ${fallback_mirror}"
  switch_to_fallback

  if [[ "$label" == "apt-get install" ]]; then
    set +e
    run_apt 2m update
    status=$?
    set -e
    if [[ "$status" -ne 0 ]]; then
      if [[ "$status" -eq 124 ]]; then
        echo "::error::apt-get update timed out using fallback mirror ${fallback_mirror}"
      fi
      return "$status"
    fi
  fi

  set +e
  run_apt "$limit" "$@"
  status=$?
  set -e
  if [[ "$status" -eq 124 ]]; then
    echo "::error::${label} timed out using fallback mirror ${fallback_mirror}"
  fi
  return "$status"
}

printf '%s\n' 'Binary::apt::APT::Keep-Downloaded-Packages "true";' \
  | sudo tee /etc/apt/apt.conf.d/01keep-debs >/dev/null

run_once_with_fallback "apt-get update" 2m update
mapfile -t packages < .github/apt-packages.txt
run_once_with_fallback "apt-get install" 7m install -y "${packages[@]}"
