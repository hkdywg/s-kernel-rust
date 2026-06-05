//! GIC (Generic Interrupt Controller) 中断控制器驱动
//!
//! 本模块实现 ARM GICv2 中断控制器的初始化、中断使能/禁止、中断应答等操作。
//!
//! # GIC 架构概述
//!
//! GIC 是 ARM 架构的标准中断控制器，负责接收硬件中断信号并分发到 CPU 核心。
//! GICv2 包含两个主要组件：
//!
//! - **Distributor（分发器）**：管理系统中的所有中断源，控制中断优先级、分组和分发
//! - **CPU Interface（CPU 接口）**：每个 CPU 核心的私有接口，处理中断应答和优先级
//!
//! # 内存映射
//!
//! ```text
//! Distributor:  0x0800_0000 - 0x0800_0FFF
//! CPU Interface: 0x0801_0000 - 0x0801_0FFF
//! ```
//!
//! # 中断 ID 类型
//!
//! | ID 范围  | 类型 | 说明                     |
//! |---------|------|--------------------------|
//! | 0-15    | SGI  | 软件生成中断（核间通信）    |
//! | 16-31   | PPI  | 私有外设中断（每核独立的定时器等）|
//! | 32-1019 | SPI  | 共享外设中断（UART、磁盘等）  |

#![allow(dead_code)]
use core::ptr;

// ============================================================================
// Distributor 基址和 CPU Interface 基址
// ============================================================================

/// GIC Distributor（分发器）基址
pub const GIC_DIST_BASE: usize = 0x0800_0000;
/// GIC CPU Interface 基址
pub const GIC_CPU_BASE: usize = 0x0801_0000;

// ============================================================================
// Distributor 寄存器偏移量
// ============================================================================

/// Distributor 控制寄存器 - 启用/禁用整个分发器
const GIC_DIST_CTRL: usize = 0x000;
/// Type 寄存器 - 查询支持的中断数量
const GIC_DIST_TYPE: usize = 0x004;
/// 中断分组寄存器 - 设置中断为 Group0(FIQ) 或 Group1(IRQ)
const GIC_DIST_IGROUP: usize = 0x080;
/// 中断使能设置寄存器 - 启用特定中断
const GIC_DIST_ENABLE_SET: usize = 0x100;
/// 中断使能清除寄存器 - 禁用特定中断
const GIC_DIST_ENABLE_CLEAR: usize = 0x180;
/// 中断挂起设置寄存器 - 触发软件中断
const GIC_DIST_PENDING_SET: usize = 0x200;
/// 中断挂起清除寄存器 - 清除中断挂起状态
const GIC_DIST_PENDING_CLEAR: usize = 0x280;
/// 中断活跃设置寄存器
const GIC_DIST_ACTIVE_SET: usize = 0x300;
/// 中断活跃清除寄存器
const GIC_DIST_ACTIVE_CLEAR: usize = 0x380;
/// 中断优先级寄存器 - 每个中断 8 位优先级
const GIC_DIST_PRI: usize = 0x400;
/// 中断目标 CPU 寄存器 - 指定中断路由到哪个 CPU 核心
const GIC_DIST_TARGET: usize = 0x800;
/// 中断配置寄存器 - 设置中断触发方式（电平/边沿）
const GIC_DIST_CONFIG: usize = 0xC00;
/// 软件中断寄存器 - 触发 SGI
const GIC_DIST_SOFTINT: usize = 0xF00;
/// 挂起 SGI 清除寄存器
const GIC_DIST_CPENDSGI: usize = 0xF10;
/// 挂起 SGI 设置寄存器
const GIC_DIST_SPENDSGI: usize = 0xF20;
/// 外设 ID 寄存器
const GIC_DIST_ICPIDR2: usize = 0xFE8;

// ============================================================================
// CPU Interface 寄存器偏移量
// ============================================================================

/// CPU 接口控制寄存器 - 启用/禁用 CPU 接口
const GIC_CPU_CTRL: usize = 0x00;
/// 优先级掩码寄存器 - 屏蔽低优先级中断
const GIC_CPU_PRIMASK: usize = 0x04;
/// 二进制点寄存器 - 优先级分组配置
const GIC_CPU_BINPOINT: usize = 0x08;
/// 中断应答寄存器 - 读取当前最高优先级中断 ID
const GIC_CPU_INTACK: usize = 0x0C;
/// 中断结束寄存器 - 通知 GIC 中断处理完成
const GIC_CPU_EOI: usize = 0x10;
/// 运行优先级寄存器
const GIC_CPU_RUNNINGPRI: usize = 0x14;
/// 最高优先级挂起中断寄存器
const GIC_CPU_HIGHPRI: usize = 0x18;
/// 实现标识寄存器
const GIC_CPU_IIDR: usize = 0xFC;

// ============================================================================
// 全局配置常量
// ============================================================================

/// 中断起始编号（从 0 开始）
pub const GIC_IRQ_START: usize = 0;
/// 最大中断处理程序数量
pub const GIC_MAX_HANDLERS: usize = 96;

// ============================================================================
// GIC 控制器结构体
// ============================================================================

/// GIC 控制器实例
///
/// 存储 Distributor 和 CPU Interface 的基址，以及中断偏移量。
///
/// # 字段说明
///
/// - `dist_base`: Distributor 基址（典型值 0x0800_0000）
/// - `cpu_base`: CPU Interface 基址（典型值 0x0801_0000）
/// - `offset`: 中断 ID 偏移量（用于计算实际中断号）
pub struct Gic {
    dist_base: usize,
    cpu_base: usize,
    offset: usize,
}

/// 全局唯一的 GIC 控制器实例
///
/// 整个系统只有一个 GIC 控制器，使用 `static mut` 存放于 BSS 段。
static mut GIC_CTL: Gic = Gic {
    dist_base: 0,
    cpu_base: 0,
    offset: 0,
};

/// 获取 GIC 实例的可变裸指针
#[inline(always)]
unsafe fn gic_mut() -> *mut Gic {
    core::ptr::addr_of_mut!(GIC_CTL)
}

/// 获取 GIC 实例的只读裸指针
#[inline(always)]
unsafe fn gic_ref() -> *const Gic {
    core::ptr::addr_of!(GIC_CTL)
}

// ============================================================================
// 底层读写辅助函数
// ============================================================================

/// 从指定物理地址读取 32 位值
///
/// 使用 volatile 读写，确保：
/// - 不会因编译器优化而省略
/// - 读写顺序不被重排
/// - 适用于 MMIO（内存映射 I/O）
#[inline(always)]
unsafe fn read32(addr: usize) -> u32 {
    ptr::read_volatile(addr as *const u32)
}

/// 向指定物理地址写入 32 位值
///
/// 使用 volatile 写确保操作不被优化或重排。
#[inline(always)]
unsafe fn write32(addr: usize, value: u32) {
    ptr::write_volatile(addr as *mut u32, value);
}

// ============================================================================
// Distributor 寄存器地址计算函数
// ============================================================================

/// 计算 Distributor 控制寄存器地址
#[inline(always)]
fn gic_dist_ctrl(base: usize) -> usize { base + GIC_DIST_CTRL }

/// 计算 Distributor Type 寄存器地址
#[inline(always)]
fn gic_dist_type(base: usize) -> usize { base + GIC_DIST_TYPE }

/// 计算指定中断的中断分组寄存器地址
///
/// 每个寄存器管理 32 个中断，每位对应一个中断：
/// - 位 = 0: Group1 (IRQ)
/// - 位 = 1: Group0 (FIQ)
#[inline(always)]
fn gic_dist_igroup(base: usize, n: u32) -> usize {
    base + GIC_DIST_IGROUP + ((n / 32) * 4) as usize
}

/// 计算指定中断的使能设置寄存器地址
///
/// 写入 1 的位使能对应的中断。
#[inline(always)]
fn gic_dist_enable_set(base: usize, n: u32) -> usize {
    base + GIC_DIST_ENABLE_SET + ((n / 32) * 4) as usize
}

/// 计算指定中断的使能清除寄存器地址
///
/// 写入 1 的位禁用对应的中断。
#[inline(always)]
fn gic_dist_enable_clear(base: usize, n: u32) -> usize {
    base + GIC_DIST_ENABLE_CLEAR + ((n / 32) * 4) as usize
}

/// 计算指定中断的挂起设置寄存器地址
#[inline(always)]
fn gic_dist_pending_set(base: usize, n: u32) -> usize {
    base + GIC_DIST_PENDING_SET + ((n / 32) * 4) as usize
}

/// 计算指定中断的挂起清除寄存器地址
#[inline(always)]
fn gic_dist_pending_clear(base: usize, n: u32) -> usize {
    base + GIC_DIST_PENDING_CLEAR + ((n / 32) * 4) as usize
}

/// 计算指定中断的活跃设置寄存器地址
#[inline(always)]
fn gic_dist_active_set(base: usize, n: u32) -> usize {
    base + GIC_DIST_ACTIVE_SET + ((n / 32) * 4) as usize
}

/// 计算指定中断的活跃清除寄存器地址
#[inline(always)]
fn gic_dist_active_clear(base: usize, n: u32) -> usize {
    base + GIC_DIST_ACTIVE_CLEAR + ((n / 32) * 4) as usize
}

/// 计算指定中断的优先级寄存器地址
///
/// 每个中断分配 8 位优先级，每 4 个中断共享一个 32 位寄存器。
/// 优先级值越小，优先级越高。
#[inline(always)]
fn gic_dist_pri(base: usize, n: u32) -> usize {
    base + GIC_DIST_PRI + ((n / 4) * 4) as usize
}

/// 计算指定中断的目标 CPU 寄存器地址
///
/// 每 4 个中断共享一个 32 位寄存器，每个中断分配 8 位：
/// - 位 0: 路由到 CPU0
/// - 位 1: 路由到 CPU1
/// - ...
#[inline(always)]
fn gic_dist_target(base: usize, n: u32) -> usize {
    base + GIC_DIST_TARGET + ((n / 4) * 4) as usize
}

/// 计算指定中断的配置寄存器地址
///
/// 每 16 个中断共享一个 32 位寄存器，每个中断分配 2 位：
/// - 0b00: 电平敏感
/// - 0b01: 边沿触发
#[inline(always)]
fn gic_dist_config(base: usize, n: u32) -> usize {
    base + GIC_DIST_CONFIG + ((n / 16) * 4) as usize
}

/// 计算软件中断寄存器地址
#[inline(always)]
fn gic_dist_softint(base: usize) -> usize { base + GIC_DIST_SOFTINT }

// ============================================================================
// CPU Interface 寄存器地址计算函数
// ============================================================================

/// 计算 CPU 接口控制寄存器地址
#[inline(always)]
fn gic_cpu_ctrl(base: usize) -> usize { base + GIC_CPU_CTRL }

/// 计算优先级掩码寄存器地址
#[inline(always)]
fn gic_cpu_primask(base: usize) -> usize { base + GIC_CPU_PRIMASK }

/// 计算二进制点寄存器地址
#[inline(always)]
fn gic_cpu_binpoint(base: usize) -> usize { base + GIC_CPU_BINPOINT }

/// 计算中断应答寄存器地址
#[inline(always)]
fn gic_cpu_intack(base: usize) -> usize { base + GIC_CPU_INTACK }

/// 计算中断结束寄存器地址
#[inline(always)]
fn gic_cpu_eoi(base: usize) -> usize { base + GIC_CPU_EOI }

/// 计算最高优先级挂起中断寄存器地址
#[inline(always)]
fn gic_cpu_highpri(base: usize) -> usize { base + GIC_CPU_HIGHPRI }

// ============================================================================
// Gic 方法实现
// ============================================================================

impl Gic {
    /// 初始化 GIC Distributor
    ///
    /// 配置分发器以接受和管理中断：
    ///
    /// 1. 读取 TYPE 寄存器获取支持的最大中断数
    /// 2. 计算 CPU 掩码（路由到 CPU0 的所有 4 组）
    /// 3. 禁用 Distributor
    /// 4. 配置所有 SPI 中断：
    ///    - 设置为电平敏感触发
    ///    - 路由到 CPU0
    ///    - 设置默认优先级 0xA0
    ///    - 清除所有使能位
    /// 5. 启用 Distributor
    ///
    /// # 参数
    ///
    /// - `dist_base`: Distributor 物理基址
    /// - `irq_start`: 中断 ID 起始偏移
    pub unsafe fn dist_init(&mut self, dist_base: usize, irq_start: usize) {
        self.dist_base = dist_base;
        self.offset = irq_start;

        // 计算支持的中断数量
        let gic_type = read32(gic_dist_type(dist_base));
        let gic_max_irq = (((gic_type & 0x1F) + 1) * 32) as usize;
        let gic_max_irq = if gic_max_irq > 1020 { 1020 } else { gic_max_irq };

        // 构造 CPU 掩码：路由到 CPU0
        // 0b01010101 填充到 32 位 → 每 8 位选 CPU0
        let mut cpu_mask: u32 = 1 << 0;
        cpu_mask |= cpu_mask << 8;
        cpu_mask |= cpu_mask << 16;
        cpu_mask |= cpu_mask << 24;

        // 禁用 Distributor（配置阶段）
        write32(gic_dist_ctrl(dist_base), 0x0);

        // 配置 SPI 中断为电平敏感
        let mut i = 32;
        while i < gic_max_irq {
            write32(gic_dist_config(dist_base, i as u32), 0x00);
            i += 16;
        }

        // 所有 SPI 路由到 CPU0
        i = 32;
        while i < gic_max_irq {
            write32(gic_dist_target(dist_base, i as u32), cpu_mask);
            i += 4;
        }

        // 设置默认优先级
        i = 0;
        while i < gic_max_irq {
            write32(gic_dist_pri(dist_base, i as u32), 0xa0a0_a0a0);
            i += 4;
        }

        // 清除所有中断使能
        i = 0;
        while i < gic_max_irq {
            write32(gic_dist_enable_clear(dist_base, i as u32), 0xffff_ffff);
            i += 32;
        }

        // 设置中断分组为 Group0
        i = 0;
        while i < gic_max_irq {
            write32(gic_dist_igroup(dist_base, i as u32), 0x00);
            i += 32;
        }

        // 启用 Distributor
        write32(gic_dist_ctrl(dist_base), 0x01);
    }

    /// 初始化 GIC CPU Interface
    ///
    /// 配置当前 CPU 核心的中断接口：
    ///
    /// 1. 设置优先级掩码为 0xF0（允许所有优先级中断）
    /// 2. 设置二进制点为 0x07（最低 3 位为子优先级）
    /// 3. 启用 CPU 接口
    ///
    /// # 参数
    ///
    /// - `cpu_base`: CPU Interface 物理基址
    pub unsafe fn cpu_init(&mut self, cpu_base: usize) {
        self.cpu_base = cpu_base;

        // 优先级掩码：0xF0 允许优先级 0-0xEF 的中断
        write32(gic_cpu_primask(cpu_base), 0xF0);

        // 二进制点：0x07 分界，高位为组优先级，低 3 位为子优先级
        write32(gic_cpu_binpoint(cpu_base), 0x07);

        // 启用 CPU 接口
        write32(gic_cpu_ctrl(cpu_base), 0x01);
    }

    /// 启用指定中断
    ///
    /// 通过设置 Distributor 使能寄存器中的对应位来启用中断。
    ///
    /// # 参数
    ///
    /// - `irq_num`: 中断 ID
    pub unsafe fn enable_irq(&self, irq_num: u32) {
        let mask: u32 = 1 << (irq_num % 32);
        write32(gic_dist_enable_set(self.dist_base, irq_num), mask);
    }

    /// 禁用指定中断
    ///
    /// 通过清除 Distributor 使能寄存器中的对应位来禁用中断。
    ///
    /// # 参数
    ///
    /// - `irq_num`: 中断 ID
    pub unsafe fn disable_irq(&self, irq_num: u32) {
        let mask: u32 = 1 << (irq_num % 32);
        write32(gic_dist_enable_clear(self.dist_base, irq_num), mask);
    }

    /// 获取当前最高优先级中断 ID
    ///
    /// 读取 CPU 接口的中断应答寄存器，返回当前待处理中断的 ID。
    /// 返回 -1 表示没有中断等待。
    pub unsafe fn get_irq(&self) -> i32 {
        let irq = read32(gic_cpu_intack(self.cpu_base)) as i32;
        irq + self.offset as i32
    }

    /// 中断应答（End of Interrupt）
    ///
    /// 中断处理完成后调用，通知 GIC 中断已处理：
    ///
    /// 1. 清除 Distributor 中的挂起状态
    /// 2. 写入 CPU 接口的 EOI 寄存器
    ///
    /// # 参数
    ///
    /// - `vector`: 中断向量号（通过 `get_irq()` 获取）
    pub unsafe fn ack_irq(&self, vector: i32) {
        let mask: u32 = 1 << ((vector % 32) as u32);
        let irq = vector - self.offset as i32;

        // 对于有效中断，清除挂起状态
        if irq >= 0 {
            write32(gic_dist_pending_clear(self.dist_base, irq as u32), mask);
        }

        // 通知 GIC 中断处理完成
        write32(gic_cpu_eoi(self.cpu_base), irq as u32);
    }
}

// ============================================================================
// 公共 API 函数（封装全局实例操作）
// ============================================================================

/// 初始化 GIC Distributor（公共接口）
pub unsafe fn gic_dist_init(dist_base: usize, irq_start: usize) {
    (*gic_mut()).dist_init(dist_base, irq_start);
}

/// 初始化 GIC CPU Interface（公共接口）
pub unsafe fn gic_cpu_init(cpu_base: usize) {
    (*gic_mut()).cpu_init(cpu_base);
}

/// 启用指定中断（公共接口）
///
/// # 使用示例
///
/// ```ignore
/// // 启用定时器中断（PPI，ID=27）
/// gic_enable_irq(27);
/// ```
pub unsafe fn gic_enable_irq(irq_num: u32) {
    (*gic_ref()).enable_irq(irq_num);
}

/// 禁用指定中断（公共接口）
pub unsafe fn gic_disable_irq(irq_num: u32) {
    (*gic_ref()).disable_irq(irq_num);
}

/// 获取当前中断 ID（公共接口）
///
/// 在中断处理程序中使用，获取触发中断的 ID。
pub unsafe fn gic_get_irq() -> i32 {
    (*gic_ref()).get_irq()
}

/// 中断应答（公共接口）
///
/// 中断处理完成后调用，清理中断状态。
pub unsafe fn gic_ack_irq(vector: i32) {
    (*gic_ref()).ack_irq(vector);
}

/// GIC 完整初始化（公共接口）
///
/// 一步完成 Distributor 和 CPU Interface 的初始化。
/// 使用预设的基址 `GIC_DIST_BASE` 和 `GIC_CPU_BASE`。
pub unsafe fn gic_init() {
    gic_dist_init(GIC_DIST_BASE, GIC_IRQ_START);
    gic_cpu_init(GIC_CPU_BASE);
}
