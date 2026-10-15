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
    mov rax, 0x2000003ff8
    cmp rsp, rax
    jne 9f
    cmp qword ptr [rsp], 0x12345678
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
    mov rax, 0x2000004000
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
    xor eax, eax
    syscall
    test rax, rax
    jnz 9f
    CHECK_USER
    xor eax, eax
    syscall
    test rax, rax
    jnz 9f
    CHECK_USER
    mov eax, 99
    syscall
    cmp rax, -38
    jne 9f
    CHECK_USER
    pop rax
    cmp rax, 0x12345678
    jne 9f
    xor edi, edi
    jmp 10f
9:
    mov edi, 1
10:
    mov eax, 1
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
    mov rax, [rax]
    mov edi, 1
    mov eax, 1
    syscall
    ud2
.balign 8
x86_isolation_target:
    .quad x86_enter_user
x86_isolation_end:
