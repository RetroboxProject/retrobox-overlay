#!/usr/bin/env bash
# Build the retrobox-overlay .deb and/or .rpm for the architecture of this machine.
#
#   scripts/package.sh [--version VERSION] [--outdir DIR] [deb|rpm|all]
#
# Needs: cargo (Rust >= rust-version in Cargo.toml), cargo-deb for the .deb,
# cargo-generate-rpm for the .rpm, and the libxkbcommon development files.
# The package metadata (dependencies, assets) lives in Cargo.toml.
#
# --version  vMAJOR.MINOR[.PATCH] or MAJOR.MINOR[.PATCH]; a missing patch is 0.
#            A fourth numeric component (MAJOR.MINOR.PATCH.STAMP, used by the
#            continuous builds) goes only into the package version: the .deb
#            version is the whole string and the .rpm release is 1.STAMP.
#            MAJOR.MINOR.PATCH is written into Cargo.toml and Cargo.lock for
#            the build and restored afterwards. Empty or absent: keep the
#            Cargo.toml version.
# --outdir   Where the packages go (default: dist/ in the repository).
# deb|rpm|all  Which packages to build (default: all).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd -P)"
SRC="$(cd "${HERE}/.." >/dev/null 2>&1 && pwd -P)"

VERSION=""
OUTDIR="${SRC}/dist"
FORMAT="all"
BACKUP_DIR=""
STAMP=""

die() {
  echo "package.sh: $*" >&2
  exit 1
}

usage() {
  sed -n '2,18p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

# Print the version as MAJOR.MINOR.PATCH[-pre][+build] or MAJOR.MINOR.PATCH.STAMP, or die.
normalize_version() {
  local version="${1#v}"
  case "${version}" in
    *.*.*) ;;
    *.*) version="${version}.0" ;;
    *) version="${version}.0.0" ;;
  esac
  [[ "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+|[-+][0-9A-Za-z.-]+)?$ ]] \
    || die "'${1}' is not a version (vMAJOR.MINOR[.PATCH])"
  printf '%s\n' "${version}"
}

restore_manifests() {
  if [[ -n "${BACKUP_DIR}" ]]; then
    cp -p "${BACKUP_DIR}/Cargo.toml" "${BACKUP_DIR}/Cargo.lock" "${SRC}/"
    rm -rf "${BACKUP_DIR}"
  fi
}

# Write the version into Cargo.toml and refresh only the crate's own entry in
# Cargo.lock (--workspace leaves every dependency where the lock has it).
apply_version() {
  local version="$1"
  BACKUP_DIR="$(mktemp -d)"
  cp -p "${SRC}/Cargo.toml" "${SRC}/Cargo.lock" "${BACKUP_DIR}/"
  trap restore_manifests EXIT
  sed -i "0,/^version = .*/s//version = \"${version}\"/" "${SRC}/Cargo.toml"
  (cd "${SRC}" && cargo update --workspace --quiet)
}

while (($# > 0)); do
  case "$1" in
    --version)
      (($# >= 2)) || die "--version needs a value"
      VERSION="$2"
      shift 2
      ;;
    --outdir)
      (($# >= 2)) || die "--outdir needs a value"
      OUTDIR="$2"
      shift 2
      ;;
    deb | rpm | all)
      FORMAT="$1"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "unknown option: $1" ;;
  esac
done

command -v cargo >/dev/null 2>&1 || die "cargo not found"
if [[ "${FORMAT}" != rpm ]]; then
  cargo deb --version >/dev/null 2>&1 \
    || die "cargo-deb not found (cargo install cargo-deb --locked)"
fi
if [[ "${FORMAT}" != deb ]]; then
  cargo generate-rpm --version >/dev/null 2>&1 \
    || die "cargo-generate-rpm not found (cargo install cargo-generate-rpm --locked)"
fi

mkdir -p "${OUTDIR}"
OUTDIR="$(cd "${OUTDIR}" >/dev/null 2>&1 && pwd -P)"

cd "${SRC}"
if [[ -n "${VERSION}" ]]; then
  # (a separate assignment, so that a version that does not validate stops the script)
  VERSION="$(normalize_version "${VERSION}")"
  CARGO_VERSION="${VERSION}"
  if [[ "${VERSION}" =~ ^([0-9]+\.[0-9]+\.[0-9]+)\.([0-9]+)$ ]]; then
    CARGO_VERSION="${BASH_REMATCH[1]}"
    STAMP="${BASH_REMATCH[2]}"
  fi
  apply_version "${CARGO_VERSION}"
fi

cargo build --release --locked

if [[ "${FORMAT}" != rpm ]]; then
  deb_args=()
  [[ -n "${STAMP}" ]] && deb_args+=(--deb-version "${VERSION}")
  cargo deb --no-build "${deb_args[@]}" --output "${OUTDIR}/"
fi
if [[ "${FORMAT}" != deb ]]; then
  rpm_args=()
  [[ -n "${STAMP}" ]] && rpm_args+=(--set-metadata "release = \"1.${STAMP}\"")
  cargo generate-rpm "${rpm_args[@]}" --output "${OUTDIR}/"
fi

echo "package.sh: packages in ${OUTDIR}:"
ls -l "${OUTDIR}"
