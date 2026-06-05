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
//! | 16-31   | PPI  | 私有外设中断（每核独立）    |
//! | 32-1019 | SPI  | 共享外设中断（UART、磁盘等） |

#![allow(dead_code)]
use core::ptr;

// ============================================================================
// 基址常量
// ============================================================================

/// GIC Distributor（分发器）基址
pub const GIC_DIST_BASE: usize = 0x0800_0000;
/// GIC CPU Interface 基址
pub const GIC_CPU_BASE: usize = 0x0801_0000;

/// 中断起始编号
pub const GIC_IRQ_START: usize = 0;
/// 最大中断处理程序数量
pub const GIC_MAX_HANDLERS: usize = 96;

// ============================================================================
// GIC 寄存器枚举 —— 所有寄存器地址统一由此计算
// ============================================================================

/// GIC 寄存器标识
///
/// 将分散的 19 个地址计算函数统一为一个枚举，通过 [`Gic::reg_addr`] 方法
/// 计算物理地址。带参数的变体用于按中断 ID 分组的寄存器（如 Enable、Priority）。
///
/// # 分组规则
///
/// | 粒度   | 寄存器                          |
/// |--------|--------------------------------|
/// | 单寄存器 | Ctrl, Type, SoftInt, CpuCtrl…   |
/// | 32 个/reg | Enable, Pending, Active, IGroup  |
/// | 16 个/reg | Config                           |
/// | 4 个/reg  | Target, Priority                 |
#[derive(Clone, Copy)]
enum GicReg {
    // ── Distributor 寄存器 ──
    /// 控制寄存器 —— 启用/禁用整个分发器
    DistCtrl,
    /// Type 寄存器 —— 查询支持的中断数量
    DistType,
    /// 中断分组（每 32 中断一个寄存器）：0=Group1(IRQ), 1=Group0(FIQ)
    DistIGroup(u32),
    /// 中断使能设置（写 1 启用）
    DistEnableSet(u32),
    /// 中断使能清除（写 1 禁用）
    DistEnableClear(u32),
    /// 中断挂起设置
    DistPendingSet(u32),
    /// 中断挂起清除
    DistPendingClear(u32),
    /// 中断活跃设置
    DistActiveSet(u32),
    /// 中断活跃清除
    DistActiveClear(u32),
    /// 中断优先级（每 4 个中断一个寄存器，每中断 8 位）
    DistPri(u32),
    /// 中断目标 CPU（每 4 个中断一个寄存器，每中断 8 位掩码）
    DistTarget(u32),
    /// 中断配置（电平/边沿，每 16 个中断一个寄存器）
    DistConfig(u32),
    /// 软件中断触发
    DistSoftInt,

    // ── CPU Interface 寄存器 ──
    /// CPU 接口控制
    CpuCtrl,
    /// 优先级掩码
    CpuPriMask,
    /// 二进制点（优先级分组）
    CpuBinPoint,
    /// 中断应答 —— 读取当前最高优先级中断 ID
    CpuIntAck,
    /// 中断结束 —— 通知 GIC 处理完成
    CpuEoi,
    /// 最高优先级挂起中断
    CpuHighPri,
}

// ============================================================================
// GIC 控制器结构体
// ============================================================================

/// GIC 控制器实例
pub struct Gic {
    dist_base: usize,
    cpu_base: usize,
    offset: usize,
}

/// 全局唯一的 GIC 控制器实例
///
/// 整个系统只有一个 GIC，存放于 BSS 段。不直接创建 `&mut` / `&` 引用，
/// 所有访问通过 `gic_mut()` / `gic_ref()` 获取裸指针操作。
static mut GIC_CTL: Gic = Gic {
    dist_base: 0,
    cpu_base: 0,
    offset: 0,
};

#[inline(always)]
unsafe fn gic_mut() -> *mut Gic { core::ptr::addr_of_mut!(GIC_CTL) }
#[inline(always)]
unsafe fn gic_ref() -> *const Gic { core::ptr::addr_of!(GIC_CTL) }

// ============================================================================
// 底层 volatile 读写
// ============================================================================

#[inline(always)]
unsafe fn read32(addr: usize) -> u32 { ptr::read_volatile(addr as *const u32) }
#[inline(always)]
unsafe fn write32(addr: usize, value: u32) { ptr::write_volatile(addr as *mut u32, value); }

// ============================================================================
// Gic 方法实现
// ============================================================================

impl Gic {
    /// 计算指定寄存器的物理地址
    ///
    /// 根据 `dist_base` / `cpu_base` 和寄存器类型自动计算偏移。
    fn reg_addr(&self, reg: GicReg) -> usize {
        match reg {
            GicReg::DistCtrl          => self.dist_base + 0x000,
            GicReg::DistType          => self.dist_base + 0x004,
            GicReg::DistIGroup(n)     => self.dist_base + 0x080 + (n as usize / 32) * 4,
            GicReg::DistEnableSet(n)  => self.dist_base + 0x100 + (n as usize / 32) * 4,
            GicReg::DistEnableClear(n)=> self.dist_base + 0x180 + (n as usize / 32) * 4,
            GicReg::DistPendingSet(n) => self.dist_base + 0x200 + (n as usize / 32) * 4,
            GicReg::DistPendingClear(n)=>self.dist_base + 0x280 + (n as usize / 32) * 4,
            GicReg::DistActiveSet(n)  => self.dist_base + 0x300 + (n as usize / 32) * 4,
            GicReg::DistActiveClear(n)=> self.dist_base + 0x380 + (n as usize / 32) * 4,
            GicReg::DistPri(n)        => self.dist_base + 0x400 + (n as usize / 4) * 4,
            GicReg::DistTarget(n)     => self.dist_base + 0x800 + (n as usize / 4) * 4,
            GicReg::DistConfig(n)     => self.dist_base + 0xC00 + (n as usize / 16) * 4,
            GicReg::DistSoftInt       => self.dist_base + 0xF00,

            GicReg::CpuCtrl           => self.cpu_base + 0x00,
            GicReg::CpuPriMask        => self.cpu_base + 0x04,
            GicReg::CpuBinPoint       => self.cpu_base + 0x08,
            GicReg::CpuIntAck         => self.cpu_base + 0x0C,
            GicReg::CpuEoi            => self.cpu_base + 0x10,
            GicReg::CpuHighPri        => self.cpu_base + 0x18,
        }
    }

    // ── 初始化 ────────────────────────────────────────────

    /// 初始化 GIC Distributor
    ///
    /// 配置所有 SPI 中断：电平敏感 → 路由到 CPU0 → 默认优先级 → 清除使能。
    pub unsafe fn dist_init(&mut self, dist_base: usize, irq_start: usize) {
        self.dist_base = dist_base;
        self.offset = irq_start;

        let gic_type = read32(self.reg_addr(GicReg::DistType));
        let gic_max_irq = (((gic_type & 0x1F) + 1) * 32).min(1020) as usize;

        // CPU 掩码：每 8 位选 CPU0，重复填充 32 位
        let cpu_mask = 0x0101_0101u32;

        // 禁用 Distributor
        write32(self.reg_addr(GicReg::DistCtrl), 0x0);

        // SPI 中断 → 电平敏感
        for i in (32..gic_max_irq).step_by(16) {
            write32(self.reg_addr(GicReg::DistConfig(i as u32)), 0x00);
        }
        // SPI 中断 → 路由到 CPU0
        for i in (32..gic_max_irq).step_by(4) {
            write32(self.reg_addr(GicReg::DistTarget(i as u32)), cpu_mask);
        }
        // 默认优先级 0xA0
        for i in (0..gic_max_irq).step_by(4) {
            write32(self.reg_addr(GicReg::DistPri(i as u32)), 0xa0a0_a0a0);
        }
        // 清除所有使能
        for i in (0..gic_max_irq).step_by(32) {
            write32(self.reg_addr(GicReg::DistEnableClear(i as u32)), 0xffff_ffff);
        }
        // 中断分组 → Group0
        for i in (0..gic_max_irq).step_by(32) {
            write32(self.reg_addr(GicReg::DistIGroup(i as u32)), 0x00);
        }

        // 启用 Distributor
        write32(self.reg_addr(GicReg::DistCtrl), 0x01);
    }

    /// 初始化 GIC CPU Interface
    ///
    /// 优先级掩码 0xF0 → 允许优先级 0~0xEF；二进制点 0x07 → 低 3 位为子优先级。
    pub unsafe fn cpu_init(&mut self, cpu_base: usize) {
        self.cpu_base = cpu_base;
        write32(self.reg_addr(GicReg::CpuPriMask), 0xF0);
        write32(self.reg_addr(GicReg::CpuBinPoint), 0x07);
        write32(self.reg_addr(GicReg::CpuCtrl), 0x01);
    }

    // ── 中断控制 ──────────────────────────────────────────

    /// 启用指定中断
    pub unsafe fn enable_irq(&self, irq_num: u32) {
        write32(self.reg_addr(GicReg::DistEnableSet(irq_num)), 1 << (irq_num % 32));
    }

    /// 禁用指定中断
    pub unsafe fn disable_irq(&self, irq_num: u32) {
        write32(self.reg_addr(GicReg::DistEnableClear(irq_num)), 1 << (irq_num % 32));
    }

    /// 获取当前最高优先级中断 ID，无中断时返回 -1
    pub unsafe fn get_irq(&self) -> i32 {
        read32(self.reg_addr(GicReg::CpuIntAck)) as i32 + self.offset as i32
    }

    /// 中断应答 —— 清除挂起状态并写 EOI
    pub unsafe fn ack_irq(&self, vector: i32) {
        let irq = vector - self.offset as i32;
        if irq >= 0 {
            write32(self.reg_addr(GicReg::DistPendingClear(irq as u32)),
                    1 << (irq as u32 % 32));
        }
        write32(self.reg_addr(GicReg::CpuEoi), irq as u32);
    }
}

// ============================================================================
// 公共 API —— 封装全局实例的裸指针访问
// ============================================================================

pub unsafe fn gic_dist_init(dist_base: usize, irq_start: usize) { (*gic_mut()).dist_init(dist_base, irq_start); }
pub unsafe fn gic_cpu_init(cpu_base: usize)                     { (*gic_mut()).cpu_init(cpu_base); }
pub unsafe fn gic_enable_irq(irq_num: u32)                      { (*gic_ref()).enable_irq(irq_num); }
pub unsafe fn gic_disable_irq(irq_num: u32)                     { (*gic_ref()).disable_irq(irq_num); }
pub unsafe fn gic_get_irq() -> i32                              { (*gic_ref()).get_irq() }
pub unsafe fn gic_ack_irq(vector: i32)                          { (*gic_ref()).ack_irq(vector); }

/// GIC 完整初始化，使用预设基址
pub unsafe fn gic_init() {
    gic_dist_init(GIC_DIST_BASE, GIC_IRQ_START);
    gic_cpu_init(GIC_CPU_BASE);
}
