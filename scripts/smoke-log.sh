#!/usr/bin/env bash
# Shared checks for completed user-mode probes; source from the smoke scripts.
smoke_log_ok() {
    local log=$1 scenario=$2 expected line='' count=0
    case "$scenario" in
        normal) expected='[WOLFRAM_TEST] privilege_switch_roundtrip: PASS' ;;
        isolation) expected='[WOLFRAM_TEST] privilege_isolation: PASS' ;;
        *) return 1 ;;
    esac
    if grep -Eiq 'FAIL|panic' "$log"; then
        return 1
    fi
    while IFS= read -r line; do
        line=${line%$'\r'}
        if [[ "$line" == *'[WOLFRAM_TEST] privilege_'* ]]; then
            [[ "$line" == "$expected" ]] || return 1
            count=$((count + 1))
        fi
    done < "$log"
    # A partial final write must not turn a crashed probe into a passing run.
    [[ -z "$line" && "$count" -eq 1 ]]
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    set -euo pipefail
    [[ "${1:-}" == --self-test ]] || exit 2
    test_log=$(mktemp)
    trap 'rm -f -- "$test_log"' EXIT
    roundtrip='[WOLFRAM_TEST] privilege_switch_roundtrip: PASS'
    isolation='[WOLFRAM_TEST] privilege_isolation: PASS'
    printf '%s\r\n' "$roundtrip" > "$test_log"
    smoke_log_ok "$test_log" normal
    printf '%s\n' "$isolation" > "$test_log"
    smoke_log_ok "$test_log" isolation
    for text in '' 'boot only' "$roundtrip" "$roundtrip"$'\nPANIC\n' \
        "$roundtrip"$'\nFAIL\n' "$isolation"$'\n' "$roundtrip extra"$'\n' \
        "$roundtrip"$'\n'"$roundtrip"$'\n'; do
        printf '%s' "$text" > "$test_log"
        if smoke_log_ok "$test_log" normal; then
            echo 'smoke log self-check accepted invalid output' >&2
            exit 1
        fi
    done
    echo 'Smoke log self-check passed'
fi
