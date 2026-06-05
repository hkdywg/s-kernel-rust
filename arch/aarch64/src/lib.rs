//! ARM64 (AArch64) 架构支持库
//!
//! 本 crate 提供 ARM64 架构特定的功能实现，包括：
//!
//! - **异常处理**：中断向量表、IRQ/FIQ 处理
//! - **上下文管理**：线程上下文保存和恢复
//! - **中断控制**：中断启用/禁用
//! - **Panic 处理**：内核 panic 时的死循环处理
//!
//! # 架构背景
//!
//! ARM64 (AArch64) 是 ARM 架构的 64 位版本，具有以下特点：
//!
//! - 31 个通用寄存器（x0-x30）
//! - 4 个异常级别（EL0-EL3）
//! - 独立的异常向量表（VBAR_EL1）
//! - 特定的系统寄存器（DAIF、VBAR 等）
//!
//! # 使用场景
//!
//! 本库作为内核的架构抽象层，被以下模块使用：
//!
//! - `kernel` crate：核心内核逻辑
//! - `sched` 模块：调度器（使用上下文切换）
//! - `interrupt` 模块：中断处理
//!
//! # 安全性
//!
//! 本库大量使用 `unsafe` 操作，因为：
//!
//! - 直接操作硬件寄存器
//! - 使用内联汇编
//! - 操作裸指针（上下文切换）
//!
//! 所有 unsafe 函数都有详细的安全要求说明，调用者必须遵守。

#![no_std]

use core::panic::PanicInfo;

/// Panic 处理函数
///
/// 当内核发生 panic（如数组越界、断言失败等）时，此函数被调用。
/// 由于是裸机环境，没有标准输出，采用死循环策略。
///
/// # Panic 时的行为
///
/// 1. 进入无限循环
/// 2. 使用 `spin_loop()` 提示 CPU 进行低功耗等待
/// 3. 系统停止响应（需要外部调试器或重启）
///
/// # 为什么需要 #[panic_handler]
///
/// 在 `#![no_std]` 环境中：
/// - 没有 Rust 标准库的 panic 处理
/// - 必须自定义 panic 处理逻辑
/// - 这是 lang item，编译器会自动找到
///
/// # 后续完善部分
///
/// 未来可以实现：
/// - 通过 UART 输出 panic 信息
/// - 保存 panic 位置到特定内存地址
/// - 触发系统复位
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

mod interrupt;
mod context;

pub use interrupt::*;
pub use context::*;

/// 禁用中断并返回当前中断状态
///
/// 用于需要原子操作的临界区，防止中断干扰。
///
/// # 返回值
///
/// 返回 `DAIF` 寄存器的当前值，包含中断屏蔽状态：
///
/// | 位 | 标志 | 含义 |
/// |----|------|------|
/// | 6 | D | Debug 异常屏蔽 |
/// | 7 | A | SError (异步错误) 屏蔽 |
/// | 8 | I | IRQ (普通中断) 屏蔽 |
/// | 9 | F | FIQ (快速中断) 屏蔽 |
///
/// 返回值可用于后续恢复中断状态（调用 `enable_interrupt`）。
///
/// # 执行流程
///
/// ```asm
/// mrs {0}, daif    # 读取 DAIF 寄存器（保存当前状态）
/// msr daifset, #3  # 设置位 3（屏蔽 IRQ 和 FIQ）
/// dsb sy           # 数据同步屏障（确保写入完成）
/// ```
///
/// # 指令解释
///
/// - **MRS (Move Register from System)**：读取系统寄存器到通用寄存器
///   - `mrs x0, daif`：将 DAIF 寄存器值复制到 x0
///
/// - **MSR (Move to System Register)**：将通用寄存器值写入系统寄存器
///   - `msr daifset, #3`：设置 DAIF 的第 3 位（IRQ + FIQ）
///
/// - **DSB (Data Synchronization Barrier)**：数据同步屏障
///   - 确保之前的内存操作完成
///   - 防止中断在屏障完成前发生
///
/// # 使用示例
///
/// ```rust
/// // 临界区操作
/// let saved_state = disable_interrupts();
/// // ... 执行原子操作 ...
/// enable_interrupt(saved_state);  // 恢复中断状态
/// ```
///
/// # #[inline(always)]
///
/// 强制内联的原因：
/// - 函数非常短小（几条汇编指令）
/// - 避免函数调用开销（栈操作）
/// - 临界区需要最快响应速度
///
/// # 安全性
///
/// **unsafe 原因：**
/// - 直接操作系统寄存器（DAIF）
/// - 使用内联汇编
///
/// **调用者责任：**
/// - 必须在适当的时机调用（不要长期禁用中断）
/// - 必须保存返回值并恢复中断状态
/// - 禁用中断期间不应执行耗时操作
#[inline(always)]
pub unsafe fn disable_interrupts() -> u64 {
    let daif: u64;
    core::arch::asm!(
        "mrs {0}, daif",       // 读取 DAIF 寄存器（当前中断状态）
        "msr daifset, #3",     // 设置位 3：屏蔽 IRQ (I) 和 FIQ (F)
        "dsb sy",              // 数据同步屏障（确保设置生效）
        out(reg) daif,         // 输出：DAIF 寄存器值
        options(nostack)       // 不操作栈，无需栈检查
    );
    daif
}

/// 恢复中断状态
///
/// 根据 `disable_interrupts()` 返回的状态值恢复中断。
/// 只在之前中断未被屏蔽时才重新启用。
///
/// # 参数
///
/// - `level`: 之前保存的 DAIF 寄存器值
///   - 由 `disable_interrupts()` 返回
///   - 包含中断屏蔽状态（D、A、I、F 标志）
///
/// # 执行逻辑
///
/// ```asm
/// dsb sy           # 数据同步屏障
/// mov x1, #0xC0    # 0xC0 = 位 6 (D) + 位 7 (A) 的掩码
/// ands {0}, {0}, x1  # 检查 level 的 D 和 A 位
/// b.ne 1f          # 如果 D 或 A 被屏蔽，跳转到标签 1（不恢复中断）
/// msr daifclr, #3  # 清除位 3：恢复 IRQ 和 FIQ
/// 1:               # 结束标签
/// ```
///
/// # 条件恢复逻辑
///
/// **只有当之前的 D 和 A 位都为 0 时才恢复中断：**
///
/// | 条件 | 行为 |
/// |------|------|
/// | D=0, A=0 | 执行 `msr daifclr, #3`，恢复 IRQ 和 FIQ |
/// | D=1 或 A=1 | 不执行恢复，跳过中断启用 |
///
/// **原因：**
/// - 如果之前 Debug 或 SError 被屏蔽，表示特殊情况
/// - 不应强制恢复 IRQ/FIQ，保持屏蔽状态
///
/// # 指令解释
///
/// - **ANDS (And with Shift)**：按位与并更新条件标志
///   - 结果为 0 → Z 标志置 1（Equal）
///   - 结果非 0 → Z 标志置 0（Not Equal）
///
/// - **B.NE (Branch if Not Equal)**：条件跳转
///   - Z=0（结果非 0）时跳转
///   - Z=1（结果为 0）时不跳转
///
/// - **MSR DAIFCLR**：清除 DAIF 寄存器的指定位
///   - `msr daifclr, #3`：清除位 3（IRQ + FIQ），即启用中断
///
/// # 使用示例
///
/// ```rust
/// // 正确的临界区模式
/// let state = disable_interrupts();
/// // ... 临界区操作 ...
/// enable_interrupt(state);
///
/// // 错误示范
/// enable_interrupt(0);  // ❌ 强制启用所有中断（危险）
/// ```
///
/// # #[inline(always)]
///
/// 强制内联，原因同 `disable_interrupts`：
/// - 性能关键路径
/// - 函数体极短
///
/// # 安全性
///
/// **unsafe 原因：**
/// - 操作系统寄存器（DAIF）
/// - 使用内联汇编
///
/// **调用者责任：**
/// - 参数必须是 `disable_interrupts()` 的返回值
/// - 不应传入任意值（可能破坏中断状态）
#[inline(always)]
pub unsafe fn enable_interrupt(level: u64) {
    core::arch::asm!(
        "dsb sy",              // 数据同步屏障（确保临界区操作完成）
        "mov x1, #0xC0",       // 0xC0 = 0b11000000（位 6 和 7）
        "ands {0}, {0}, x1",   // 检查 level 的 D 和 A 位是否被屏蔽
        "b.ne 1f",             // 如果被屏蔽（非零），跳转到标签 1
        "msr daifclr, #3",     // 清除位 3：恢复 IRQ 和 FIQ（启用中断）
        "1:",                  // 标签 1：结束（不恢复中断）
        in(reg) level,         // 输入：之前保存的 DAIF 值
        options(nostack)       // 不操作栈
    );
}

/// 中断初始化函数
///
/// 设置中断向量表基址，启用中断控制器。
///
/// # 初始化内容
///
/// 应包括：
/// 1. 调用 `set_vbar()` 设置向量表基址
/// 2. 初始化 GIC（Generic Interrupt Controller）
/// 3. 配置中断优先级
/// 4. 启用特定中断源（定时器、UART 等）
///
/// # 使用示例
///
/// ```rust
/// // 在内核启动时调用
/// interrupt_init();
/// ```
///
/// # 注意事项
///
/// 当前为空实现，需要后续完善：
/// - 实现 GIC 驱动
/// - 配置中断分发
/// - 设置优先级掩码
pub fn interrupt_init() {
    // TODO: 实现中断控制器初始化
    // TODO: 设置中断向量表 (set_vbar)
    // TODO: 配置 GIC (Generic Interrupt Controller)
}
