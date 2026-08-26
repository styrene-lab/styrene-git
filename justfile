default:
    just --list --unsorted

format:
    cargo fmt --all

test:
    cargo test --workspace --all-targets

lint:
    cargo clippy --workspace --all-targets -- -D warnings

docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

parity-check:
    python3 scripts/check-heartwood-parity.py

identity-conformance-harness-test:
    python3 scripts/test-check-identity-conformance.py

property:
    cargo test -p styrene-git-core --test parity_property

check: format-check
    cargo check --workspace --all-targets
    just docs
    just parity-check
    just openspec

pre-push: check test lint

parity: parity-check test

validate: format-check test lint openspec

format-check:
    cargo fmt --all -- --check

openspec:
    python3 ~/.agents/skills/openspec/scripts/openspec.py validate bootstrap-self-certifying-repositories

images:
    bash infra/packaging/build-images.sh all

images-verify:
    bash infra/packaging/verify-images.sh all

functional-smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    podman-compose -f compose.functional.yml down -v --remove-orphans >/dev/null 2>&1 || true
    trap 'podman-compose -f compose.functional.yml down -v --remove-orphans >/dev/null 2>&1' EXIT
    mkdir -p artifacts
    if [[ ${SKIP_IMAGE_BUILD:-0} != 1 ]]; then bash infra/packaging/build-images.sh git; fi
    podman-compose -f compose.functional.yml up --abort-on-container-exit --exit-code-from scenario 2>&1 | tee artifacts/compose.log

backbone-smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    compose=(podman-compose -f compose.lab.yml)
    "${compose[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
    trap 'podman-compose -f compose.lab.yml down -v --remove-orphans >/dev/null 2>&1' EXIT
    mkdir -p artifacts
    if [[ ${SKIP_IMAGE_BUILD:-0} != 1 ]]; then bash infra/packaging/build-images.sh all; fi
    "${compose[@]}" up -d hub-daemon alice-daemon bob-daemon carol-daemon alice bob carol
    "${compose[@]}" run --rm backbone-check 2>&1 | tee artifacts/backbone.log
    "${compose[@]}" ps > artifacts/backbone-ps.txt
