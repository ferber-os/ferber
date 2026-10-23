#!/usr/bin/env bash
# Shared checks for completed user-mode probes; source from the smoke scripts.
smoke_payload_failure_ok() {
    local log=$1 scenario=$2 expected line='' memory=0 banner=0 detail=0
    case "$scenario" in
        init-missing) expected='  detail: boot payload required' ;;
        init-malformed-ready) expected='  detail: init bootstrap failed' ;;
        init-exit-before-ready) expected='  detail: init exited unexpectedly: 17' ;;
        init-exit-after-ready) expected='  detail: init exited unexpectedly: 18' ;;
        init-close-bootstrap) expected='  detail: init exited unexpectedly: 19' ;;
        init-wrong-channel) expected='  detail: cooperative deadlock: live tasks with no runnable producer' ;;
        payload-missing) expected='  detail: boot payload required' ;;
        payload-invalid) expected='  detail: load userspace ELF: Image(Header)' ;;
        payload-exit) expected='  detail: assertion `left == right` failed: incomplete ELF protocol' ;;
        *) return 1 ;;
    esac
    while IFS= read -r line; do
        line=${line%$'\r'}
        case "$line" in
            '[FERBER_TEST] memory_verification: PASS') memory=$((memory + 1)) ;;
            *'[FERBER_TEST]'*) return 1 ;;
            *'F E R B E R   K E R N E L   P A N I C'*) banner=$((banner + 1)) ;;
            "$expected") detail=$((detail + 1)) ;;
            *'detail:'*|*[Ff][Aa][Ii][Ll]*) return 1 ;;
        esac
    done < "$log"
    [[ -z "$line" && "$memory" -eq 1 && "$banner" -eq 1 && "$detail" -eq 1 ]]
}

smoke_deadlock_ok() {
    local log=$1 line='' memory=0 banner=0 detail=0 roundtrip=0 handles=0 mutation=0 channel=0
    grep -Eiq 'FAIL' "$log" && return 1
    while IFS= read -r line; do
        line=${line%$'\r'}
        case "$line" in
            '[FERBER_TEST] memory_verification: PASS') memory=$((memory + 1)) ;;
            '[FERBER_TEST] privilege_switch_roundtrip: PASS') roundtrip=$((roundtrip + 1)) ;;
            '[FERBER_TEST] handle_syscalls: PASS') handles=$((handles + 1)) ;;
            '[FERBER_TEST] capability_mutation: PASS') mutation=$((mutation + 1)) ;;
            '[FERBER_TEST] channel_syscalls: PASS') channel=$((channel + 1)) ;;
            *'[FERBER_TEST]'*) return 1 ;;
            *'F E R B E R   K E R N E L   P A N I C'*) banner=$((banner + 1)) ;;
            '  detail: cooperative deadlock: live tasks with no runnable producer') detail=$((detail + 1)) ;;
            *'detail:'*) return 1 ;;
        esac
    done < "$log"
    [[ -z "$line" && "$memory" -eq 1 && "$banner" -eq 1 && "$detail" -eq 1 \
        && "$roundtrip" -le 1 && "$handles" -le 1 && "$mutation" -le 1 && "$channel" -le 1 ]]
}

smoke_log_ok() {
    local log=$1 scenario=$2 expected line='' count=0
    local handle_count=0 mutation_count=0 memory_count=0 channel_count=0 bootstrap_count=0 payload_count=0
    local scheduling_count=0 blocking_count=0 reclamation_count=0 init_ready_count=0
    if [[ "$scenario" == deadlock ]]; then
        smoke_deadlock_ok "$log"
        return $?
    fi
    if [[ "$scenario" == payload-* || "$scenario" == init-* ]]; then
        smoke_payload_failure_ok "$log" "$scenario"
        return $?
    fi
    case "$scenario" in
        normal) expected='[FERBER_TEST] privilege_switch_roundtrip: PASS' ;;
        isolation) expected='[FERBER_TEST] privilege_isolation: PASS' ;;
        write-rx) expected='[FERBER_TEST] privilege_write_rx: PASS' ;;
        execute-nx) expected='[FERBER_TEST] privilege_execute_nx: PASS' ;;
        init) expected='[FERBER_TEST] init_waiting: PASS' ;;
        vmo) expected='[FERBER_TEST] vmo_syscalls: PASS' ;;
        elf) expected='[FERBER_TEST] userspace_elf: PASS' ;;
        *) return 1 ;;
    esac
    if grep -Eiq 'FAIL|panic' "$log"; then
        return 1
    fi
    while IFS= read -r line; do
        line=${line%$'\r'}
        if [[ "$line" == "$expected" ]]; then
            count=$((count + 1))
        elif [[ "$scenario" == normal && "$line" == '[FERBER_TEST] handle_syscalls: PASS' ]]; then
            handle_count=$((handle_count + 1))
        elif [[ "$scenario" == normal && "$line" == '[FERBER_TEST] capability_mutation: PASS' ]]; then
            mutation_count=$((mutation_count + 1))
        elif [[ "$scenario" == normal && "$line" == '[FERBER_TEST] channel_syscalls: PASS' ]]; then
            channel_count=$((channel_count + 1))
        elif [[ "$scenario" == normal && "$line" == '[FERBER_TEST] cooperative_scheduling: PASS' ]]; then
            scheduling_count=$((scheduling_count + 1))
        elif [[ "$scenario" == normal && "$line" == '[FERBER_TEST] blocking_channel_receive: PASS' ]]; then
            blocking_count=$((blocking_count + 1))
        elif [[ "$scenario" == normal && "$line" == '[FERBER_TEST] task_reclamation: PASS' ]]; then
            reclamation_count=$((reclamation_count + 1))
        elif [[ "$scenario" == elf && "$line" == '[FERBER_TEST] userspace_bootstrap_channel: PASS' ]]; then
            bootstrap_count=$((bootstrap_count + 1))
        elif [[ "$scenario" == init && "$line" == '[FERBER_TEST] init_ready: PASS' ]]; then
            init_ready_count=$((init_ready_count + 1))
        elif [[ ( "$scenario" == elf || "$scenario" == init || "$scenario" == vmo ) && "$line" == '[FERBER_TEST] boot_payload: PASS' ]]; then
            payload_count=$((payload_count + 1))
        elif [[ "$line" == '[FERBER_TEST] memory_verification: PASS' ]]; then
            memory_count=$((memory_count + 1))
        elif [[ "$line" == *'[FERBER_TEST]'* ]]; then
            return 1
        fi
    done < "$log"
    # A partial final write must not turn a crashed probe into a passing run.
    [[ -z "$line" && "$count" -eq 1 && "$memory_count" -eq 1 ]] || return 1
    if [[ "$scenario" == init ]]; then
        [[ "$init_ready_count" -eq 1 && "$payload_count" -eq 1 ]] || return 1
    fi
    if [[ "$scenario" == vmo ]]; then
        [[ "$payload_count" -eq 1 ]] || return 1
    fi
    if [[ "$scenario" == elf ]]; then
        [[ "$bootstrap_count" -eq 1 && "$payload_count" -eq 1 ]] || return 1
    fi
    if [[ "$scenario" == normal ]]; then
        [[ "$handle_count" -eq 1 && "$mutation_count" -eq 1 && "$channel_count" -eq 1 \
            && "$scheduling_count" -eq 1 && "$blocking_count" -eq 1 && "$reclamation_count" -eq 1 ]]
    else
        [[ "$handle_count" -eq 0 && "$mutation_count" -eq 0 && "$channel_count" -eq 0 \
            && "$scheduling_count" -eq 0 && "$blocking_count" -eq 0 && "$reclamation_count" -eq 0 ]]
    fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    set -euo pipefail
    [[ "${1:-}" == --self-test ]] || exit 2
    test_log=$(mktemp)
    trap 'rm -f -- "$test_log"' EXIT
    markers=(privilege_switch_roundtrip privilege_isolation privilege_write_rx privilege_execute_nx userspace_elf init_waiting vmo_syscalls)
    scenarios=(normal isolation write-rx execute-nx elf init vmo)
    handle_marker='[FERBER_TEST] handle_syscalls: PASS'
    mutation_marker='[FERBER_TEST] capability_mutation: PASS'
    memory_marker='[FERBER_TEST] memory_verification: PASS'
    channel_marker='[FERBER_TEST] channel_syscalls: PASS'
    scheduling_marker='[FERBER_TEST] cooperative_scheduling: PASS'
    blocking_marker='[FERBER_TEST] blocking_channel_receive: PASS'
    reclamation_marker='[FERBER_TEST] task_reclamation: PASS'
    bootstrap_marker='[FERBER_TEST] userspace_bootstrap_channel: PASS'
    payload_marker='[FERBER_TEST] boot_payload: PASS'
    init_ready_marker='[FERBER_TEST] init_ready: PASS'
    for index in "${!scenarios[@]}"; do
        scenario=${scenarios[index]}
        expected="[FERBER_TEST] ${markers[index]}: PASS"
        valid="$memory_marker"$'\n'"$expected"$'\n'
        [[ "$scenario" != normal ]] || valid+="$handle_marker"$'\n'"$mutation_marker"$'\n'"$channel_marker"$'\n'"$scheduling_marker"$'\n'"$blocking_marker"$'\n'"$reclamation_marker"$'\n'
        [[ "$scenario" != elf ]] || valid+="$bootstrap_marker"$'\n'"$payload_marker"$'\n'
        [[ "$scenario" != vmo ]] || valid+="$payload_marker"$'\n'
        [[ "$scenario" != init ]] || valid+="$init_ready_marker"$'\n'"$payload_marker"$'\n'
        printf '%s' "$valid" > "$test_log"
        smoke_log_ok "$test_log" "$scenario"
        printf '%s' "${valid//$'\n'/$'\r\n'}" > "$test_log"
        smoke_log_ok "$test_log" "$scenario"
        for text in '' 'boot only' "${valid%$'\n'}" "$valid"$'PANIC\n' \
            "$valid"$'FAIL\n' "$valid"$'truncated' "$expected extra"$'\n' \
            "$valid$expected"$'\n' "$valid$handle_marker"$'\n' \
            "$valid$mutation_marker"$'\n' "$mutation_marker"$'\n' \
            "$valid$memory_marker"$'\n' "$valid$channel_marker"$'\n' \
            "$valid$scheduling_marker"$'\n' "$valid$blocking_marker"$'\n' \
            "$valid$reclamation_marker"$'\n' \
            "$valid$bootstrap_marker"$'\n' "$valid$init_ready_marker"$'\n' \
            "$valid"$'FAIL: a different check failed\n' "$valid"$'fAiL: unrelated\n' \
            "$valid$payload_marker"$'\n' \
            "$handle_marker"$'\n' "$valid"$'[FERBER_TEST] unknown: PASS\n'; do
            printf '%s' "$text" > "$test_log"
            if smoke_log_ok "$test_log" "$scenario"; then
                echo 'smoke log self-check accepted invalid output' >&2
                exit 1
            fi
        done
        required=("$memory_marker" "$expected")
        [[ "$scenario" != normal ]] || required+=("$handle_marker" "$mutation_marker" "$channel_marker" "$scheduling_marker" "$blocking_marker" "$reclamation_marker")
        [[ "$scenario" != elf ]] || required+=("$bootstrap_marker" "$payload_marker")
        [[ "$scenario" != vmo ]] || required+=("$payload_marker")
        [[ "$scenario" != init ]] || required+=("$init_ready_marker" "$payload_marker")
        for missing in "${required[@]}"; do
            printf '%s' "${valid/"$missing"$'\n'/}" > "$test_log"
            if smoke_log_ok "$test_log" "$scenario"; then
                echo "smoke log self-check accepted missing evidence: $missing" >&2
                exit 1
            fi
        done
        for other in "${!markers[@]}"; do
            [[ "$index" != "$other" ]] || continue
            printf '[FERBER_TEST] %s: PASS\n' "${markers[other]}" > "$test_log"
            if smoke_log_ok "$test_log" "$scenario"; then
                echo 'smoke log self-check accepted the wrong scenario' >&2
                exit 1
            fi
        done
    done
    deadlock_banner='  F E R B E R   K E R N E L   P A N I C  '
    deadlock_detail='  detail: cooperative deadlock: live tasks with no runnable producer'
    valid="$memory_marker"$'\n'"$deadlock_banner"$'\n'"$deadlock_detail"$'\n'
    legacy='[FERBER_TEST] privilege_switch_roundtrip: PASS'$'\n'"$handle_marker"$'\n'"$mutation_marker"$'\n'"$channel_marker"$'\n'
    for text in "$valid" "${valid//$'\n'/$'\r\n'}" "$legacy$valid"; do
        printf '%s' "$text" > "$test_log"
        smoke_log_ok "$test_log" deadlock
    done
    for text in '' 'boot only' "${valid%$'\n'}" "$valid"'truncated' \
        "$memory_marker"$'\n'"$deadlock_banner"$'\n' \
        "$memory_marker"$'\n'"$deadlock_detail"$'\n' \
        "$deadlock_banner"$'\n'"$deadlock_detail"$'\n' \
        "$valid$deadlock_detail"$'\n' "$valid$deadlock_banner"$'\n' \
        "$valid$memory_marker"$'\n' "$legacy$legacy$valid" \
        "$valid$scheduling_marker"$'\n' "$valid$blocking_marker"$'\n' \
        "$valid$reclamation_marker"$'\n' "$valid"$'FAIL\n' \
        "$valid"$'  detail: unexpected panic\n' \
        "$valid"$'[FERBER_TEST] unknown: PASS\n'; do
        printf '%s' "$text" > "$test_log"
        if smoke_log_ok "$test_log" deadlock; then
            echo 'smoke log self-check accepted invalid deadlock evidence' >&2
            exit 1
        fi
    done
    for scenario in payload-missing payload-invalid payload-exit init-missing init-malformed-ready init-exit-before-ready init-exit-after-ready init-close-bootstrap init-wrong-channel; do
        case "$scenario" in
            init-missing) detail='  detail: boot payload required' ;;
            init-malformed-ready) detail='  detail: init bootstrap failed' ;;
            init-exit-before-ready) detail='  detail: init exited unexpectedly: 17' ;;
            init-exit-after-ready) detail='  detail: init exited unexpectedly: 18' ;;
            init-close-bootstrap) detail='  detail: init exited unexpectedly: 19' ;;
            init-wrong-channel) detail='  detail: cooperative deadlock: live tasks with no runnable producer' ;;
            payload-missing) detail='  detail: boot payload required' ;;
            payload-invalid) detail='  detail: load userspace ELF: Image(Header)' ;;
            payload-exit) detail='  detail: assertion `left == right` failed: incomplete ELF protocol' ;;
        esac
        valid="$memory_marker"$'\n'"$deadlock_banner"$'\n'"$detail"$'\n'
        printf '%s' "$valid" > "$test_log"
        smoke_log_ok "$test_log" "$scenario"
        for text in '' "${valid%$'\n'}" "$valid$detail"$'\n' \
            "$valid$deadlock_banner"$'\n' "$valid$memory_marker"$'\n' \
            "$valid"$'FAIL: a different check failed\n' "$valid"$'fAiL: unrelated\n' \
            "$valid$payload_marker"$'\n' "$valid$bootstrap_marker"$'\n' \
            "$memory_marker"$'\n'"$deadlock_banner"$'\n''  detail: unrelated panic'$'\n'; do
            printf '%s' "$text" > "$test_log"
            if smoke_log_ok "$test_log" "$scenario"; then
                echo 'smoke log self-check accepted invalid payload failure' >&2
                exit 1
            fi
        done
    done
    echo 'Smoke log self-check passed'
fi
