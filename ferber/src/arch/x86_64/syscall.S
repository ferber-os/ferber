.section .text, "ax"
.global x86_syscall_entry
x86_syscall_entry:
    swapgs
    mov gs:[8], rsp
    mov rsp, gs:[0]
    push 0x1b
    push qword ptr gs:[8]
    push r11
    push 0x23
    push rcx
    push rax
    push rdi
    push rsi
    push rdx
    push r10
    push r8
    push r9
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    pushfq
    pop rsi
    cld
    mov rdi, rsp
    call x86_syscall_handler
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    pop r9
    pop r8
    pop r10
    pop rdx
    pop rsi
    pop rdi
    pop rax
    swapgs
    iretq

.global x86_enter_user
x86_enter_user:
    push 0x1b
    push rsi
    push 2
    push 0x23
    push rdi
    mov ax, 0x1b
    mov ds, ax
    mov es, ax
    xor eax, eax
    xor ebx, ebx
    xor ecx, ecx
    xor edx, edx
    xor edi, edi
    xor esi, esi
    xor ebp, ebp
    xor r8d, r8d
    xor r9d, r9d
    xor r10d, r10d
    xor r11d, r11d
    xor r12d, r12d
    xor r13d, r13d
    xor r14d, r14d
    xor r15d, r15d
    swapgs
    iretq

.macro CHECK_USER
    cmp rdi, 0x11
    jne 9f
    cmp rsi, 0x22
    jne 9f
    cmp rdx, 0x33
    jne 9f
    cmp r10, 0x44
    jne 9f
    cmp r8, 0x55
    jne 9f
    cmp r9, 0x66
    jne 9f
    cmp rbx, 0x12345678
    jne 9f
    cmp rbp, 0x23456789
    jne 9f
    cmp r12, 0x3456789a
    jne 9f
    cmp r13, 0x456789ab
    jne 9f
    cmp r14, 0x56789abc
    jne 9f
    cmp r15, 0x6789abcd
    jne 9f
    mov rax, {user_stack} + {stack_size} - 8
    cmp rsp, rax
    jne 9f
    cmp qword ptr [rsp], 0x12345678
    jne 9f
.endm

.macro PROBE_SYSCALL
    # Set DF and every arithmetic flag; capture returned flags before comparisons.
    push 0xcd7
    popfq
    syscall
    pushfq
    pop rcx
    cmp rcx, 0x8d7
    jne 9f
.endm

.macro HANDLE_CALL nr, result
    mov eax, \nr
    call .Lhandle_call
    cmp rax, \result
    jne 9f
.endm

.macro CHECK_INSPECT rights=0x307
    cmp dword ptr [rsi], 2
    jne 9f
    cmp dword ptr [rsi + 4], \rights
    jne 9f
.endm

.global x86_user_probe_start
.global x86_user_probe_end
x86_user_probe_start:
    # Every initial GPR must be zero, before using any as scratch.
    test rax, rax
    jnz 9f
    test rbx, rbx
    jnz 9f
    test rcx, rcx
    jnz 9f
    test rdx, rdx
    jnz 9f
    test rdi, rdi
    jnz 9f
    test rsi, rsi
    jnz 9f
    test rbp, rbp
    jnz 9f
    test r8, r8
    jnz 9f
    test r9, r9
    jnz 9f
    test r10, r10
    jnz 9f
    test r11, r11
    jnz 9f
    test r12, r12
    jnz 9f
    test r13, r13
    jnz 9f
    test r14, r14
    jnz 9f
    test r15, r15
    jnz 9f
    mov ax, cs
    and eax, 3
    cmp eax, 3
    jne 9f
    mov rax, {user_stack} + {stack_size}
    cmp rsp, rax
    jne 9f
    mov ebx, 0x12345678
    mov ebp, 0x23456789
    mov r12d, 0x3456789a
    mov r13d, 0x456789ab
    mov r14d, 0x56789abc
    mov r15d, 0x6789abcd
    push rbx
    mov edi, 0x11
    mov esi, 0x22
    mov edx, 0x33
    mov r10d, 0x44
    mov r8d, 0x55
    mov r9d, 0x66
    mov eax, {sys_yield}
    PROBE_SYSCALL
    test rax, rax
    jnz 9f
    CHECK_USER
    mov eax, {sys_yield}
    PROBE_SYSCALL
    test rax, rax
    jnz 9f
    CHECK_USER
    mov eax, 99
    PROBE_SYSCALL
    cmp rax, -38
    jne 9f
    CHECK_USER
    pop rax
    cmp rax, 0x12345678
    jne 9f
    # Fixtures are explicitly delegated after the process's empty-table check.
    # Keep calls below the final four stack bytes used by the preflight test.
    sub rsp, 128
    mov edi, 0xdead
    HANDLE_CALL {sys_yield}, 0
    HANDLE_CALL 99, -38
    HANDLE_CALL 100, -38

    xor edi, edi
    mov rsi, {user_stack} + 128
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT
    inc rsi
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT
    mov rsi, {user_stack} + 4092
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT

    # Invalid outputs must fail before any destination byte changes.
    xor esi, esi
    HANDLE_CALL {sys_cap_inspect}, -14
    mov rsi, {user_pc}
    HANDLE_CALL {sys_cap_inspect}, -14
    mov rsi, {user_stack} + {stack_size}
    HANDLE_CALL {sys_cap_inspect}, -14
    mov rsi, 0xffff800000000000
    HANDLE_CALL {sys_cap_inspect}, -14
    mov rsi, -4
    HANDLE_CALL {sys_cap_inspect}, -14
    mov rsi, {user_stack} + {stack_size} - 4
    mov dword ptr [rsi], 0x5a5a5a5a
    HANDLE_CALL {sys_cap_inspect}, -14
    cmp dword ptr [rsi], 0x5a5a5a5a
    jne 9f

    mov rsi, {user_stack} + 128
    mov qword ptr [rsi], 0x5a5a5a5a
    mov edi, 1
    HANDLE_CALL {sys_cap_inspect}, -13
    cmp qword ptr [rsi], 0x5a5a5a5a
    jne 9f
    HANDLE_CALL {sys_handle_duplicate}, -13
    HANDLE_CALL {sys_cap_revoke}, -13
    mov esi, 0x307
    HANDLE_CALL {sys_cap_attenuate}, -13
    mov rsi, {user_stack} + 128
    mov edi, 2
    HANDLE_CALL {sys_cap_inspect}, -116
    cmp qword ptr [rsi], 0x5a5a5a5a
    jne 9f
    HANDLE_CALL {sys_handle_duplicate}, -116
    HANDLE_CALL {sys_cap_revoke}, -116
    mov esi, 0x307
    HANDLE_CALL {sys_cap_attenuate}, -116
    mov rsi, {user_stack} + 128
    HANDLE_CALL {sys_handle_close}, 0
    HANDLE_CALL {sys_handle_close}, -9
    HANDLE_CALL {sys_cap_inspect}, -9
    HANDLE_CALL {sys_handle_duplicate}, -9

    # High bits must not be silently narrowed to a live u32 slot.
    mov rdi, 0x100000000
    HANDLE_CALL {sys_cap_inspect}, -9
    HANDLE_CALL {sys_handle_duplicate}, -9
    HANDLE_CALL {sys_handle_close}, -9
    mov edi, 256
    HANDLE_CALL {sys_cap_inspect}, -9
    HANDLE_CALL {sys_handle_duplicate}, -9
    HANDLE_CALL {sys_handle_close}, -9

    # Duplicate aliases share the node, but closing either handle preserves the other.
    xor edi, edi
    HANDLE_CALL {sys_handle_duplicate}, 2
    HANDLE_CALL {sys_handle_close}, 0
    mov edi, 2
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT
    HANDLE_CALL {sys_handle_duplicate}, 0
    HANDLE_CALL {sys_handle_close}, 0
    xor edi, edi
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT

    # Rights are a checked u32 mask; validation must never truncate or amplify.
    xor edi, edi
    mov esi, 0x400
    HANDLE_CALL {sys_cap_attenuate}, -22
    mov rsi, 0x100000000
    HANDLE_CALL {sys_cap_attenuate}, -22
    mov esi, 8
    HANDLE_CALL {sys_cap_attenuate}, -13
    mov edi, 2
    HANDLE_CALL {sys_cap_attenuate}, -9
    HANDLE_CALL {sys_cap_revoke}, -9
    mov rdi, 0x100000000
    HANDLE_CALL {sys_cap_attenuate}, -9
    HANDLE_CALL {sys_cap_revoke}, -9

    # A zero-rights child is valid and closeable, but grants no operations.
    xor edi, edi
    xor esi, esi
    HANDLE_CALL {sys_cap_attenuate}, 2
    mov edi, 2
    HANDLE_CALL {sys_cap_revoke}, -13
    HANDLE_CALL {sys_cap_attenuate}, -13
    HANDLE_CALL {sys_handle_duplicate}, -13
    mov rsi, {user_stack} + 128
    HANDLE_CALL {sys_cap_inspect}, -13
    HANDLE_CALL {sys_handle_close}, 0

    # Equal rights still create a distinct branch; attenuation preserves its parent.
    xor edi, edi
    mov esi, 0x307
    HANDLE_CALL {sys_cap_attenuate}, 2
    mov esi, 0x104
    HANDLE_CALL {sys_cap_attenuate}, 3
    mov edi, 2
    HANDLE_CALL {sys_cap_attenuate}, 4
    HANDLE_CALL {sys_handle_duplicate}, 5
    mov rsi, {user_stack} + 128
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT
    mov edi, 3
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT 0x104
    HANDLE_CALL {sys_cap_revoke}, -13
    mov edi, 4
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT 0x104
    HANDLE_CALL {sys_cap_revoke}, -13

    # Revocation through an alias reaches its branch and descendants, not siblings.
    mov edi, 5
    HANDLE_CALL {sys_cap_revoke}, 0
    .irp handle, 2, 4, 5
        mov edi, \handle
        mov rsi, {user_stack} + 128
        HANDLE_CALL {sys_cap_inspect}, -116
        HANDLE_CALL {sys_handle_duplicate}, -116
        mov esi, 0x307
        HANDLE_CALL {sys_cap_attenuate}, -116
        HANDLE_CALL {sys_cap_revoke}, -116
        HANDLE_CALL {sys_handle_close}, 0
    .endr
    xor edi, edi
    mov rsi, {user_stack} + 128
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT
    mov edi, 3
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT 0x104
    HANDLE_CALL {sys_handle_close}, 0
    xor edi, edi

    # Fill every free slot, recover one slot, then close all handles.
    mov r12d, 2
.Lfill_handles:
    HANDLE_CALL {sys_handle_duplicate}, r12
    inc r12
    cmp r12, 256
    jne .Lfill_handles
    HANDLE_CALL {sys_handle_duplicate}, -24
    mov esi, 0x307
    HANDLE_CALL {sys_cap_attenuate}, -24
    mov rsi, {user_stack} + 128
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT
    mov edi, 127
    HANDLE_CALL {sys_handle_close}, 0
    xor edi, edi
    HANDLE_CALL {sys_handle_duplicate}, 127
    HANDLE_CALL {sys_handle_duplicate}, -24
    xor edi, edi
.Lclose_handles:
    HANDLE_CALL {sys_handle_close}, 0
    inc edi
    cmp edi, 255
    jne .Lclose_handles
    xor edi, edi
    HANDLE_CALL {sys_handle_close}, -9
    HANDLE_CALL {sys_handle_duplicate}, -9
    HANDLE_CALL {sys_cap_inspect}, -9
    # One delegated VMO alias remains in slot 255 for transfer.
    xor edi, edi
    HANDLE_CALL {sys_chan_create}, -14
    mov rdi, {user_stack} + 1024
    HANDLE_CALL {sys_chan_create}, 0
    mov rbx, {user_stack} + 1024
    cmp dword ptr [rbx], 0
    jne 9f
    cmp dword ptr [rbx + 4], 1
    jne 9f

    .macro CHANNEL_RECV result, capacity=4, counts=({user_stack}+1088), endpoint=1
    mov edi, \endpoint
    mov rsi, {user_stack} + 1072
    mov edx, \capacity
    mov r10, {user_stack} + 1080
    mov r8d, 1
    mov r9, \counts
    HANDLE_CALL {sys_chan_recv}, \result
    .endm
    CHANNEL_RECV -11
    mov rbx, {user_stack} + 1056
    mov dword ptr [rbx], 0x74c53819
    mov dword ptr [rbx + 8], 255
    xor edi, edi
    xor esi, esi
    mov edx, 4
    mov r10, {user_stack} + 1064
    mov r8d, 1
    HANDLE_CALL {sys_chan_send}, -14
    xor edi, edi
    mov rsi, {user_stack} + 1056
    HANDLE_CALL {sys_chan_send}, 0
    mov edi, 255
    HANDLE_CALL {sys_handle_close}, -9
    CHANNEL_RECV -90, 3
    CHANNEL_RECV -14, 4, 0
    CHANNEL_RECV 0
    mov rbx, {user_stack} + 1088
    cmp dword ptr [rbx], 4
    jne 9f
    cmp dword ptr [rbx + 4], 1
    jne 9f
    mov rbx, {user_stack} + 1072
    cmp dword ptr [rbx], 0x74c53819
    jne 9f
    cmp dword ptr [rbx + 8], 2
    jne 9f
    mov edi, 2
    mov rsi, {user_stack} + 512
    HANDLE_CALL {sys_cap_inspect}, 0
    CHECK_INSPECT
    HANDLE_CALL {sys_handle_close}, 0
    mov rbx, {user_stack} + 1064
    mov dword ptr [rbx], 0
    xor edi, edi
    xor esi, esi
    xor edx, edx
    mov r10, {user_stack} + 1064
    mov r8d, 1
    HANDLE_CALL {sys_chan_send}, -22
    xor edi, edi
    mov esi, 4
    HANDLE_CALL {sys_cap_attenuate}, 2
    mov edi, 2
    xor esi, esi
    xor edx, edx
    xor r10d, r10d
    xor r8d, r8d
    HANDLE_CALL {sys_chan_send}, -13
    HANDLE_CALL {sys_handle_close}, 0
    mov edi, 1
    mov esi, 8
    HANDLE_CALL {sys_cap_attenuate}, 2
    mov edi, 2
    HANDLE_CALL {sys_chan_recv}, -13
    HANDLE_CALL {sys_handle_close}, 0
    xor edi, edi
    xor esi, esi
    xor edx, edx
    xor r10d, r10d
    xor r8d, r8d
    HANDLE_CALL {sys_chan_send}, 0
    CHANNEL_RECV 0, 0
    mov rbx, {user_stack} + 1088
    cmp qword ptr [rbx], 0
    jne 9f
    mov edi, 1
    mov rsi, {user_stack} + 1056
    mov edx, 4
    xor r10d, r10d
    xor r8d, r8d
    HANDLE_CALL {sys_chan_send}, 0
    mov edi, 1
    HANDLE_CALL {sys_handle_close}, 0
    CHANNEL_RECV 0, 4, ({user_stack}+1088), 0
    CHANNEL_RECV -32, 4, ({user_stack}+1088), 0
    xor edi, edi
    xor esi, esi
    xor edx, edx
    xor r10d, r10d
    xor r8d, r8d
    HANDLE_CALL {sys_chan_send}, -32
    HANDLE_CALL {sys_handle_close}, 0
    add rsp, 128
    xor edi, edi
    jmp 10f

.Lhandle_call:
    push rdi
    push rsi
    push rdx
    push r10
    push r8
    push r9
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    PROBE_SYSCALL
    .irp reg, r15, r14, r13, r12, rbp, rbx, r9, r8, r10, rdx, rsi, rdi
        pop rcx
        cmp \reg, rcx
        jne 9f
    .endr
    ret
9:
    mov edi, 1
10:
    mov eax, {sys_exit}
    xor esi, esi
    xor edx, edx
    xor r10d, r10d
    xor r8d, r8d
    xor r9d, r9d
    syscall
    ud2
x86_user_probe_end:

.section .data.rel.ro, "aw"
.global x86_isolation_start
.global x86_isolation_fault
.global x86_isolation_end
x86_isolation_start:
    mov rax, [rip + x86_isolation_target]
x86_isolation_fault:
    .if {write_probe}
    mov [rax], rax
    .else
    mov rax, [rax]
    .endif
    mov edi, 1
    mov eax, {sys_exit}
    syscall
    ud2
.balign 8
x86_isolation_target:
    .if {write_probe}
    .quad {user_pc}
    .else
    .quad x86_enter_user
    .endif
x86_isolation_end:
