#![no_std]
#![no_main]

extern crate arch;
extern crate common;
extern crate driver;
extern crate fs;
extern crate ipc;
extern crate mem;
extern crate shell;

pub unsafe fn kernel_init() {
    arch::interrupt_init();
}

#[no_mangle]
pub unsafe extern "C" fn kernel_start() -> ! {
    kernel_init();

    loop {
        core::hint::spin_loop();
    }
}
