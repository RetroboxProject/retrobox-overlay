#!/usr/bin/env bash
# Tests of continuous-version.sh and publish-release.sh (no GitHub: gh is a
# stand-in that records its arguments). Same file in the three repositories.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
failed=0
check() {
    if [[ "$2" == "$3" ]]; then
        printf 'ok - %s\n' "$1"
    else
        printf 'not ok - %s: wanted [%s], got [%s]\n' "$1" "$3" "$2"
        failed=1
    fi
}

# --- continuous-version.sh
repo="${WORK}/repo"
git init -q "${repo}"
git -C "${repo}" -c user.name=t -c user.email=t@t commit -q --allow-empty -m one
export CONTINUOUS_STAMP=20261126093012
check "no tag at all" "$("${HERE}/continuous-version.sh" "${repo}")" "0.0.0.20261126093012"
for tag in v0.7.9 v0.7.10 v0.7.8 continuous v0.7.11-rc1 vnext; do git -C "${repo}" tag "${tag}"; done
check "the highest tag, numerically, and no other kind of tag" \
    "$("${HERE}/continuous-version.sh" "${repo}")" "0.7.10.20261126093012"
git -C "${repo}" tag v0.8
check "a tag with no patch gets 0" "$("${HERE}/continuous-version.sh" "${repo}")" "0.8.0.20261126093012"
if command -v dpkg > /dev/null; then
    last="$("${HERE}/continuous-version.sh" "${repo}")"
    dpkg --compare-versions "${last}" gt 0.8 && ok=1 || ok=0
    check "above the last release (dpkg)" "${ok}" 1
    dpkg --compare-versions "${last}" lt 0.8.1 && ok=1 || ok=0
    check "below the next patch release (dpkg)" "${ok}" 1
    CONTINUOUS_STAMP=20261126093013 dpkg --compare-versions \
        "$(CONTINUOUS_STAMP=20261126093013 "${HERE}/continuous-version.sh" "${repo}")" gt "${last}" && ok=1 || ok=0
    check "a later build is above an earlier one (dpkg)" "${ok}" 1
fi

# --- publish-release.sh, with a gh that only writes down what it is asked
mkdir "${WORK}/bin" "${WORK}/dist"
cat > "${WORK}/bin/gh" << 'GH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${GH_LOG}"
[[ "$1 $2" == "release view" ]] && exit "${GH_VIEW_STATUS:-1}"
exit 0
GH
chmod +x "${WORK}/bin/gh"
export PATH="${WORK}/bin:${PATH}" GH_LOG="${WORK}/log" GITHUB_REPOSITORY=o/r GITHUB_SHA=0123456789abcdef
printf 'a' > "${WORK}/dist/a.deb"
printf 'b' > "${WORK}/dist/b.rpm"
: > "${GH_LOG}"
"${HERE}/publish-release.sh" continuous "${WORK}/dist" 0.7.9.1 > /dev/null || failed=1
check "the sums name every file with a bare name" \
    "$(awk '{print $2}' "${WORK}/dist/SHA256SUMS" | tr '\n' ' ')" "a.deb b.rpm "
check "the continuous release is deleted first" "$(sed -n 1p "${GH_LOG}")" \
    "release delete continuous --repo o/r --cleanup-tag --yes"
check "and made again as a pre-release on the commit" \
    "$(sed -n 2p "${GH_LOG}" | grep -c -- '--target 0123456789abcdef --prerelease')" 1
check "with every file, SHA256SUMS included" \
    "$(tr ' ' '\n' < "${GH_LOG}" | grep -c "^${WORK}/dist/\(a.deb\|b.rpm\|SHA256SUMS\)\$")" 3
: > "${GH_LOG}"
"${HERE}/publish-release.sh" tag v1.2.3 "${WORK}/dist" --prerelease > /dev/null || failed=1
check "a new tag is created, as a pre-release when asked" \
    "$(grep -c '^release create v1.2.3 .*--prerelease' "${GH_LOG}")" 1
: > "${GH_LOG}"
GH_VIEW_STATUS=0 "${HERE}/publish-release.sh" tag v1.2.3 "${WORK}/dist" > /dev/null || failed=1
check "an existing tag gets its files replaced" "$(grep -c '^release upload v1.2.3 ' "${GH_LOG}")" 1
exit "${failed}"
