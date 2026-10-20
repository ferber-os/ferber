.section .text.entry,"ax"
.global _start
_start:
    test rsp, 15
    jnz fail
    sub rsp, 16
    mov r12, 0x12345678
    mov [rsp], r12
    cmp [rip + probe_rodata], r12
    jne fail
    cmp [rip + probe_data], r12
    jne fail
    lea rbx, [rip + probe_bss]
    mov ecx, 1024
1:
    cmp qword ptr [rbx], 0
    jne fail
    add rbx, 8
    loop 1b
    lea rbx, [rip + probe_bss]
    mov [rbx], r12
    mov [rbx + 8184], r12
    mov eax, 0x7fff
    syscall
    cmp rax, -38
    jne fail
    mov eax, 9
    xor edi, edi
    lea rsi, [rip + probe_message]
    mov edx, 16
    lea r10, [rip + probe_handles]
    xor r8d, r8d
    lea r9, [rip + probe_counts]
    syscall
    test rax, rax
    jnz fail
    cmp dword ptr [rip + probe_counts], 8
    jne fail
    cmp dword ptr [rip + probe_counts + 4], 0
    jne fail
    mov rbx, 0x313a726562726566
    cmp qword ptr [rip + probe_message], rbx
    jne fail
    mov eax, 8
    xor edi, edi
    lea rsi, [rip + probe_reply]
    mov edx, 8
    lea r10, [rip + probe_handles]
    xor r8d, r8d
    syscall
    test rax, rax
    jnz fail
    mov eax, 2
    xor edi, edi
    syscall
    test rax, rax
    jnz fail
    xor eax, eax
    syscall
    test rax, rax
    jnz fail
    cmp r12, 0x12345678
    jne fail
    cmp [rsp], r12
    jne fail
    lea rbx, [rip + probe_bss]
    cmp [rbx], r12
    jne fail
    cmp [rbx + 8184], r12
    jne fail
    mov qword ptr [rip + probe_data], 0
    xor edi, edi
    jmp exit
fail:
    mov edi, 1
exit:
    mov eax, 1
    syscall
    ud2

.section .rodata,"a"
.balign 8
probe_rodata: .quad 0x12345678
.section .data,"aw"
.balign 8
probe_data: .quad 0x12345678
.section .bss,"aw",@nobits
.balign 8
probe_bss: .zero 8192
.balign 8
probe_message: .zero 16
probe_handles: .zero 8
probe_counts: .zero 8
.section .rodata,"a"
.balign 8
probe_reply: .ascii "ready-v1"
