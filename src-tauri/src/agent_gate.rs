//! Agent 提交的并发闸门（同时执行的会话数上限）。
//!
//! 值来自设置页 `agent_max_concurrency`，用途是**在 pending → running 处限流**：
//! `dispatch_run` 拿到执行位才把 run 标 running 并开始执行，拿不到就停在 pending
//! 排队（前端据此显示「已达并发上限」）。
//!
//! # 为什么不直接用「容量 = 上限」的裸 `Semaphore`
//!
//! 上限可在运行中修改，而 `Semaphore` 的容量只能间接调整：`add_permits` 立即生效，
//! 但 `forget_permits` 只减得掉**当前可用**的许可（tokio 1.52 的
//! `batch_semaphore::forget_permits` 就是一次 `saturating_sub`，减不动的差额直接
//! 丢弃、返回值被忽略）。于是「3 个 run 在跑时把上限从 3 调到 1」会静默减掉 0 个，
//! 3 个跑完后可用位又回到 3 —— 闸门比设置值宽，且后续操作不会纠正它，只能重启应用；
//! 同时设置页/队列状态读的是 DB 里的 1，UI 显示与真实并发长期不一致。

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{AcquireError, OwnedSemaphorePermit, Semaphore, SemaphorePermit};

use crate::db::agent::MAX_AGENT_CONCURRENCY;

/// Agent 提交的并发闸门。
///
/// # 做法：信号量容量恒定，闸门常驻持有一部分「保留许可」
///
/// 信号量容量固定为 `MAX_AGENT_CONCURRENCY`（**永不变化**，因此不存在需要
/// `forget_permits` 补差额的场景），可用执行位 = 上限，由闸门自己常驻持有的
/// `MAX − 上限` 个保留许可决定：
///
/// - **抬升上限**：归还（drop）多余的保留许可，立即生效。
/// - **降低上限**：保留许可不够时去借；借不到就先挂在信号量队列里（FIFO 队首，
///   新提交排在它后面）—— 已运行的 run 不受影响，只是不再有新的并发位可用，
///   等某个 run 收尾归还许可时补上。
///
/// 借位任务每拿到一个许可都复查一次目标保留数：若期间上限被抬回去，它就不再占用
/// （把许可还回信号量）。因此反复升降不会累积漂移。
pub struct ConcurrencyGate {
    /// 执行位与保留位都取自这里；容量恒为 `MAX_AGENT_CONCURRENCY`。
    semaphore: Arc<Semaphore>,
    /// 保留许可（= `MAX_AGENT_CONCURRENCY` − 当前上限）。stdlib Mutex：只在同步
    /// 代码里短暂持有，不跨 await。
    reserved: Mutex<Vec<OwnedSemaphorePermit>>,
    /// 当前上限（1..=MAX_AGENT_CONCURRENCY）。
    limit: AtomicI64,
}

impl ConcurrencyGate {
    /// 建闸门（`limit` 越界时收敛到 [1, MAX_AGENT_CONCURRENCY]）。
    pub fn new(limit: i64) -> Arc<Self> {
        let gate = Arc::new(ConcurrencyGate {
            semaphore: Arc::new(Semaphore::new(MAX_AGENT_CONCURRENCY as usize)),
            reserved: Mutex::new(Vec::new()),
            limit: AtomicI64::new(limit.clamp(1, MAX_AGENT_CONCURRENCY)),
        });
        // 建闸门时不可能有 run：同步取满保留位，无需借位任务
        gate.reserve_available();
        gate
    }

    /// 取一个执行位（FIFO；闸门被关闭时 Err）。
    pub async fn acquire(&self) -> Result<SemaphorePermit<'_>, AcquireError> {
        self.semaphore.acquire().await
    }

    /// 调整上限（就地生效，不重启应用）；返回实际生效的上限。
    pub fn set_limit(self: &Arc<Self>, limit: i64) -> i64 {
        let limit = limit.clamp(1, MAX_AGENT_CONCURRENCY);
        self.limit.store(limit, Ordering::SeqCst);

        // 抬升上限 → 目标保留数变小：多余的就地归还（立即生效）
        let target = self.target_reserved();
        {
            let mut reserved = self.reserved.lock().unwrap();
            while reserved.len() > target {
                reserved.pop();
            }
        }
        // 降低上限 → 目标保留数变大：能同步借到的立刻拿到（空闲时就是全部），
        // 借不到的部分（许可正被 run 借出）交给后台任务等归还时补。
        self.reserve_available();
        if self.reserved.lock().unwrap().len() < self.target_reserved() {
            self.spawn_reserve_task();
        }
        limit
    }

    /// 当前上限。
    pub fn limit(&self) -> i64 {
        self.limit.load(Ordering::SeqCst)
    }

    /// 当前可用执行位数（诊断 / 测试用）。
    pub fn available(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// 从 DB 重新对齐上限（读失败时保持现状）。
    ///
    /// 备份导入是「整库替换 app_settings」，闸门这个进程内缓存值不会自己更新——
    /// 不重新对齐就会出现「设置页显示 1、实际按 3 并行」，直到重启应用。
    pub fn sync_from_db(self: &Arc<Self>, conn: &rusqlite::Connection) -> i64 {
        let limit = match crate::db::agent::load_agent_config(conn) {
            Ok(cfg) => cfg.max_concurrency,
            Err(e) => {
                log::warn!("agent gate limit not synced: {}", e);
                self.limit()
            }
        };
        self.set_limit(limit)
    }

    /// 目标保留许可数 = MAX − 上限。
    fn target_reserved(&self) -> usize {
        (MAX_AGENT_CONCURRENCY - self.limit.load(Ordering::SeqCst)) as usize
    }

    /// 同步补齐保留位（能拿多少拿多少）：建闸门与「空闲时降低上限」都能一次拿满，
    /// 无需等后台任务；拿不到的差额由 `spawn_reserve_task` 等 run 归还。
    fn reserve_available(&self) {
        let target = self.target_reserved();
        let mut reserved = self.reserved.lock().unwrap();
        while reserved.len() < target {
            let Ok(permit) = self.semaphore.clone().try_acquire_owned() else {
                break;
            };
            reserved.push(permit);
        }
    }

    /// 后台补齐保留位：借不到就挂在队列里等 run 收尾归还。
    ///
    /// 复核（拿到许可后重读目标值）是必要的：借的过程中上限可能被抬回去，
    /// 此时这一位不该再占（占着会让闸门比设置值窄）。
    fn spawn_reserve_task(self: &Arc<Self>) {
        let gate = self.clone();
        tokio::spawn(async move {
            loop {
                if gate.reserved.lock().unwrap().len() >= gate.target_reserved() {
                    return;
                }
                let Ok(permit) = gate.semaphore.clone().acquire_owned().await else {
                    return;
                };
                let mut reserved = gate.reserved.lock().unwrap();
                if reserved.len() >= gate.target_reserved() {
                    // 上限已被抬回：不占这一位（drop 归还给信号量），退出
                    return;
                }
                reserved.push(permit);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 越界上限收敛到合法区间：0 会让所有提交永久排队，超大值会放开进程数。
    #[test]
    fn new_clamps_limit_into_range() {
        assert_eq!(ConcurrencyGate::new(0).limit(), 1);
        assert_eq!(ConcurrencyGate::new(-5).limit(), 1);
        assert_eq!(ConcurrencyGate::new(999).limit(), MAX_AGENT_CONCURRENCY);
        // 空闲时可用位 = 上限（保留位在构造时已取出）
        assert_eq!(ConcurrencyGate::new(3).available(), 3);
        assert_eq!(ConcurrencyGate::new(1).available(), 1);
    }

    /// 抬升上限立即生效：归还保留许可，可用位随之增加。
    #[tokio::test]
    async fn raising_limit_takes_effect_immediately() {
        let gate = ConcurrencyGate::new(1);
        assert_eq!(gate.available(), 1);

        assert_eq!(gate.set_limit(3), 3);
        assert_eq!(gate.available(), 3);

        let permit = gate.acquire().await.unwrap();
        assert_eq!(gate.available(), 2);
        drop(permit);
        assert_eq!(gate.available(), 3);
    }

    /// 空闲时降低上限立即生效（没有借出的执行位，保留许可同步借得到）。
    #[tokio::test]
    async fn lowering_limit_when_idle_takes_effect_immediately() {
        let gate = ConcurrencyGate::new(3);
        assert_eq!(gate.set_limit(1), 1);
        assert_eq!(gate.available(), 1);
    }

    /// 执行位全部借出时降低上限：信号量里没有可减的许可，只能等 run 收尾归还。
    ///
    /// 这是「设置页改成 1、实际仍并行 3 个」那个 bug 的回归测试：旧实现用
    /// `forget_permits` 减 0 个就结束了，这里必须在归还后收敛到新上限。
    #[tokio::test]
    async fn lowering_limit_takes_effect_after_borrowed_permits_return() {
        let gate = ConcurrencyGate::new(3);
        let p1 = gate.acquire().await.unwrap();
        let p2 = gate.acquire().await.unwrap();
        let p3 = gate.acquire().await.unwrap();
        assert_eq!(gate.available(), 0);

        assert_eq!(gate.set_limit(1), 1);
        assert_eq!(gate.limit(), 1);

        // run 收尾归还执行位（顺序无关：借位任务在队列里逐个补）
        drop(p1);
        drop(p2);
        drop(p3);
        // 借位任务是独立 task，需让出执行权直到收敛（不能靠固定 sleep 定长度）
        let mut left = 1000;
        while gate.available() != 1 && left > 0 {
            tokio::task::yield_now().await;
            left -= 1;
        }
        assert_eq!(
            gate.available(),
            1,
            "归还全部 3 个执行位后，闸门应只剩 1 个可用位（上限已降到 1）"
        );
    }

    /// 反复升降（含「降低后立刻抬回」）不漂移：借位任务是幂等的，它每拿到一个许可
    /// 都重读目标值，多占的会在下次调整时被回收。收敛后可用位应精确等于最终上限
    /// —— 少了会让提交白等（比设置窄），多了会突破上限（比设置宽）。
    #[tokio::test]
    async fn repeated_limit_changes_do_not_drift() {
        let gate = ConcurrencyGate::new(3);
        let p1 = gate.acquire().await.unwrap();
        let p2 = gate.acquire().await.unwrap();

        gate.set_limit(1);
        tokio::task::yield_now().await; // 让借位任务推进到「等归还」
        gate.set_limit(5); // 抬回：多占的保留位应被回收
        gate.set_limit(2);
        drop(p1);
        drop(p2);

        let mut left = 1000;
        while gate.available() != 2 && left > 0 {
            tokio::task::yield_now().await;
            left -= 1;
        }
        assert_eq!(gate.available(), 2, "反复升降后可用位应精确等于最终上限");
    }
}
