.option norelax
.section .text.entry,"ax"
.global _start
_start:
    mv a0, sp
    call user_main
    li a0, 1
    li a7, 1
    ecall
    unimp

.section .text,"ax"
.global syscall6
syscall6:
    mv a7, a0
    mv a0, a1
    mv a1, a2
    mv a2, a3
    mv a3, a4
    mv a4, a5
    mv a5, a6
    ecall
    ret

