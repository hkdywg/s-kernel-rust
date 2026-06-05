//! 内核镜像入口
//!
//! 本文件是最终链接出的 `kernel` 二进制可执行文件的入口点。
//! 整个启动流程：
//!
//! ```text
//! bootloader / QEMU
//!     ↓ 跳转到 .text.entrypoint
//! _start（汇编入口）
//!     ↓ 设置栈指针 SP
//! main（C ABI）
//!     ↓ 调用 kernel_init()
//! kernel_init
//!     ↓ 初始化中断、GIC 等子系统
//! 空闲循环（WFI 低功耗等待）
//! ```
//!
//! # 属性说明
//!
//! - `#![no_std]`: 裸机环境，禁用 Rust 标准库
//! - `#![no_main]`: 不生成标准 `main` 入口，由 `_start` 接管

#![no_std]
#![no_main]

extern crate arch;
extern crate common;
extern crate driver;
extern crate fs;
extern crate ipc;
extern crate mem;
extern crate shell;

/// 内核初始化入口
///
/// 由 `main` 调用，完成各子系统的初始化：
/// - 中断系统（向量表 + GIC）
///
/// 后续可扩展：内存管理初始化、驱动初始化、调度器启动等。
fn kernel_init() {
    unsafe {
        arch::interrupt_init();
    }
}

/// 汇编级入口点
///
/// bootloader 加载内核后执行的第一条指令。
///
/// # 功能
///
/// 1. 设置栈指针 SP = `0x4008_0000`（4MB 处，避开低地址外设区域）
/// 2. 调用 `main` 进入 Rust 世界
/// 3. `main` 返回后（理论上不会）执行 `wfi` + 死循环
///
/// # 属性
///
/// - `#[no_mangle]`: 保持符号名 `_start`，供链接器/启动脚本使用
/// - `#[link_section = ".text.entrypoint"]`: 放在入口代码段，确保在镜像最前面
/// - `extern "C"`: 使用 C ABI，与 bootloader 调用约定一致
/// - `#[noreturn]`: `asm!` 中 `b 2b` 永不返回，告诉编译器不做 epilogue
#[no_mangle]
#[link_section = ".text.entrypoint"]
pub unsafe extern "C" fn _start() -> ! {
    const SP_ADDR: u64 = 0x4008_0000;
    core::arch::asm!(
        "mov sp, {0}",    // 设置栈指针
        "bl main",        // 调用 Rust main
        "2:",             // 以下为安全兜底，正常不会执行
        "wfi",            // 低功耗等待
        "b 2b",           // 死循环
        in(reg) SP_ADDR,
        options(noreturn)
    );
}

/// C ABI 兼容的 main 函数
///
/// 由 `_start` 汇编入口调用（`bl main`）。
/// 注意：这不是 Rust 标准库的 `fn main()`，而是一个普通的
/// `extern "C"` 函数，只是约定命名为 main。
///
/// # 返回值
///
/// `!` 表示永不返回（发散函数），内核将进入空闲循环。
#[no_mangle]
pub extern "C" fn main() -> ! {
    kernel_init();
    loop {
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)) };
    }
}
