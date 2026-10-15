    .section .text
    .option push
    .option norvc
    .align 2
    .globl riscv_drop_to_user
riscv_drop_to_user:
    addi a2, a2, -16
    sd gp, 0(a2)
    sd tp, 8(a2)
    csrw sscratch, a2
    csrw sepc, a0
    csrw sie, zero
    csrr t0, sstatus
    li t1, 0x46702
    not t1, t1
    and t0, t0, t1
    ori t0, t0, 32
    csrw sstatus, t0
    fence.i
    mv sp, a1
    li ra, 0
    li gp, 0
    li tp, 0
    li t0, 0
    li t1, 0
    li t2, 0
    li s0, 0
    li s1, 0
    li a0, 0
    li a1, 0
    li a2, 0
    li a3, 0
    li a4, 0
    li a5, 0
    li a6, 0
    li a7, 0
    li s2, 0
    li s3, 0
    li s4, 0
    li s5, 0
    li s6, 0
    li s7, 0
    li s8, 0
    li s9, 0
    li s10, 0
    li s11, 0
    li t3, 0
    li t4, 0
    li t5, 0
    li t6, 0
    sret

    .macro seed nr
    li ra, 1
    li gp, 3
    li tp, 4
    li t0, 5
    li t1, 6
    li t2, 7
    li s0, 8
    li s1, 9
    li a0, 17
    li a1, 34
    li a2, 51
    li a3, 68
    li a4, 85
    li a5, 102
    li a6, 16
    li a7, \nr
    li s2, 18
    li s3, 19
    li s4, 20
    li s5, 21
    li s6, 22
    li s7, 23
    li s8, 24
    li s9, 25
    li s10, 26
    li s11, 27
    li t3, 28
    li t4, 29
    li t5, 30
    li t6, 31
    .endm

    .macro check result, nr
    addi sp, sp, -272
    sd ra, 8(sp)
    sd gp, 24(sp)
    sd tp, 32(sp)
    sd t0, 40(sp)
    sd t1, 48(sp)
    sd t2, 56(sp)
    sd s0, 64(sp)
    sd s1, 72(sp)
    sd a0, 80(sp)
    sd a1, 88(sp)
    sd a2, 96(sp)
    sd a3, 104(sp)
    sd a4, 112(sp)
    sd a5, 120(sp)
    sd a6, 128(sp)
    sd a7, 136(sp)
    sd s2, 144(sp)
    sd s3, 152(sp)
    sd s4, 160(sp)
    sd s5, 168(sp)
    sd s6, 176(sp)
    sd s7, 184(sp)
    sd s8, 192(sp)
    sd s9, 200(sp)
    sd s10, 208(sp)
    sd s11, 216(sp)
    sd t3, 224(sp)
    sd t4, 232(sp)
    sd t5, 240(sp)
    sd t6, 248(sp)
    addi t0, sp, 272
    sd t0, 16(sp)
    ld t0, 8(sp)
    li t1, 1
    bne t0, t1, .Luser_fail
    ld t0, 16(sp)
    li t1, 137438969856
    bne t0, t1, .Luser_fail
    ld t0, 24(sp)
    li t1, 3
    bne t0, t1, .Luser_fail
    ld t0, 32(sp)
    li t1, 4
    bne t0, t1, .Luser_fail
    ld t0, 40(sp)
    li t1, 5
    bne t0, t1, .Luser_fail
    ld t0, 48(sp)
    li t1, 6
    bne t0, t1, .Luser_fail
    ld t0, 56(sp)
    li t1, 7
    bne t0, t1, .Luser_fail
    ld t0, 64(sp)
    li t1, 8
    bne t0, t1, .Luser_fail
    ld t0, 72(sp)
    li t1, 9
    bne t0, t1, .Luser_fail
    ld t0, 80(sp)
    li t1, \result
    bne t0, t1, .Luser_fail
    ld t0, 88(sp)
    li t1, 34
    bne t0, t1, .Luser_fail
    ld t0, 96(sp)
    li t1, 51
    bne t0, t1, .Luser_fail
    ld t0, 104(sp)
    li t1, 68
    bne t0, t1, .Luser_fail
    ld t0, 112(sp)
    li t1, 85
    bne t0, t1, .Luser_fail
    ld t0, 120(sp)
    li t1, 102
    bne t0, t1, .Luser_fail
    ld t0, 128(sp)
    li t1, 16
    bne t0, t1, .Luser_fail
    ld t0, 136(sp)
    li t1, \nr
    bne t0, t1, .Luser_fail
    ld t0, 144(sp)
    li t1, 18
    bne t0, t1, .Luser_fail
    ld t0, 152(sp)
    li t1, 19
    bne t0, t1, .Luser_fail
    ld t0, 160(sp)
    li t1, 20
    bne t0, t1, .Luser_fail
    ld t0, 168(sp)
    li t1, 21
    bne t0, t1, .Luser_fail
    ld t0, 176(sp)
    li t1, 22
    bne t0, t1, .Luser_fail
    ld t0, 184(sp)
    li t1, 23
    bne t0, t1, .Luser_fail
    ld t0, 192(sp)
    li t1, 24
    bne t0, t1, .Luser_fail
    ld t0, 200(sp)
    li t1, 25
    bne t0, t1, .Luser_fail
    ld t0, 208(sp)
    li t1, 26
    bne t0, t1, .Luser_fail
    ld t0, 216(sp)
    li t1, 27
    bne t0, t1, .Luser_fail
    ld t0, 224(sp)
    li t1, 28
    bne t0, t1, .Luser_fail
    ld t0, 232(sp)
    li t1, 29
    bne t0, t1, .Luser_fail
    ld t0, 240(sp)
    li t1, 30
    bne t0, t1, .Luser_fail
    ld t0, 248(sp)
    li t1, 31
    bne t0, t1, .Luser_fail
    ld t0, 264(sp)
    li t1, 0x5a5a
    bne t0, t1, .Luser_fail
    addi sp, sp, 272
    .endm

    .align 2
    .globl riscv_user_probe_start
riscv_user_probe_start:
    bnez ra, .Luser_fail
    bnez gp, .Luser_fail
    bnez tp, .Luser_fail
    bnez t0, .Luser_fail
    bnez t1, .Luser_fail
    bnez t2, .Luser_fail
    bnez s0, .Luser_fail
    bnez s1, .Luser_fail
    bnez a0, .Luser_fail
    bnez a1, .Luser_fail
    bnez a2, .Luser_fail
    bnez a3, .Luser_fail
    bnez a4, .Luser_fail
    bnez a5, .Luser_fail
    bnez a6, .Luser_fail
    bnez a7, .Luser_fail
    bnez s2, .Luser_fail
    bnez s3, .Luser_fail
    bnez s4, .Luser_fail
    bnez s5, .Luser_fail
    bnez s6, .Luser_fail
    bnez s7, .Luser_fail
    bnez s8, .Luser_fail
    bnez s9, .Luser_fail
    bnez s10, .Luser_fail
    bnez s11, .Luser_fail
    bnez t3, .Luser_fail
    bnez t4, .Luser_fail
    bnez t5, .Luser_fail
    bnez t6, .Luser_fail
    li t0, 0x2000004000
    bne sp, t0, .Luser_fail
    li t0, 0x5a5a
    sd t0, -8(sp)
    seed 0
    ecall
    check 0, 0
    seed 0
    ecall
    check 0, 0
    seed 99
    ecall
    check -38, 99
    li a0, 0
    j .Luser_exit
.Luser_fail:
    li a0, 1
.Luser_exit:
    li a1, 0
    li a2, 0
    li a3, 0
    li a4, 0
    li a5, 0
    li a7, 1
    ecall
    unimp
    .globl riscv_user_probe_end
riscv_user_probe_end:

    .align 2
    .globl riscv_isolation_probe_start
riscv_isolation_probe_start:
    .if {write_probe}
    li t0, {user_pc}
    .else
    li t0, 0x80200000
    .endif
    .globl riscv_isolation_fault
riscv_isolation_fault:
    .if {write_probe}
    sd t1, 0(t0)
    .else
    ld t1, 0(t0)
    .endif
    li a0, 1
    li a7, 1
    ecall
    unimp
    .globl riscv_isolation_probe_end
riscv_isolation_probe_end:
    .option pop
