#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
STYRENE_RS=${STYRENE_RS:-$ROOT/../styrene-rs}
MELANGE_IMAGE=${MELANGE_IMAGE:-cgr.dev/chainguard/melange@sha256:0bd81188f36664078d16bfc6fffb908b0ae0943a54b40855b37d4a48a96161e7}
APKO_IMAGE=${APKO_IMAGE:-cgr.dev/chainguard/apko@sha256:e5a11fd740d4f1f34caaf6a83acf2e970b9ff2c00cb2e8797a37eaede80e48ab}
TARGET=${1:-all}

case $(uname -m) in
    arm64 | aarch64) ARCH=arm64 ;;
    x86_64 | amd64) ARCH=amd64 ;;
    *) echo "unsupported package architecture: $(uname -m)" >&2; exit 2 ;;
esac

mkdir -p "$ROOT/artifacts/packages" "$ROOT/artifacts/images"
KEY="$ROOT/artifacts/packages/local-melange.rsa"
if [[ ! -f $KEY || ! -f $KEY.pub ]]; then
    podman run --rm \
        -v "$ROOT:/work" \
        "$MELANGE_IMAGE" keygen /work/artifacts/packages/local-melange.rsa
fi

melange_build() {
    local config=$1
    local source=$2
    local stage="$ROOT/artifacts/sources/${config%.melange.yaml}"
    rm -rf "$stage"
    mkdir -p "$stage"
    tar -C "$source" -cf - Cargo.toml Cargo.lock rust-toolchain.toml crates \
        | tar -C "$stage" -xf -
    podman run --rm --privileged \
        -v "$ROOT:/work" \
        -v "$stage:/source:ro" \
        "$MELANGE_IMAGE" build \
        --arch "$ARCH" \
        --source-dir /source \
        --out-dir /work/artifacts/packages \
        --license MIT \
        --signing-key /work/artifacts/packages/local-melange.rsa \
        "/work/infra/packaging/$config"
}

apko_build() {
    local config=$1
    local tag=$2
    local output=$3
    local sbom=${output%.tar}
    rm -f "$ROOT/artifacts/images/$output"
    rm -rf "$ROOT/artifacts/sboms/$sbom"
    mkdir -p "$ROOT/artifacts/sboms/$sbom"
    podman run --rm \
        -v "$ROOT:/work" \
        "$APKO_IMAGE" build \
        --arch "$ARCH" \
        --sbom-path "/work/artifacts/sboms/$sbom" \
        "/work/infra/packaging/$config" \
        "$tag" \
        "/work/artifacts/images/$output"
    podman load -i "$ROOT/artifacts/images/$output"
    if podman image exists "$tag-$ARCH"; then
        podman tag "$tag-$ARCH" "$tag"
    fi
}

build_git() {
    melange_build styrene-git.melange.yaml "$ROOT"
    apko_build styrene-git.apko.yaml localhost/styrene-git-functional:dev styrene-git.tar
    bash "$ROOT/infra/packaging/verify-images.sh" git
}

build_styrened() {
    if [[ ! -d $STYRENE_RS ]]; then
        echo "styrene-rs workspace not found: $STYRENE_RS" >&2
        exit 2
    fi
    melange_build styrened.melange.yaml "$STYRENE_RS"
    apko_build styrened.apko.yaml localhost/styrened-git-lab:dev styrened.tar
    bash "$ROOT/infra/packaging/verify-images.sh" styrened
}

case $TARGET in
    git) build_git ;;
    styrened) build_styrened ;;
    all) build_styrened; build_git ;;
    *) echo "usage: $0 [all|git|styrened]" >&2; exit 2 ;;
esac
