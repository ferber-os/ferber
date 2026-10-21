.option norelax
.section .text.entry,"ax"
.global _start
_start:
    mv a0, sp
    call probe_main
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

.global yield_preserved
yield_preserved:
    addi sp, sp, -32
    sd s0, 0(sp)
    sd ra, 8(sp)
    li s0, 0x12345678
    sd s0, 16(sp)
    sd sp, 24(sp)
    li a7, 0
    ecall
    bnez a0, 1f
    li t0, 0x12345678
    bne s0, t0, 1f
    ld t1, 16(sp)
    bne t1, t0, 1f
    ld t1, 24(sp)
    bne t1, sp, 1f
    j 2f
1:  li a0, 1
2:  ld s0, 0(sp)
    ld ra, 8(sp)
    addi sp, sp, 32
    ret
