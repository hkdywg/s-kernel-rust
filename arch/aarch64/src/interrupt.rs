//! ARM64 (AArch64) 异常处理系统实现
//!
//! 本模块负责：
//! - 定义中断向量表（16 个异常向量）
//! - 保存/恢复 CPU 寄存器上下文
//! - 通过 GIC 分发中断到注册的处理函数
//! - 提供中断处理函数注册接口
//!
//! # 中断处理流程
//!
//! ```text
//! 硬件中断 → vector_irq（汇编入口，保存寄存器）
//!          → interrupt_handler（分发）
//!          → ISR_TABLE[irq].handler（已注册的处理函数）
//!          → gic_ack_irq（应答 GIC）
//!          → eret（恢复寄存器，返回）
//! ```

#![allow(dead_code)]

use core::arch::asm;
use core::ptr::null_mut;

use crate::gic::{gic_get_irq, gic_ack_irq, GIC_MAX_HANDLERS};

// ============================================================================
// 中断服务例程表
// ============================================================================

/// 中断描述符
///
/// 将中断号映射到具体的处理函数。每个中断 ID 对应一个 `IrqDesc`，
/// 存储在 `ISR_TABLE` 中。
///
/// # 字段说明
///
/// - `handler`: 处理函数指针，`None` 表示该中断未注册处理程序
/// - `param`: 传递给处理函数的自定义参数（如设备结构体指针）
///
/// # 为什么使用 `#[repr(C)]`
///
/// 保证布局与 C 兼容，如果将来需要从 C 代码或汇编中访问。
#[repr(C)]
pub struct IrqDesc {
    handler: Option<unsafe extern "C" fn(irq_num: u32, param: *mut u8)>,
    param: *mut u8,
}

/// 中断服务例程表
///
/// 最多支持 [`GIC_MAX_HANDLERS`]（96）个中断处理函数。
/// 初始时所有条目为空（`handler: None`），通过 [`interrupt_install`] 注册。
///
/// # 使用示例
///
/// ```ignore
/// extern "C" fn timer_handler(irq_num: u32, _param: *mut u8) {
///     // 处理定时器中断
/// }
///
/// // 注册定时器中断（ID=27）
/// unsafe { interrupt_install(27, timer_handler, null_mut()); }
/// ```
static mut ISR_TABLE: [IrqDesc; GIC_MAX_HANDLERS] = {
    const INIT: IrqDesc = IrqDesc {
        handler: None,
        param: null_mut(),
    };
    [INIT; GIC_MAX_HANDLERS]
};

// ============================================================================
// 异常向量表
// ============================================================================

/// ARM64 异常向量表
///
/// 16 个条目，地址必须 2KB 对齐。每 4 个一组对应不同异常级别和来源。
///
/// # 布局
///
/// | 偏移  | 异常类型   | 说明                     |
/// |-------|-----------|-------------------------|
/// | 0x000 | EL0 同步   | 用户态系统调用、缺页异常    |
/// | 0x080 | EL0 IRQ    | 用户态普通中断            |
/// | 0x100 | EL0 FIQ    | 用户态快速中断            |
/// | 0x180 | EL0 SError | 用户态系统错误            |
/// | 0x200 | EL1 同步   | 内核态异常（缺页、断点）    |
/// | 0x280 | EL1 IRQ    | 内核态普通中断（主要使用）  |
/// | 0x300 | EL1 FIQ    | 内核态快速中断            |
/// | 0x380 | EL1 SError | 内核态系统错误            |
///
/// EL0_64 / EL1_64 组同理。EL2（虚拟化）、EL3（安全世界）暂不实现。
#[no_mangle]
#[link_section = ".vectors"]
pub static INTERRUPT_VECTOR: [unsafe extern "C" fn(); 16] = [
    vector_error, vector_irq, vector_fiq, vector_error,  // EL0
    vector_error, vector_irq, vector_fiq, vector_error,  // EL1
    vector_error, vector_irq, vector_fiq, vector_error,  // EL0_64
    vector_error, vector_irq, vector_fiq, vector_error,  // EL1_64
];

// ============================================================================
// 异常向量函数
// ============================================================================

/// 错误异常向量 —— 死循环占位
///
/// 处理 SError 和未实现的同步异常，`b .` 原地死循环防止进一步崩溃。
#[no_mangle]
pub unsafe extern "C" fn vector_error() {
    asm!("b .", options(noreturn));
}

/// IRQ（普通中断）处理向量
///
/// 硬件中断发生时 CPU 跳转至此。处理流程：保存寄存器 → 分发 → 恢复 → eret。
///
/// # 寄存器保存策略
///
/// 仅保存 callee-saved（x19-x30）和参数寄存器（x0-x1），
/// caller-saved（x2-x18）由调用者负责。
///
/// | 寄存器组  | 原因                           |
/// |----------|--------------------------------|
/// | x0-x1    | 参数寄存器，可能被 handler 修改   |
/// | x19-x28  | callee-saved，Rust 函数必须保存 |
/// | x29(FP)  | 栈帧指针，调试/回溯必需           |
/// | x30(LR)  | 返回地址，eret 恢复 PC 时依赖    |
#[no_mangle]
pub unsafe extern "C" fn vector_irq() {
    asm!(
        // 保存 callee-saved 寄存器 + 参数寄存器
        "stp x29, x30, [sp, #-16]!",
        "stp x27, x28, [sp, #-16]!",
        "stp x25, x26, [sp, #-16]!",
        "stp x23, x24, [sp, #-16]!",
        "stp x21, x22, [sp, #-16]!",
        "stp x19, x20, [sp, #-16]!",
        "stp x0,  x1,  [sp, #-16]!",
        // 分发
        "bl interrupt_handler",
        // 恢复（顺序与保存相反）
        "ldp x0, x1,  [sp], #16",
        "ldp x19, x20, [sp], #16",
        "ldp x21, x22, [sp], #16",
        "ldp x23, x24, [sp], #16",
        "ldp x25, x26, [sp], #16",
        "ldp x27, x28, [sp], #16",
        "ldp x29, x30, [sp], #16",
        // 异常返回
        "eret",
        options(nostack)
    );
}

/// FIQ（快速中断）处理向量
///
/// FIQ 优先级高于 IRQ，用于安全事件和高实时性硬件。处理流程与 IRQ 相同，
/// 直接复用 `interrupt_handler`。
#[no_mangle]
pub unsafe extern "C" fn vector_fiq() {
    asm!(
        "stp x29, x30, [sp, #-16]!",
        "stp x27, x28, [sp, #-16]!",
        "stp x25, x26, [sp, #-16]!",
        "stp x23, x24, [sp, #-16]!",
        "stp x21, x22, [sp, #-16]!",
        "stp x19, x20, [sp, #-16]!",
        "stp x0,  x1,  [sp, #-16]!",
        "bl fiq_handler",
        "ldp x0, x1,  [sp], #16",
        "ldp x19, x20, [sp], #16",
        "ldp x21, x22, [sp], #16",
        "ldp x23, x24, [sp], #16",
        "ldp x25, x26, [sp], #16",
        "ldp x27, x28, [sp], #16",
        "ldp x29, x30, [sp], #16",
        "eret",
        options(nostack)
    );
}

// ============================================================================
// 中断控制
// ============================================================================

/// 设置异常向量表基址
///
/// 将向量表地址写入系统寄存器 `VBAR_EL1`，CPU 发生异常时跳转至此表。
/// 地址必须 2KB 对齐（由链接脚本 `.vectors` 段保证）。
pub unsafe fn set_vbar() {
    asm!(
        "msr vbar_el1, {0}",
        in(reg) INTERRUPT_VECTOR.as_ptr() as u64,
        options(nostack)
    );
}

// ============================================================================
// 中断分发处理
// ============================================================================

/// IRQ 中断分发函数
///
/// 由 `vector_irq` 的汇编入口调用，执行以下逻辑：
///
/// 1. 从 GIC 读取当前中断 ID（`gic_get_irq`）
/// 2. 如果 ID 为 1023（GIC 特殊值，表示无中断），直接返回
/// 3. 查表 `ISR_TABLE`，找到已注册的处理函数
/// 4. 调用处理函数，传入中断号和自定义参数
/// 5. 应答 GIC（`gic_ack_irq`），清除中断状态
#[no_mangle]
pub extern "C" fn interrupt_handler() {
    unsafe {
        let irq = gic_get_irq();
        if irq == 1023 { return };           // 1023 = GIC 无中断标识

        let irq_usize = irq as usize;
        if irq_usize < GIC_MAX_HANDLERS {
            if let Some(handler) = ISR_TABLE[irq_usize].handler {
                let param = ISR_TABLE[irq_usize].param;
                handler(irq as u32, param);
            }
        }

        gic_ack_irq(irq);
    }
}

/// FIQ 快速中断分发函数
///
/// 当前与 IRQ 共享同一套分发逻辑（直接转调 `interrupt_handler`）。
/// 后续可扩展为独立的快速路径。
#[no_mangle]
pub extern "C" fn fiq_handler() {
    interrupt_handler();
}

// ============================================================================
// 中断注册 API
// ============================================================================

/// 注册中断处理函数
///
/// 将自定义处理函数注册到 `ISR_TABLE` 中，当中断发生时由 `interrupt_handler` 调用。
///
/// # 参数
///
/// - `irq_num`: 中断 ID（0-95）
/// - `handler`: 处理函数，签名 `unsafe extern "C" fn(irq_num: u32, param: *mut u8)`
/// - `param`: 传递给 handler 的自定义参数（如设备指针）
///
/// # 使用示例
///
/// ```ignore
/// extern "C" fn uart_irq_handler(_irq: u32, uart_ptr: *mut u8) {
///     // 处理 UART 中断
/// }
///
/// unsafe { interrupt_install(33, uart_irq_handler, uart_dev as *mut u8); }
/// ```
pub unsafe fn interrupt_install(
    irq_num: u32,
    handler: unsafe extern "C" fn(irq_num: u32, param: *mut u8),
    param: *mut u8,
) {
    if (irq_num as usize) < GIC_MAX_HANDLERS {
        ISR_TABLE[irq_num as usize].handler = Some(handler);
        ISR_TABLE[irq_num as usize].param = param;
    }
}
