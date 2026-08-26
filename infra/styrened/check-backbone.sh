#!/usr/bin/env bash
set -euo pipefail
export NO_COLOR=1

HUB=/run/hub/control.sock
ALICE=/run/alice/control.sock
BOB=/run/bob/control.sock
CAROL=/run/carol/control.sock
TIMEOUT=${BACKBONE_TIMEOUT_SECONDS:-120}
MARKER=styrene-git-backbone-smoke

wait_for_command() {
    local socket=$1
    local command=$2
    local deadline=$((SECONDS + TIMEOUT))
    while ((SECONDS < deadline)); do
        if styrene --socket "$socket" $command >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    echo "timeout waiting for '$command' on $socket" >&2
    return 1
}

wait_for_peer() {
    local socket=$1
    local peer=$2
    local deadline=$((SECONDS + TIMEOUT))
    local peers
    while ((SECONDS < deadline)); do
        peers=$(styrene --socket "$socket" peers 2>&1) || peers=""
        if grep -qiF -- "$peer" <<<"$peers"; then
            return 0
        fi
        sleep 1
    done
    echo "timeout waiting for peer '$peer' on $socket" >&2
    styrene --socket "$socket" peers >&2 || true
    return 1
}

for socket in "$HUB" "$ALICE" "$BOB" "$CAROL"; do
    wait_for_command "$socket" status
done

for peer in alice bob carol; do
    wait_for_peer "$HUB" "$peer"
done
wait_for_peer "$ALICE" bob
wait_for_peer "$BOB" alice

alice_identity=$(styrene --socket "$ALICE" identity 2>&1)
bob_identity=$(styrene --socket "$BOB" identity 2>&1)
carol_identity=$(styrene --socket "$CAROL" identity 2>&1)
alice_hash=$(awk '/hash/ && !/dest|lxmf/ {print $2; exit}' <<<"$alice_identity")
alice_lxmf=$(awk '/lxmf/ {print $2; exit}' <<<"$alice_identity")
bob_lxmf=$(awk '/lxmf/ {print $2; exit}' <<<"$bob_identity")
carol_lxmf=$(awk '/lxmf/ {print $2; exit}' <<<"$carol_identity")

if [[ -z $alice_hash || -z $alice_lxmf || -z $bob_lxmf || -z $carol_lxmf ]]; then
    echo "daemon identity output did not contain required hashes" >&2
    exit 1
fi

styrene --socket "$ALICE" send "$bob_lxmf" "$MARKER" >/dev/null 2>&1
deadline=$((SECONDS + TIMEOUT))
delivered=false
while ((SECONDS < deadline)); do
    messages=$(styrene --socket "$BOB" messages "$alice_hash" --limit 20 2>&1) \
        || messages=""
    if grep -qF -- "$MARKER" <<<"$messages"; then
        delivered=true
        break
    fi
    sleep 2
done
if [[ $delivered != true ]]; then
    echo "Alice-to-Bob LXMF delivery timed out" >&2
    exit 1
fi

mkdir -p /artifacts
jq -n \
    --arg alice "$alice_lxmf" \
    --arg bob "$bob_lxmf" \
    --arg carol "$carol_lxmf" \
    --arg marker "$MARKER" \
    '{
        scenario: "styrened-backbone",
        topology: "hub-and-three-full-nodes",
        ipc: {hub: "passed", alice: "passed", bob: "passed", carol: "passed"},
        discovery: {hub_sees: ["alice", "bob", "carol"], alice_sees: ["bob"], bob_sees: ["alice"]},
        lxmf_destinations: {alice: $alice, bob: $bob, carol: $carol},
        delivery: {source: "alice", destination: "bob", marker: $marker, result: "passed"},
        git_transport_adapter: "not_yet_connected",
        status: "passed"
    }' > /artifacts/backbone.json

echo "styrened backbone passed: discovery and Alice-to-Bob LXMF delivery"
