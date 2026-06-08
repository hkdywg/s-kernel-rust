//! 内核通用定义
//!
//! 本 crate 定义内核中跨模块共享的基础类型，避免循环依赖：
//!
//! - [`Thread`] — 线程控制块（TCB）
//! - [`ThreadState`] — 线程生命周期状态
//! - [`list`] — 侵入式双向循环链表
//!
//! 各子系统只需依赖 `common`，即可使用线程、链表等通用结构体。

#![no_std]

pub mod list;

use list::ListNode;

/// 线程/文件名最大字节数
pub const NAME_MAX: usize = 32;

// ============================================================================
// 线程状态
// ============================================================================

/// 线程生命周期状态
///
/// # 状态转换
///
/// ```text
///                   ┌─────────┐
///            ┌────→ │  Ready   │ ←─────┐
///            │      └────┬─────┘       │
///            │   调度选中  │  时间片耗尽  │
///            │      ┌────↓─────┐       │
/// 创建 ┌─────┴─┐   │  Running  │   ┌──┴─────┐  退出
/// ───→ │ Init │──→└───────────┘──→│ Suspend │──→│ Close │
///      └──────┘                   └────────┘
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// 初始化 — 刚创建，尚未加入调度就绪队列
    Init,
    /// 就绪 — 等待 CPU 调度
    Ready,
    /// 运行中 — 当前正在 CPU 上执行
    Running,
    /// 挂起 — 等待事件（信号量、延时等），不参与调度
    Suspend,
    /// 已关闭 — 线程退出，等待回收
    Close,
}

// ============================================================================
// 线程控制块（TCB）
// ============================================================================

/// 线程控制块
///
/// 每个线程一个实例，存储线程的所有状态。
/// 通过嵌入的 [`ListNode`] 挂载到调度器的就绪/挂起链表中。
///
/// # `#[repr(C)]`
///
/// 保证字段顺序与 C 兼容，汇编代码（上下文切换）可直接按偏移访问 `sp`。
///
/// # 字段分组
///
/// | 分组   | 字段 | 说明 |
/// |--------|------|------|
/// | 调度   | `state`, `current_priority`, `init_priority`, `number_mask`, `init_tick`, `remain_tick` | 线程调度相关 |
/// | 栈     | `sp`, `stack_addr`, `stack_size` | 栈指针和栈信息 |
/// | 入口   | `entry`, `parameter` | 线程入口函数和参数 |
/// | 标识   | `name` | 线程名称 |
/// | 定时   | `thread_timer` | 关联的定时器（延时用） |
/// | 清理   | `cleanup` | 线程退出时回调 |
/// | 链表   | `list_node` | 嵌入的链表节点，挂入调度队列 |
/// | 扩展   | `user_data` | 用户自定义数据 |
#[repr(C)]
pub struct Thread {
    /// 线程名称（最多 NAME_MAX 字节）
    pub name: [u8; NAME_MAX],
    /// 栈指针 — 上下文切换时保存/恢复
    pub sp: u64,
    /// 线程入口函数，接受一个 `*mut u8` 参数且永不返回
    pub entry: extern "C" fn(*mut u8) -> !,
    /// 传递给入口函数的参数
    pub parameter: *mut u8,
    /// 线程栈基址（低地址）
    pub stack_addr: *mut u8,
    /// 线程栈大小（字节）
    pub stack_size: usize,
    /// 当前优先级（可能因优先级继承等动态变化）
    pub current_priority: u8,
    /// 初始优先级（创建时指定，基准值）
    pub init_priority: u8,
    /// 线程状态
    pub state: ThreadState,
    /// 调度器位图掩码 — 在优先级位图中定位
    pub number_mask: u32,
    /// 嵌入的链表节点，挂入就绪/挂起等队列
    pub list_node: ListNode,
    /// 初始时间片（tick 数）
    pub init_tick: u32,
    /// 剩余时间片，减到 0 时让出 CPU
    pub remain_tick: u32,
    /// 定时器指针，实现线程延时唤醒
    pub thread_timer: *mut u8,
    /// 线程退出时调用的清理函数
    pub cleanup: Option<unsafe fn(*mut Thread)>,
    /// 用户自定义数据（扩展用）
    pub user_data: usize,
}