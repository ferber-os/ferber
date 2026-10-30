#![allow(dead_code)]
unsafe extern "C" {
    fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize;
}
pub fn call(n: usize, a: [usize; 6]) -> isize {
    unsafe { syscall6(n, a[0], a[1], a[2], a[3], a[4], a[5]) }
}
pub fn exit(status: usize) -> ! {
    call(1, [status, 0, 0, 0, 0, 0]);
    loop {
        core::hint::spin_loop()
    }
}
pub fn checked(n: usize, a: [usize; 6]) -> usize {
    let r = call(n, a);
    if r < 0 {
        exit(10 + n)
    }
    r as usize
}
pub fn close(h: usize) {
    checked(2, [h, 0, 0, 0, 0, 0]);
}
pub fn send(h: usize, b: &[u8]) {
    checked(8, [h, b.as_ptr() as usize, b.len(), 0, 0, 0]);
}
pub fn receive(h: usize, b: &mut [u8]) -> usize {
    let mut c = [0u32; 2];
    checked(
        10,
        [
            h,
            b.as_mut_ptr() as usize,
            b.len(),
            0,
            0,
            c.as_mut_ptr() as usize,
        ],
    );
    if c[1] != 0 {
        exit(40)
    }
    c[0] as usize
}
pub fn recv(h: usize, b: &mut [u8]) {
    if receive(h, b) != b.len() {
        exit(40)
    }
}
pub fn pair() -> [u32; 2] {
    let mut p = [0; 2];
    checked(7, [p.as_mut_ptr() as usize, 0, 0, 0, 0, 0]);
    p
}
pub fn wait(handles: &[u32]) -> usize {
    checked(23, [handles.as_ptr() as usize, handles.len(), 0, 0, 0, 0])
}
