#!/usr/bin/env bash
# The version of a continuous build: the last release's, with the build time
# as one more number.
#
#   .github/scripts/continuous-version.sh [REPO_DIR]    ->   0.7.9.20261126093012
#
# The last release is the highest tag vX.Y or vX.Y.Z (a missing patch is 0;
# no tag at all: 0.0.0). The same rule is in the three Retrobox repositories
# (core, overlay, system-settings): keep the copies equal.
#
# That number is above the last release's (a package with one more component
# is newer, in dpkg and in rpm) and below the next one's (0.7.9.2026... <
# 0.7.10 and < 0.8.0), so a continuous build installs over the release, each
# build over the one before, and the next release over all of them. It is
# digits and dots only: GitHub rewrites any other character in the name of an
# asset (~ and + become dots), which would no longer match SHA256SUMS.
#
# CONTINUOUS_STAMP sets the time (tests); the tags have to be fetched.
set -euo pipefail

repo="${1:-.}"
last="$(git -C "${repo}" tag --list 'v[0-9]*' |
    grep -E '^v[0-9]+\.[0-9]+(\.[0-9]+)?$' | sort -V | tail -n 1 || true)"
base="${last#v}"
base="${base:-0.0.0}"
[[ "${base}" == *.*.* ]] || base="${base}.0"
printf '%s.%s\n' "${base}" "${CONTINUOUS_STAMP:-$(date -u +%Y%m%d%H%M%S)}"
