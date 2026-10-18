    .section .text
    .option push
    .option norvc
    .align 2
    .globl riscv_cooperative_probe_start
riscv_cooperative_probe_start:
    .irp reg,1,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    li x\reg, \reg
    .endr
    li s2, {user_stack}
    ld s0, 0(s2)
    li s3, 0x74000000
    or s3, s3, s0
    li t0, 0x5a5a
    sd t0, -8(sp)
    li s1, 1

    .macro coop_call nr, result
    li a7, \nr
    jal ra, .Lcoop_checked
    li t0, \result
    bne a0, t0, .Lcoop_fail
    ld t0, 16(s2)
    bne t0, s3, .Lcoop_fail
    .endm

    .macro coop_recv
    li a0, 0
    addi a1, s2, 128
    li a2, 4
    li a3, 0
    li a4, 0
    addi a5, s2, 136
    coop_call {sys_chan_recv_wait}, 0
    lwu t0, 136(s2)
    li t1, 4
    bne t0, t1, .Lcoop_fail
    lwu t0, 140(s2)
    bnez t0, .Lcoop_fail
    lwu t0, 128(s2)
    bne t0, s4, .Lcoop_fail
    .endm

    .macro coop_send value
    sw \value, 128(s2)
    li a0, 0
    addi a1, s2, 128
    li a2, 4
    li a3, 0
    li a4, 0
    li a5, 0
    coop_call {sys_chan_send}, 0
    .endm

.Lcoop_round:
    bnez s0, .Lcoop_sender
    mv s4, s1
    coop_recv
    addi s4, s1, 256
    coop_send s4
    coop_call {sys_yield}, 0
    j .Lcoop_next
.Lcoop_sender:
    coop_call {sys_yield}, 0
    coop_send s1
    addi s4, s1, 256
    coop_recv
.Lcoop_next:
    addi s1, s1, 1
    li t0, 5
    bne s1, t0, .Lcoop_round
    bnez s0, .Lcoop_success
    li a0, 0
    addi a1, s2, 128
    li a2, 4
    li a3, 0
    li a4, 0
    addi a5, s2, 136
    coop_call {sys_chan_recv_wait}, -32
    li a0, 0
    coop_call {sys_close}, 0
.Lcoop_success:
    li a0, 0
    j .Lcoop_exit
.Lcoop_fail:
    li a0, 1
.Lcoop_exit:
    li a7, {sys_exit}
    ecall
    unimp

    # Snapshot every non-result GPR across each returning syscall, including sp.
.Lcoop_checked:
    addi sp, sp, -528
    .irp reg,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    sd x\reg, (\reg * 8)(sp)
    .endr
    ld t0, 520(sp)
    sd t0, 0(sp)
    ld t0, 40(sp)
    ecall
    .irp reg,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    sd x\reg, (256 + \reg * 8)(sp)
    .endr
    li t0, 8
.Lcoop_compare:
    li t1, 80
    beq t0, t1, .Lcoop_skip_result
    add t1, sp, t0
    ld t2, 0(t1)
    ld t3, 256(t1)
    bne t2, t3, .Lcoop_fail
.Lcoop_skip_result:
    addi t0, t0, 8
    li t1, 256
    blt t0, t1, .Lcoop_compare
    ld t0, 520(sp)
    ld t1, 0(sp)
    bne t0, t1, .Lcoop_fail
    li t1, 0x5a5a
    bne t0, t1, .Lcoop_fail
    ld ra, 8(sp)
    ld t0, 40(sp)
    ld t1, 48(sp)
    ld t2, 56(sp)
    ld t3, 224(sp)
    addi sp, sp, 528
    ret
    .globl riscv_cooperative_probe_end
riscv_cooperative_probe_end:
    .option pop
