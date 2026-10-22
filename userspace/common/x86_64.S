.section .text.entry,"ax"
.global _start
_start:
    mov rdi, rsp
    call user_main
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

