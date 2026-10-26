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

is_timeout_status() {
  local status="$1"
  [[ "$status" -eq 124 || "$status" -eq 137 ]]
}

recover_dpkg() {
  set +e
  sudo DEBIAN_FRONTEND=noninteractive timeout \
    --signal=TERM --kill-after=30s 2m \
    dpkg --configure -a
  local status=$?
  set -e
  if [[ "$status" -ne 0 ]]; then
    echo "::error::dpkg --configure -a failed before APT retry with exit ${status}"
    return "$status"
  fi
}

switch_to_fallback() {
  local sources
  if [[ -f /etc/apt/apt-mirrors.txt ]]; then
    sudo sed -i "/${primary_mirror//./\\.}/d" /etc/apt/apt-mirrors.txt
  fi
  for sources in /etc/apt/sources.list /etc/apt/sources.list.d; do
    if [[ -f "$sources" ]]; then
      sudo sed -i "s|${primary_mirror}|${fallback_mirror}|g" "$sources"
    elif [[ -d "$sources" ]]; then
      while IFS= read -r source_file; do
        sudo sed -i "s|${primary_mirror}|${fallback_mirror}|g" "$source_file"
      done < <(sudo grep -RIl -- "$primary_mirror" "$sources" || true)
    fi
  done

  for sources in \
    /etc/apt/sources.list \
    /etc/apt/sources.list.d \
    /etc/apt/apt-mirrors.txt; do
    if [[ -e "$sources" ]] && sudo grep -Rqs -- "$primary_mirror" "$sources"; then
      echo "::error::fallback could not remove ${primary_mirror} from APT sources"
      return 1
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
  if ! is_timeout_status "$status"; then
    return "$status"
  fi
  if [[ "$current_mirror" != "$primary_mirror" ]]; then
    echo "::error::${label} timed out using fallback mirror ${fallback_mirror} (exit ${status})"
    return "$status"
  fi

  echo "::error::${label} timed out using slow mirror ${primary_mirror} (exit ${status}); retrying once with ${fallback_mirror}"
  switch_to_fallback || return $?
  recover_dpkg || return $?

  if [[ "$label" == "apt-get install" ]]; then
    set +e
    run_apt 2m update
    status=$?
    set -e
    if [[ "$status" -ne 0 ]]; then
      if is_timeout_status "$status"; then
        echo "::error::apt-get update timed out using fallback mirror ${fallback_mirror} (exit ${status})"
      fi
      return "$status"
    fi
  fi

  set +e
  run_apt "$limit" "$@"
  status=$?
  set -e
  if is_timeout_status "$status"; then
    echo "::error::${label} timed out using fallback mirror ${fallback_mirror} (exit ${status})"
  fi
  return "$status"
}

printf '%s\n' 'Binary::apt::APT::Keep-Downloaded-Packages "true";' \
  | sudo tee /etc/apt/apt.conf.d/01keep-debs >/dev/null

run_once_with_fallback "apt-get update" 2m update
mapfile -t packages < .github/apt-packages.txt
run_once_with_fallback "apt-get install" 7m install -y "${packages[@]}"
