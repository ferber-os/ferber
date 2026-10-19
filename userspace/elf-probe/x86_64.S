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
