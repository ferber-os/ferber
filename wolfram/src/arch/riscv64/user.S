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
    li t1, ({user_stack} + {stack_size})
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
    li t0, ({user_stack} + {stack_size})
    bne sp, t0, .Luser_fail
    li t0, 0x5a5a
    sd t0, -8(sp)
    seed {sys_yield}
    ecall
    check 0, {sys_yield}
    seed {sys_yield}
    ecall
    check 0, {sys_yield}
    seed 99
    ecall
    check -38, 99
    j .Lhandle_probe

.Luser_fail:
    li a0, 1
.Luser_exit:
    li a1, 0
    li a2, 0
    li a3, 0
    li a4, 0
    li a5, 0
    li a7, {sys_exit}
    ecall
    unimp
    // Snapshot both sides of ecall so every non-result GPR is checked.
.Lchecked_ecall:
    addi sp, sp, -528
    sd x1, 8(sp)
    sd x2, 16(sp)
    sd x3, 24(sp)
    sd x4, 32(sp)
    sd x5, 40(sp)
    sd x6, 48(sp)
    sd x7, 56(sp)
    sd x8, 64(sp)
    sd x9, 72(sp)
    sd x10, 80(sp)
    sd x11, 88(sp)
    sd x12, 96(sp)
    sd x13, 104(sp)
    sd x14, 112(sp)
    sd x15, 120(sp)
    sd x16, 128(sp)
    sd x17, 136(sp)
    sd x18, 144(sp)
    sd x19, 152(sp)
    sd x20, 160(sp)
    sd x21, 168(sp)
    sd x22, 176(sp)
    sd x23, 184(sp)
    sd x24, 192(sp)
    sd x25, 200(sp)
    sd x26, 208(sp)
    sd x27, 216(sp)
    sd x28, 224(sp)
    sd x29, 232(sp)
    sd x30, 240(sp)
    sd x31, 248(sp)
    ld t0, 520(sp)
    sd t0, 0(sp)
    ld t0, 40(sp)
    ecall
    sd x1, 264(sp)
    sd x2, 272(sp)
    sd x3, 280(sp)
    sd x4, 288(sp)
    sd x5, 296(sp)
    sd x6, 304(sp)
    sd x7, 312(sp)
    sd x8, 320(sp)
    sd x9, 328(sp)
    sd x10, 336(sp)
    sd x11, 344(sp)
    sd x12, 352(sp)
    sd x13, 360(sp)
    sd x14, 368(sp)
    sd x15, 376(sp)
    sd x16, 384(sp)
    sd x17, 392(sp)
    sd x18, 400(sp)
    sd x19, 408(sp)
    sd x20, 416(sp)
    sd x21, 424(sp)
    sd x22, 432(sp)
    sd x23, 440(sp)
    sd x24, 448(sp)
    sd x25, 456(sp)
    sd x26, 464(sp)
    sd x27, 472(sp)
    sd x28, 480(sp)
    sd x29, 488(sp)
    sd x30, 496(sp)
    sd x31, 504(sp)
    li t0, 8
.Lcompare_regs:
    li t1, 80
    beq t0, t1, .Lnext_reg
    add t1, sp, t0
    ld t2, 0(t1)
    ld t3, 256(t1)
    bne t2, t3, .Luser_fail
.Lnext_reg:
    addi t0, t0, 8
    li t1, 256
    blt t0, t1, .Lcompare_regs
    ld t0, 520(sp)
    ld t1, 0(sp)
    bne t0, t1, .Luser_fail
    ld ra, 8(sp)
    ld t0, 40(sp)
    ld t1, 48(sp)
    ld t2, 56(sp)
    ld t3, 224(sp)
    addi sp, sp, 528
    ret

    .macro checked nr, result
    li a7, \nr
    jal ra, .Lchecked_ecall
    li t0, \result
    bne a0, t0, .Luser_fail
    .endm

    .macro cap nr, handle, result
    li a0, \handle
    checked \nr, \result
    .endm

    .macro inspect_at address, result
    li a0, 0
    li a1, \address
    checked {sys_inspect}, \result
    .endm

.Lhandle_probe:
    li a0, 123
    li a1, 456
    checked {sys_yield}, 0
    checked 100, -38
    checked 99, -38
    .macro check_info address
    li t0, \address
    li t1, 0x0000010500000002
    li t2, 8
.Linfo_loop\@:
    lbu t3, 0(t0)
    andi t4, t1, 255
    bne t3, t4, .Luser_fail
    srli t1, t1, 8
    addi t0, t0, 1
    addi t2, t2, -1
    bnez t2, .Linfo_loop\@
    .endm
    inspect_at ({user_stack} + 512), 0
    check_info ({user_stack} + 512)
    inspect_at ({user_stack} + 513), 0
    check_info ({user_stack} + 513)
    inspect_at ({user_stack} + 4092), 0
    check_info ({user_stack} + 4092)
    inspect_at 0, -14
    inspect_at {user_pc}, -14
    inspect_at ({user_stack} - 4096), -14
    inspect_at 0x80200000, -14
    inspect_at -4, -14
    // The valid prefix must stay unchanged when the next page is missing.
    li s0, ({user_stack} + {stack_size} - 4)
    li t0, 0x5a5a5a5a
    sw t0, 0(s0)
    mv a1, s0
    cap {sys_inspect}, 0, -14
    lwu t0, 0(s0)
    li t1, 0x5a5a5a5a
    bne t0, t1, .Luser_fail
    // Restore the stack sentinel after the overlapping range-failure test.
    li t0, 0x5a5a
    sd t0, -8(sp)
    cap {sys_duplicate}, 0, 3
    cap {sys_close}, 0, 0
    li a1, ({user_stack} + 512)
    cap {sys_inspect}, 3, 0
    cap {sys_close}, 0, -9
    cap {sys_duplicate}, 1, -13
    cap {sys_inspect}, 1, -13
    cap {sys_duplicate}, 2, -116
    cap {sys_inspect}, 2, -116
    cap {sys_close}, 2, 0
    cap {sys_close}, 1, 0
    cap {sys_duplicate}, 256, -9
    cap {sys_inspect}, 256, -9
    cap {sys_close}, 256, -9
    cap {sys_duplicate}, 0x100000003, -9
    cap {sys_inspect}, 0x100000003, -9
    cap {sys_close}, 0x100000003, -9
    li s0, 0
.Lfill_handles:
    li a0, 3
    li a7, {sys_duplicate}
    jal ra, .Lchecked_ecall
    bne a0, s0, .Luser_fail
    addi s0, s0, 1
    li t0, 3
    bne s0, t0, .Lfill_next
    addi s0, s0, 1
.Lfill_next:
    li t0, 256
    blt s0, t0, .Lfill_handles
    cap {sys_duplicate}, 3, -24
    cap {sys_close}, 255, 0
    cap {sys_duplicate}, 3, 255
    li s0, 0
.Lclose_handles:
    mv a0, s0
    checked {sys_close}, 0
    addi s0, s0, 1
    li t0, 256
    blt s0, t0, .Lclose_handles
    cap {sys_close}, 255, -9
    li a0, 0
    j .Luser_exit
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
    li a7, {sys_exit}
    ecall
    unimp
    .globl riscv_isolation_probe_end
riscv_isolation_probe_end:
    .option pop
