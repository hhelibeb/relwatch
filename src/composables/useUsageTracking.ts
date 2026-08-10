import { recordUsage } from '../api/usage'

// ── 功能按钮点击统计（诊断用途）────────────────────────
// 模块级单例：内存聚合 + 5s 节流批量写库，避免每次点击一次 IPC。
// 写入失败静默（统计丢失可接受，绝不影响功能）；开关关闭时 track() 直接 no-op。
// 生产代码不包含任何统计展示 UI——查看请用开发模式下的 StatsDevPanel（Ctrl+Shift+U）。

const FLUSH_INTERVAL_MS = 5000

const pending = new Map<string, number>()
let flushTimer: ReturnType<typeof setTimeout> | null = null
let enabled = true

/** 由 App.vue 依据设置项 enable_usage_stats 驱动；关闭时丢弃未上报计数。 */
export function setUsageTrackingEnabled(value: boolean): void {
  enabled = value
  if (!value) {
    pending.clear()
    if (flushTimer) {
      clearTimeout(flushTimer)
      flushTimer = null
    }
  }
}

/** 记录一次功能交互。key 遵循 `<域>.<动作>` 命名（如 'source.add'、'release.translate'）。 */
export function track(key: string): void {
  if (!enabled || !key) return
  pending.set(key, (pending.get(key) ?? 0) + 1)
  scheduleFlush()
}

function scheduleFlush(): void {
  if (flushTimer) return
  flushTimer = setTimeout(() => {
    flushTimer = null
    void flush()
  }, FLUSH_INTERVAL_MS)
}

async function flush(): Promise<void> {
  if (pending.size === 0) return
  const events: [string, number][] = [...pending.entries()]
  pending.clear()
  try {
    await recordUsage(events)
  } catch {
    // 静默失败：统计丢失仅影响诊断数据，不影响功能
  }
}

/** 立即冲刷待上报计数（App 卸载/关闭前兜底）。返回 Promise 供调用方等待。 */
export function flushUsageTrackingNow(): Promise<void> {
  if (flushTimer) {
    clearTimeout(flushTimer)
    flushTimer = null
  }
  return flush()
}
