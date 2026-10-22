.section .text,"ax"
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
