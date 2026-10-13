    .section .text, "ax"
    .global switch_context
    .type switch_context, @function

# switch_context(prev: *mut CpuContext, next: *const CpuContext)
# System V AMD64 ABI:
#   rdi = prev
#   rsi = next
switch_context:
    # Save current context into `prev`
    mov [rdi + 0], rsp
    mov [rdi + 8], rbp
    mov [rdi + 16], rbx
    mov [rdi + 24], r12
    mov [rdi + 32], r13
    mov [rdi + 40], r14
    mov [rdi + 48], r15

    # Restore next context from `next`
    mov rsp, [rsi + 0]
    mov rbp, [rsi + 8]
    mov rbx, [rsi + 16]
    mov r12, [rsi + 24]
    mov r13, [rsi + 32]
    mov r14, [rsi + 40]
    mov r15, [rsi + 48]

    # Clear direction flag (System V ABI requirement)
    cld

    # Return pops the return address from the restored stack and jumps to it.
    ret

    .size switch_context, . - switch_context
