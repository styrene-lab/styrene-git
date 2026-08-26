#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
TARGET=${1:-all}

case $(uname -m) in
    arm64 | aarch64) ARCH=arm64; APK_ARCH=aarch64 ;;
    x86_64 | amd64) ARCH=amd64; APK_ARCH=x86_64 ;;
    *) echo "unsupported image architecture: $(uname -m)" >&2; exit 2 ;;
esac

verify_sbom() {
    local directory=$1
    local package=$2
    python3 - "$directory" "$package" <<'PY'
import glob
import json
import os
import sys

directory, expected = sys.argv[1:]
index = os.path.join(directory, "sbom-index.spdx.json")
documents = [path for path in glob.glob(os.path.join(directory, "sbom-*.spdx.json")) if path != index]
if not os.path.isfile(index) or not documents:
    raise SystemExit(f"missing APKO SPDX documents in {directory}")

found = False
for path in [index, *documents]:
    with open(path, encoding="utf-8") as stream:
        document = json.load(stream)
    if document.get("spdxVersion") != "SPDX-2.3":
        raise SystemExit(f"unexpected SPDX version in {path}")
    found |= expected in {package.get("name") for package in document.get("packages", [])}
if not found:
    raise SystemExit(f"package {expected} absent from APKO SPDX documents")
PY
}

verify_metadata() {
    local image=$1
    local archive=$2
    local entrypoint=$3
    local package=$4
    local sbom=$5
    [[ -s $ROOT/artifacts/images/$archive ]] || {
        echo "missing OCI image archive: artifacts/images/$archive" >&2
        exit 1
    }
    [[ -s $ROOT/artifacts/packages/$APK_ARCH/APKINDEX.tar.gz ]]
    [[ -s $ROOT/artifacts/packages/local-melange.rsa.pub ]]
    compgen -G "$ROOT/artifacts/packages/$APK_ARCH/$package-*.apk" >/dev/null
    podman image exists "$image" || {
        echo "missing loaded image: $image" >&2
        exit 1
    }
    [[ $(podman image inspect "$image" --format '{{.Os}}') == linux ]]
    [[ $(podman image inspect "$image" --format '{{.Architecture}}') == "$ARCH" ]]
    [[ $(podman image inspect "$image" --format '{{json .Config.Entrypoint}}') == "[\"$entrypoint\"]" ]]
    verify_sbom "$ROOT/artifacts/sboms/$sbom" "$package"
}

verify_git() {
    verify_metadata \
        localhost/styrene-git-functional:dev \
        styrene-git.tar \
        /usr/bin/styrene-git-lab \
        styrene-git-functional \
        styrene-git
    local output
    local status
    set +e
    output=$(podman run --rm --entrypoint /usr/bin/styrene-git-lab \
        localhost/styrene-git-functional:dev 2>&1)
    status=$?
    set -e
    [[ $status == 1 ]]
    [[ $output == *"usage: styrene-git-lab operator"* ]]
}

verify_styrened() {
    verify_metadata \
        localhost/styrened-git-lab:dev \
        styrened.tar \
        /usr/bin/styrened \
        styrened-git-lab \
        styrened
    podman run --rm --entrypoint /bin/busybox localhost/styrened-git-lab:dev \
        sh -c 'test -x /usr/bin/styrened && test -x /usr/bin/styrene && command -v grep >/dev/null && command -v sleep >/dev/null'
    podman run --rm --entrypoint /usr/bin/styrened localhost/styrened-git-lab:dev --version
    podman run --rm --entrypoint /usr/bin/styrene localhost/styrened-git-lab:dev --version
}

case $TARGET in
    git) verify_git ;;
    styrened) verify_styrened ;;
    all) verify_styrened; verify_git ;;
    *) echo "usage: $0 [all|git|styrened]" >&2; exit 2 ;;
esac

echo "OCI image verification passed: $TARGET"
