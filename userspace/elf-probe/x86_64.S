.section .text.entry,"ax"
.global _start
_start:
    mov rdi, rsp
    call probe_main
    mov edi, 1
    mov eax, 1
    syscall
    ud2

.section .text,"ax"
.global syscall6
syscall6:
    mov rax, rdi
    mov rdi, rsi
    mov rsi, rdx
    mov rdx, rcx
    mov r10, r8
    mov r8, r9
    mov r9, [rsp + 8]
    syscall
    ret

.global yield_preserved
yield_preserved:
    push r12
    sub rsp, 16
    mov r12, 0x12345678
    mov [rsp], r12
    mov [rsp + 8], rsp
    xor eax, eax
    syscall
    test rax, rax
    jnz 1f
    cmp r12, 0x12345678
    jne 1f
    cmp [rsp], r12
    jne 1f
    cmp [rsp + 8], rsp
    jne 1f
    jmp 2f
1:  mov eax, 1
2:  add rsp, 16
    pop r12
    ret
