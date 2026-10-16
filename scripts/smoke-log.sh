#!/usr/bin/env bash
# Shared checks for completed user-mode probes; source from the smoke scripts.
smoke_log_ok() {
    local log=$1 scenario=$2 expected line='' count=0 handle_count=0 mutation_count=0 memory_count=0
    [[ "$scenario" != capacity ]] || scenario=normal
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
        elif [[ "$line" == '[WOLFRAM_TEST] memory_verification: PASS' ]]; then
            memory_count=$((memory_count + 1))
        elif [[ "$line" == *'[WOLFRAM_TEST]'* ]]; then
            return 1
        fi
    done < "$log"
    # A partial final write must not turn a crashed probe into a passing run.
    [[ -z "$line" && "$count" -eq 1 && "$memory_count" -eq 1 ]] || return 1
    if [[ "$scenario" == normal ]]; then
        [[ "$handle_count" -eq 1 && "$mutation_count" -eq 1 ]]
    else
        [[ "$handle_count" -eq 0 && "$mutation_count" -eq 0 ]]
    fi
}

smoke_capacity_ok() {
    local log=$1 line managed=0 capacity_count=0 high_count=0
    while IFS= read -r line; do
        line=${line%$'\r'}
        if [[ "$line" =~ ^\[mem\]\ \ \ managed\ RAM:\ [0-9]+\ MiB\ \([0-9]+\.[0-9]{2}\ GiB,\ ([0-9]+)\ total\ frames\)$ ]]; then
            managed=${BASH_REMATCH[1]}
        elif [[ "$line" == '[mem]   capacity: retained and released 1049088 distinct frames' ]]; then
            capacity_count=$((capacity_count + 1))
        elif [[ "$line" == '[mem]   frame above 4 GiB: zero/read/write/map/copy/free ok' ]]; then
            high_count=$((high_count + 1))
        fi
    done < "$log"
    [[ -z "$line" && "$managed" -gt 1048576 && "$capacity_count" -eq 1 && "$high_count" -eq 1 ]]
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
    memory_marker='[WOLFRAM_TEST] memory_verification: PASS'
    for index in "${!scenarios[@]}"; do
        scenario=${scenarios[index]}
        expected="[WOLFRAM_TEST] ${markers[index]}: PASS"
        valid="$memory_marker"$'\n'"$expected"$'\n'
        [[ "$scenario" != normal ]] || valid+="$handle_marker"$'\n'"$mutation_marker"$'\n'
        printf '%s' "$valid" > "$test_log"
        smoke_log_ok "$test_log" "$scenario"
        printf '%s' "${valid//$'\n'/$'\r\n'}" > "$test_log"
        smoke_log_ok "$test_log" "$scenario"
        for text in '' 'boot only' "${valid%$'\n'}" "$valid"$'PANIC\n' \
            "$valid"$'FAIL\n' "$valid"$'truncated' "$expected extra"$'\n' \
            "$valid$expected"$'\n' "$valid$handle_marker"$'\n' \
            "$valid$mutation_marker"$'\n' "$mutation_marker"$'\n' \
            "$valid$memory_marker"$'\n' \
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
            if smoke_log_ok "$test_log" normal; then
                echo 'smoke log self-check accepted missing memory evidence' >&2
                exit 1
            fi
            printf '%s\n%s\n%s\n%s\n' "$mutation_marker" "$memory_marker" "$handle_marker" "$expected" > "$test_log"
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
    capacity_valid=$'[mem]   managed RAM: 8186 MiB (7.99 GiB, 2095760 total frames)\n[mem]   frame above 4 GiB: zero/read/write/map/copy/free ok\n[mem]   capacity: retained and released 1049088 distinct frames\n'
    for valid in "$capacity_valid" "${capacity_valid//$'\n'/$'\r\n'}"; do
        printf '%s' "$valid" > "$test_log"
        smoke_capacity_ok "$test_log"
    done
    for invalid in '' "${capacity_valid/2095760/1048576}" "${capacity_valid/1049088/1048576}" \
        "${capacity_valid%$'\n'}" "$capacity_valid$capacity_valid"; do
        printf '%s' "$invalid" > "$test_log"
        if smoke_capacity_ok "$test_log"; then
            echo 'smoke capacity self-check accepted invalid output' >&2
            exit 1
        fi
    done
    for malformed in \
        '[mem]   managed RAM: 8186 MiB (7.99 GiB, 2095760 total frames) trailing' \
        'prefix [mem]   managed RAM: 8186 MiB (7.99 GiB, 2095760 total frames)'; do
        printf '%s\n%s\n%s\n' "$malformed" \
            '[mem]   frame above 4 GiB: zero/read/write/map/copy/free ok' \
            '[mem]   capacity: retained and released 1049088 distinct frames' > "$test_log"
        if smoke_capacity_ok "$test_log"; then
            echo 'smoke capacity self-check accepted a malformed RAM summary' >&2
            exit 1
        fi
    done
    echo 'Smoke log self-check passed'
fi
