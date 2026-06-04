#![no_std]
#![no_main]

extern crate arch;
extern crate common;
extern crate driver;
extern crate fs;
extern crate ipc;
extern crate mem;
extern crate shell;

fn kernel_init() {
    arch::interrupt_init();
}

#[no_mangle]
#[link_section = ".text.entrypoint"]
pub unsafe extern "C" fn _start() -> ! {
    const SP_ADDR: u64 = 0x4008_0000;
    core::arch::asm!(
        "mov sp, {0}",
        "bl main",
        "2:",
        "wfi",
        "b 2b",
        in(reg) SP_ADDR,
        options(noreturn)
    );
}

#[no_mangle]
pub fn main() {
    kernel_init();
    loop {
        core::hint::spin_loop();
    }
}
