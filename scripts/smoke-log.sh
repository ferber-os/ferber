#!/usr/bin/env bash
# Shared checks for completed user-mode probes; source from the smoke scripts.
smoke_log_ok() {
    local log=$1 scenario=$2 expected line='' count=0 handle_count=0 mutation_count=0
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
        if [[ "$line" == "$expected" ]]; then
            count=$((count + 1))
        elif [[ "$scenario" == normal && "$line" == '[WOLFRAM_TEST] handle_syscalls: PASS' ]]; then
            handle_count=$((handle_count + 1))
        elif [[ "$scenario" == normal && "$line" == '[WOLFRAM_TEST] capability_mutation: PASS' ]]; then
            mutation_count=$((mutation_count + 1))
        elif [[ "$line" == *'[WOLFRAM_TEST]'* ]]; then
            return 1
        fi
    done < "$log"
    # A partial final write must not turn a crashed probe into a passing run.
    [[ -z "$line" && "$count" -eq 1 ]] || return 1
    if [[ "$scenario" == normal ]]; then
        [[ "$handle_count" -eq 1 && "$mutation_count" -eq 1 ]]
    else
        [[ "$handle_count" -eq 0 && "$mutation_count" -eq 0 ]]
    fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    set -euo pipefail
    [[ "${1:-}" == --self-test ]] || exit 2
    test_log=$(mktemp)
    trap 'rm -f -- "$test_log"' EXIT
    markers=(privilege_switch_roundtrip privilege_isolation privilege_write_rx privilege_execute_nx)
    scenarios=(normal isolation write-rx execute-nx)
    handle_marker='[WOLFRAM_TEST] handle_syscalls: PASS'
    mutation_marker='[WOLFRAM_TEST] capability_mutation: PASS'
    for index in "${!scenarios[@]}"; do
        scenario=${scenarios[index]}
        expected="[WOLFRAM_TEST] ${markers[index]}: PASS"
        valid="$expected"$'\n'
        [[ "$scenario" != normal ]] || valid+="$handle_marker"$'\n'"$mutation_marker"$'\n'
        printf '%s' "$valid" > "$test_log"
        smoke_log_ok "$test_log" "$scenario"
        printf '%s' "${valid//$'\n'/$'\r\n'}" > "$test_log"
        smoke_log_ok "$test_log" "$scenario"
        for text in '' 'boot only' "${valid%$'\n'}" "$valid"$'PANIC\n' \
            "$valid"$'FAIL\n' "$valid"$'truncated' "$expected extra"$'\n' \
            "$valid$expected"$'\n' "$valid$handle_marker"$'\n' \
            "$valid$mutation_marker"$'\n' "$mutation_marker"$'\n' \
            "$handle_marker"$'\n' "$valid"$'[WOLFRAM_TEST] unknown: PASS\n'; do
            printf '%s' "$text" > "$test_log"
            if smoke_log_ok "$test_log" "$scenario"; then
                echo 'smoke log self-check accepted invalid output' >&2
                exit 1
            fi
        done
        if [[ "$scenario" == normal ]]; then
            printf '%s\n' "$expected" > "$test_log"
            if smoke_log_ok "$test_log" normal; then
                echo 'smoke log self-check accepted missing handle evidence' >&2
                exit 1
            fi
            printf '%s\n%s\n' "$expected" "$handle_marker" > "$test_log"
            if smoke_log_ok "$test_log" normal; then
                echo 'smoke log self-check accepted missing mutation evidence' >&2
                exit 1
            fi
            printf '%s\n%s\n%s\n' "$mutation_marker" "$handle_marker" "$expected" > "$test_log"
            smoke_log_ok "$test_log" normal
        fi
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
