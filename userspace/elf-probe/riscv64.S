.section .text.entry,"ax"
.global _start
_start:
    andi t0, sp, 15
    bnez t0, fail
    addi sp, sp, -16
    li s0, 0x12345678
    sd s0, 0(sp)
    lla t0, probe_rodata
    ld t1, 0(t0)
    bne t1, s0, fail
    lla t0, probe_data
    ld t1, 0(t0)
    bne t1, s0, fail
    lla t0, probe_bss
    li t1, 1024
1:
    ld t2, 0(t0)
    bnez t2, fail
    addi t0, t0, 8
    addi t1, t1, -1
    bnez t1, 1b
    sd s0, -8(t0)
    lla t0, probe_bss
    sd s0, 0(t0)
    li a7, 0x7fff
    ecall
    li t0, -38
    bne a0, t0, fail
    li a7, 0
    ecall
    bnez a0, fail
    li t0, 0x12345678
    bne s0, t0, fail
    ld t1, 0(sp)
    bne t1, t0, fail
    lla t1, probe_bss
    ld t2, 0(t1)
    bne t2, t0, fail
    li t2, 8184
    add t1, t1, t2
    ld t2, 0(t1)
    bne t2, t0, fail
    lla t1, probe_data
    sd zero, 0(t1)
    li a0, 0
    j exit
fail:
    li a0, 1
exit:
    li a7, 1
    ecall
    unimp

.section .rodata,"a"
.balign 8
probe_rodata: .quad 0x12345678
.section .data,"aw"
.balign 8
probe_data: .quad 0x12345678
.section .bss,"aw",@nobits
.balign 8
probe_bss: .zero 8192
