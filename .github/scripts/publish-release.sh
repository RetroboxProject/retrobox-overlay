#!/usr/bin/env bash
# Publishes the packages of a build as a GitHub Release (the gh CLI).
#
#   publish-release.sh continuous DIR VERSION   the continuous pre-release
#   publish-release.sh tag TAG DIR [--prerelease]   a release of the tag
#
# DIR holds the files to publish; SHA256SUMS is made here, over all of them
# (bare names, one line each), so that the three repositories need not do it
# and retrobox-update, install.sh and the web installer verify the same way.
# GH_TOKEN (contents: write), GITHUB_REPOSITORY and GITHUB_SHA come from the
# workflow. The same file is in the three Retrobox repositories (core,
# overlay, system-settings): keep the copies equal.
#
# The continuous release is always the tag "continuous": it is deleted and made
# again on the new commit, so that there is one release (pre-release, so
# /releases/latest never offers it) and no tag of its own per push. Its
# packages carry the version of continuous-version.sh, and the newest of the
# repository's releases is the one retrobox-update --allow-prereleases takes.
set -euo pipefail

die() {
    printf 'publish-release: %s\n' "$*" >&2
    exit 1
}

checksums() {
    local dir="$1"
    (
        cd "${dir}"
        rm -f SHA256SUMS
        # listed before the file exists (the redirection would make it first)
        mapfile -t names < <(find . -maxdepth 1 -type f -printf '%P\n' | sort)
        sha256sum -- "${names[@]}" > SHA256SUMS
        cat SHA256SUMS
    )
}

mode="${1:-}"
case "${mode}" in
    continuous)
        dir="${2:?usage: publish-release.sh continuous DIR VERSION}"
        version="${3:?usage: publish-release.sh continuous DIR VERSION}"
        tag=continuous
        ;;
    tag)
        tag="${2:?usage: publish-release.sh tag TAG DIR [--prerelease]}"
        dir="${3:?usage: publish-release.sh tag TAG DIR [--prerelease]}"
        prerelease="${4:-}"
        [[ -z "${prerelease}" || "${prerelease}" == --prerelease ]] || die "unknown option: ${prerelease}"
        ;;
    *) die "usage: publish-release.sh continuous DIR VERSION | tag TAG DIR [--prerelease]" ;;
esac
[[ -d "${dir}" ]] || die "${dir}: no such directory"
repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is not set}"
sha="${GITHUB_SHA:?GITHUB_SHA is not set}"

checksums "${dir}"
mapfile -t files < <(find "${dir}" -maxdepth 1 -type f | sort)
((${#files[@]} > 0)) || die "${dir} has no files"

if [[ "${mode}" == continuous ]]; then
    # a release made again, not edited: its date is the push's, and the tag
    # moves to the commit
    gh release delete continuous --repo "${repo}" --cleanup-tag --yes 2> /dev/null || true
    notes="Continuous build of ${sha::7} (version ${version}), made by every push to main."
    notes+=$'\n\n'"Not a release: it is replaced by the next push. Install it over whatever is there with"
    notes+=" \`retrobox-update --allow-prereleases\` on a console, or with the packages below."
    gh release create continuous --repo "${repo}" --target "${sha}" --prerelease \
        --title "Continuous build (${version})" --notes "${notes}" "${files[@]}"
elif gh release view "${tag}" --repo "${repo}" > /dev/null 2>&1; then
    gh release upload "${tag}" "${files[@]}" --repo "${repo}" --clobber
else
    flags=(--target "${sha}" --title "${tag}" --generate-notes)
    [[ -z "${prerelease:-}" ]] || flags+=(--prerelease)
    gh release create "${tag}" --repo "${repo}" "${flags[@]}" "${files[@]}"
fi
