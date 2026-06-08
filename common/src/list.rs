//! 双向循环链表
//!
//! 实现 Linux 内核风格的侵入式链表：
//!
//! - 链表节点（`ListNode`）**嵌入**在宿主结构体中，而非独立分配
//! - 通过 [`list_entry!`] 宏从节点指针反算出宿主结构体地址
//! - 头节点本身不携带数据，仅作为哨兵
//!
//! # 侵入式 vs 独立式
//!
//! ```text
//! 独立式（std::collections::LinkedList）：
//!   Task1 → Node → Task1  |  Task2 → Node → Task2
//!   ↑ 两次分配、两次间接访问
//!
//! 侵入式（本模块）：
//!   Task1 { node, ... } ⇄ Task2 { node, ... }
//!   ↑ 零额外分配、零间接访问
//! ```

use core::ptr;

/// 双向链表节点

///      ----------------------------------------------------- 
///      |                                                   | 
///      |    node        node        node        node       | 
///      |   +++++++     +++++++     +++++++     +++++++     | 
///      --->|     |     |     |     |     |     |     |     | 
///          |  n  |---->|  n  |---->|  n  |---->|  n  |------ 
///      ----|  p  |<----|  p  |<----|  p  |<----|  p  |       
///      |   |     |     |     |     |     |     |     |<----- 
///      |   +++++++     +++++++     +++++++     +++++++     | 
///      |                                                   | 
///       ---------------------------------------------------- 
///
///
/// 嵌入在宿主结构体中，形成双向循环链表。
///
/// # 使用方式
///
/// ```ignore
/// struct Task {
///     pid: u32,
///     node: ListNode,  // ← 嵌入，不需要单独分配
/// }
/// ```
///
/// # `#[repr(C)]`
///
/// 保证 `prev` / `next` 字段顺序和偏移与 C 一致，便于汇编或 C 代码访问。
#[repr(C)]
pub struct ListNode {
    pub prev: *mut ListNode,
    pub next: *mut ListNode,
}

// ============================================================================
// 基本操作
// ============================================================================

/// 初始化链表头
///
/// 头节点的 `prev` 和 `next` 都指向自身，表示空链表。
///
/// ```text
/// 初始状态:  head ⇄ head
/// ```
pub unsafe fn list_init(list: *mut ListNode) {
    (*list).prev = list;
    (*list).next = list;
}

/// 在头部之后插入（头插法）
///
/// 新节点插入在 `head` 和 `head->next` 之间。
///
/// ```text
/// 插入前:  head ⇄ A
/// 插入后:  head ⇄ new ⇄ A
/// ```
pub unsafe fn list_add(head: *mut ListNode, new: *mut ListNode) {
    (*new).next = (*head).next;
    (*new).prev = head;

    (*(*head).next).prev = new;
    (*head).next = new;
}

/// 在尾部插入（尾插法）
///
/// 新节点插入在 `head->prev` 和 `head` 之间。
///
/// ```text
/// 插入前:  A ⇄ head
/// 插入后:  A ⇄ new ⇄ head
/// ```
pub unsafe fn list_add_tail(head: *mut ListNode, new: *mut ListNode) {
    (*new).next = head;
    (*new).prev = (*head).prev;

    (*(*head).prev).next = new;
    (*head).prev = new;
}

/// 删除指定节点
///
/// 被删除节点的 `prev`/`next` 指向自身，便于调试检测 use-after-free。
pub unsafe fn list_del(entry: *mut ListNode) {
    (*(*entry).prev).next = (*entry).next;
    (*(*entry).next).prev = (*entry).prev;

    // 指向自身，标记为已脱离链表
    (*entry).prev = entry;
    (*entry).next = entry;
}

// ============================================================================
// 查询操作
// ============================================================================

/// 判断链表是否为空
///
/// 头节点的 `next` 指向自身即为空。
pub unsafe fn list_empty(list: *mut ListNode) -> bool {
    (*list).next == list
}

/// 获取第一个有效节点
///
/// 假设 `ListNode` 是宿主结构体的**第一个字段**，通过指针转换直接返回宿主指针。
/// 如果链表为空，返回 `null_mut()`。
///
/// ⚠️ 当 ListNode 不是第一个字段时，请用 [`list_entry!`] 宏代替。
pub unsafe fn list_first_entry(head: *mut ListNode) -> *mut u8 {
    if list_empty(head) {
        return ptr::null_mut();
    }
    (*head).next as *mut u8
}

/// 遍历链表，对每个节点调用回调
pub unsafe fn list_for_each(head: *mut ListNode, callback: unsafe fn(*mut ListNode)) {
    let mut pos = (*head).next;
    while pos != head {
        callback(pos);
        pos = (*pos).next;
    }
}

/// 返回链表节点数
pub unsafe fn list_len(head: *mut ListNode) -> usize {
    let mut count = 0;
    let mut pos = (*head).next;
    while pos != head {
        count += 1;
        pos = (*pos).next;
    }
    count
}

// ============================================================================
// container_of 宏
// ============================================================================

/// 从内嵌的 `ListNode` 指针还原宿主结构体指针
///
/// 内核链表中存储的是 `ListNode` 节点，业务代码需要的是包含它的结构体。
/// 此宏等价于 Linux 内核的 `container_of`。
///
/// # 原理
///
/// ```text
///                    ptr 指向 node 字段
///                         ↓
/// ┌─────────────────────┬──────────┐
/// │   其他字段           │  node    │
/// └────── offset ──────→┴──────────┘
///   ↑ 宿主地址 = ptr - offset
/// ```
///
/// # 使用
///
/// ```ignore
/// // Task 结构体
/// struct Task { pid: u32, node: ListNode }
///
/// // 遍历时还原
/// let task: *mut Task = list_entry!(node_ptr, Task, node);
/// ```
///
/// # 参数
///
/// - `$ptr`: 指向 `ListNode` 的指针
/// - `$type`: 宿主结构体类型
/// - `$member`: `ListNode` 字段在宿主结构体中的名称
#[macro_export]
macro_rules! list_entry {
    ($ptr:expr, $type:ty, $member:ident) => {
        ($ptr as *mut u8)
            .byte_sub(core::mem::offset_of!($type, $member))
            as *mut $type
    };
}
