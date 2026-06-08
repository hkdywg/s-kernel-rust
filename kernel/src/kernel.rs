#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn main() -> ! {
    unsafe {
        write_uart("Rust Kernel Started!\n");

        kernel::kernel_start();
    }
}

unsafe fn write_uart(msg: &str) {
    let uart_base: *mut u8 = 0x09000000 as *mut u8;
    for byte in msg.bytes() {
        core::ptr::write_volatile(uart_base, byte);
    }
}
