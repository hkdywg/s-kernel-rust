//! ARM64 内核启动入口
//!
//! 本模块实现从 CPU 上电到跳转 `kernel_start` 的完整启动流程，
//! 所有函数均为 `#[unsafe(naked)]` 裸函数，使用 `naked_asm!` 编写，
//! 编译器不插入任何 prologue/epilogue，完全由代码控制。
//!
//! # 启动流程
//!
//! ```text
//! CPU 上电 / bootloader
//!     ↓ 跳转到 cpu_setup（当前 EL 未知）
//! cpu_setup
//!     ↓ 进入 EL1，eret 到 _start
//! _start
//!     ↓ bl cpu_start
//! cpu_start（当前文件未定义，由链接脚本或外部提供）
//!     ↓ 最终调用
//! kernel_start（kernel crate 入口）
//!     ↓ 内核启动完成
//! cpu_idle（空闲循环，wfe 等待事件）
//! ```
//!

use core::arch::naked_asm;

extern "C" {
    /// 内核主启动函数（由 kernel crate 提供）
    fn kernel_start() -> !;
    /// BSS 段起始地址（由链接脚本定义）
    static __bss_start: u8;
    /// BSS 段结束地址（由链接脚本定义）
    static __bss_end: u8;
}

// ============================================================================
// 入口点
// ============================================================================

/// 内核入口点
///
/// 执行最简单的跳转——`bl cpu_start` 将控制权交给启动代码。
/// 实际的 CPU 环境设置由 `cpu_setup` 完成后再回到此处。
///
/// # 属性
///
/// - `#[unsafe(naked)]`: 裸函数，编译器不生成栈帧
/// - `#[link_section = ".text.entrypoint"]`: 放在镜像入口段
#[unsafe(naked)]
#[no_mangle]
#[link_section = ".text.entrypoint"]
pub unsafe extern "C" fn _start() -> ! {
    naked_asm!("bl cpu_start");
}

// ============================================================================
// CPU 初始化
// ============================================================================

/// CPU 级别设置入口
///
/// 从当前未知的异常级别跳转到 EL1 并初始化环境。
///
/// # 执行步骤
///
/// 1. `ldr x1, =_start` —— 加载 EL1 入口地址
/// 2. `bl cpu_in_el1` —— 调用 EL1 初始化逻辑
/// 3. `eret` —— 异常返回，切到 EL1 并从 `_start` 开始执行
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn cpu_setup() -> ! {
    naked_asm!(
        "ldr x1, =_start",
        "bl cpu_in_el1",
        "eret"
    );
}

/// EL1 级别 CPU 初始化
///
/// 在内核态（EL1）完成系统初始化后跳转到 `kernel_start`。
///
/// # 初始化步骤
///
/// | 步骤 | 操作 | 含义 |
/// |------|------|------|
/// | 1 | `mov sp, x1` | 设置栈指针（x1 由调用者传入 `_start` 地址） |
/// | 2 | `msr cpacr_el1, #0x0030_0000` | 启用 FP/SIMD（浮点和向量指令） |
/// | 3 | `mrs/msr sctlr_el1` | 配置系统控制寄存器：启用 I-cache、禁用 MMU/对齐检查 |
/// | 4 | `ldr x1, =__bss_start` | BSS 段起始地址 |
/// | 5 | `ldr x2, =__bss_size` | BSS 段大小 |
/// | 6 | `bl clean_bss` | 将 BSS 段清零 |
/// | 7 | `blr x0` | 跳转到 `kernel_start` |
///
/// # SCTLR_EL1 配置
///
/// - `orr #(1<<12)`: 启用指令缓存（I-cache）
/// - `bic #(3<<3)`: 清除 SA/SA0（禁用栈对齐检查）
/// - `bic #(1<<1)`: 清除 A（禁用对齐错误检查，因为 MMU 未启用）
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn cpu_in_el1() -> ! {
    naked_asm!(
        // 栈指针（x1 由调用者传入）
        "mov sp, x1",
        // 启用 FP/SIMD（CPACR_EL1 的 FPEN 位）
        "mov x1, #0x00300000",
        "msr cpacr_el1, x1",
        // 配置系统控制寄存器
        "mrs x1, sctlr_el1",
        "orr x1, x1, #(1 << 12)",    // 启用 I-cache
        "bic x1, x1, #(3 << 3)",     // 禁用栈对齐检查
        "bic x1, x1, #(1 << 1)",     // 禁用对齐错误检查
        "msr sctlr_el1, x1",
        // 清零 BSS 段
        "ldr x1, =__bss_start",
        "ldr x2, =__bss_size",
        "bl clean_bss",
        // 跳转到内核主入口
        "ldr x0, =kernel_start",
        "blr x0",
        // kernel_start 返回后的兜底
        "b cpu_idle"
    );
}

// ============================================================================
// BSS 清零
// ============================================================================

/// BSS 段清零
///
/// 将 BSS 段（未初始化的全局/静态变量）全部写入 0。
/// BSS 清零是启动的关键步骤，C/Rust 语言的未初始化全局变量
/// 默认应为 0，但硬件上电后该区域的值是随机的。
///
/// # 参数约定（通过寄存器）
///
/// - `x1`: BSS 段起始地址
/// - `x2`: BSS 段大小（以 8 字节为单位）
///
/// # 汇编逻辑
///
/// ```text
/// if w2 == 0 → 跳过（BSS 为空）
/// loop:
///     *x1 = 0   (str xzr)
///     x1 += 8
///     w2 -= 1
///     if w2 != 0 → 继续
/// ret
/// ```
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn clean_bss() {
    naked_asm!(
        "cbz w2, 2f",        // w2 == 0 则跳到结束
        "1:",                // 循环标签
        "str xzr, [x1], #8", // 存 0，x1 += 8
        "sub w2, w2, #1",    // 计数减 1
        "cbnz w2, 1b",       // w2 != 0 则继续循环
        "2:",                // 结束标签
        "ret"
    );
}

// ============================================================================
// 空闲循环
// ============================================================================

/// CPU 空闲循环
///
/// 内核启动完成后的兜底状态，执行 `wfe`（Wait For Event）指令。
///
/// `wfe` 与 `wfi` 的区别：
///
/// | 指令 | 唤醒条件 |
/// |------|---------|
/// | `wfi` | 中断（IRQ/FIQ） |
/// | `wfe` | 事件（SEV 指令或中断） |
///
/// 使用 `wfe` 而非 `wfi` 是因为多核环境下可通过 SEV 指令唤醒。
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn cpu_idle() -> ! {
    naked_asm!(
        "wfe",
        "b cpu_idle"
    );
}
