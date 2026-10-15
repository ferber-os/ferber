#!/usr/bin/env bash
# Shared checks for completed user-mode probes; source from the smoke scripts.
smoke_log_ok() {
    local log=$1 scenario=$2 expected line='' count=0
    case "$scenario" in
        normal) expected='[WOLFRAM_TEST] privilege_switch_roundtrip: PASS' ;;
        isolation) expected='[WOLFRAM_TEST] privilege_isolation: PASS' ;;
        write-rx) expected='[WOLFRAM_TEST] privilege_write_rx: PASS' ;;
        execute-nx) expected='[WOLFRAM_TEST] privilege_execute_nx: PASS' ;;
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
    markers=(privilege_switch_roundtrip privilege_isolation privilege_write_rx privilege_execute_nx)
    scenarios=(normal isolation write-rx execute-nx)
    for index in "${!scenarios[@]}"; do
        scenario=${scenarios[index]}
        expected="[WOLFRAM_TEST] ${markers[index]}: PASS"
        printf '%s\n' "$expected" > "$test_log"
        smoke_log_ok "$test_log" "$scenario"
        printf '%s\r\n' "$expected" > "$test_log"
        smoke_log_ok "$test_log" "$scenario"
        for text in '' 'boot only' "$expected" "$expected"$'\nPANIC\n' \
            "$expected"$'\nFAIL\n' "$expected extra"$'\n' \
            "$expected"$'\n'"$expected"$'\n'; do
            printf '%s' "$text" > "$test_log"
            if smoke_log_ok "$test_log" "$scenario"; then
                echo 'smoke log self-check accepted invalid output' >&2
                exit 1
            fi
        done
        for other in "${!markers[@]}"; do
            [[ "$index" != "$other" ]] || continue
            printf '[WOLFRAM_TEST] %s: PASS\n' "${markers[other]}" > "$test_log"
            if smoke_log_ok "$test_log" "$scenario"; then
                echo 'smoke log self-check accepted the wrong scenario' >&2
                exit 1
            fi
        done
    done
    echo 'Smoke log self-check passed'
fi
