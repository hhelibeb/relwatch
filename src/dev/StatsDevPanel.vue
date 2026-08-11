<script setup lang="ts">
// ── 开发者统计面板（诊断用）────────────────────────────
// 仅开发模式（import.meta.env.DEV）下通过 Ctrl+Shift+U 呼出，生产构建永不加载。
// 不进入任何用户 UI；事件名映射内置在此组件，不占用产品 i18n。
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { confirm } from '@tauri-apps/plugin-dialog'
import { getUsageStats, clearUsageStats, type UsageStatRow } from '../api/usage'
import { flushUsageTrackingNow } from '../composables/useUsageTracking'
import { registerOverlayActive } from '../composables/contextMenuBus'

const emit = defineEmits<{ close: [] }>()

const rows = ref<UsageStatRow[]>([])
const loading = ref(false)
const error = ref('')

let unregisterOverlay: (() => void) | null = null
const TREND_DAYS = 7

const totalClicks = computed(() => rows.value.reduce((sum, r) => sum + r.total_count, 0))
const maxCount = computed(() => Math.max(1, ...rows.value.map(r => r.total_count)))

// 事件 key → 开发者可读名称（无映射时兜底显示原始 key）
const EVENT_LABELS: Record<string, string> = {
  'app.check_now': '立即检查',
  'source.add': '添加源',
  'source.switch_search': '切换到搜索',
  'source.switch_add': '切换到添加',
  'source.open_url': '打开源主页',
  'source.view_releases': '查看源版本',
  'source.pending': '待更新跳转',
  'source.check': '单源检查',
  'source.pause': '暂停源',
  'source.resume': '恢复源',
  'source.mute': '静默源',
  'source.unmute': '取消静默',
  'source.remove': '删除源',
  'source.select': '进入选择模式',
  'source.bulk_select_all': '批量全选',
  'source.bulk_clear': '批量清空选择',
  'source.bulk_resume': '批量恢复',
  'source.bulk_pause': '批量暂停',
  'source.bulk_mute': '批量静默',
  'source.bulk_unmute': '批量取消静默',
  'source.bulk_delete': '批量删除',
  'source.sort': '源排序',
  'source.yt_subscribe': 'YT 订阅配置',
  'release.view_simple': '视图-简单列表',
  'release.view_aggregated': '视图-聚合',
  'release.view_calendar': '视图-日历',
  'release.filter_status': '状态筛选',
  'release.filter_importance': '重要度筛选',
  'release.open': '打开版本链接',
  'release.snooze': '稍后提醒',
  'release.ignore': '忽略版本',
  'release.read_full': '阅读全文',
  'release.translate': 'AI 翻译',
  'release.delete': '删除版本',
  'release.copy': '复制（链接/内容）',
  'release.detail_prev': '详情-上一个',
  'release.detail_next': '详情-下一个',
  'release.detail_close': '关闭详情',
  'release.detail_mode': '详情-内容切换',
  'calendar.prev_month': '日历-上月',
  'calendar.next_month': '日历-下月',
  'calendar.select_date': '日历-选日',
  'calendar.back': '日历-返回',
  'aggregated.expand_all': '聚合-展开/收起全部',
  'aggregated.toggle_repo': '聚合-展开/收起仓库',
  'log.clear': '清空日志',
  'log.filter': '日志级别筛选',
  'settings.save': '保存设置',
  'settings.discard': '放弃修改',
  'settings.export': '导出备份',
  'settings.import': '导入备份',
  'settings.bili_login': 'B 站一键登录',
  'settings.bili_clear': '清除 B 站 Cookie',
  'settings.test_ai': '测试 AI 连接',
  'settings.theme': '主题选择',
  'settings.lang': '语言选择',
}

function eventLabel(key: string): string {
  return EVENT_LABELS[key] ?? key
}

/** 最近 N 天的本地日期 key（YYYY-MM-DD），与后端分桶口径一致。 */
function recentDayKeys(n: number): string[] {
  const days: string[] = []
  for (let i = n - 1; i >= 0; i--) {
    const d = new Date()
    d.setDate(d.getDate() - i)
    days.push(
      `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`,
    )
  }
  return days
}

const trendDays = recentDayKeys(TREND_DAYS)

function dayCount(row: UsageStatRow, day: string): number {
  return row.daily.find(d => d.day === day)?.count ?? 0
}

function barWidth(count: number): string {
  return `${Math.round((count / maxCount.value) * 100)}%`
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    // 先冲刷前端内存计数，保证面板数据完整
    await flushUsageTrackingNow()
    rows.value = await getUsageStats()
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

async function handleClear() {
  const ok = await confirm('确定清空全部使用统计？此操作不可恢复。', {
    title: '清空使用统计',
    kind: 'warning',
  })
  if (!ok) return
  await clearUsageStats()
  await load()
}

onMounted(() => {
  window.addEventListener('keydown', handleKeydown)
  void load()
  // 面板挂载即视为覆盖层打开：面板内 Esc 不应最小化到托盘（供 useEscapeToTray 判定）
  unregisterOverlay = registerOverlayActive(() => true)
})
onUnmounted(() => {
  window.removeEventListener('keydown', handleKeydown)
  unregisterOverlay?.()
})

// Esc 关闭面板；覆盖层活跃状态由 contextMenuBus 注册表维护，面板打开时 Esc 不会冒泡成最小化到托盘。
function handleKeydown(e: KeyboardEvent) {
  if (e.defaultPrevented) return
  if (e.key === 'Escape') emit('close')
}
</script>

<template>
  <div class="stats-dev-overlay" @click.self="emit('close')">
    <div class="stats-dev-panel">
      <div class="stats-dev-header">
        <h2>功能使用统计（开发者）</h2>
        <div class="stats-dev-actions">
          <button class="btn-sm" @click="load">刷新</button>
          <button class="btn-sm btn-danger" @click="handleClear">清空</button>
          <button class="btn-sm" @click="emit('close')">关闭 (Esc)</button>
        </div>
      </div>
      <div v-if="error" class="stats-dev-error">{{ error }}</div>
      <div v-if="loading" class="stats-dev-empty">加载中...</div>
      <div v-else-if="rows.length === 0" class="stats-dev-empty">暂无统计数据</div>
      <template v-else>
        <div class="stats-dev-summary">
          <span>总点击：<b>{{ totalClicks }}</b></span>
          <span>事件种类：<b>{{ rows.length }}</b></span>
          <span>统计窗口：全部</span>
        </div>
        <div class="stats-dev-table">
          <div class="stats-dev-row stats-dev-head">
            <span class="col-key">功能</span>
            <span class="col-bar">占比</span>
            <span class="col-count">次数</span>
            <span class="col-trend">近 {{ TREND_DAYS }} 天</span>
            <span class="col-last">最近点击</span>
          </div>
          <div v-for="row in rows" :key="row.key" class="stats-dev-row">
            <span class="col-key" :title="row.key">{{ eventLabel(row.key) }}</span>
            <span class="col-bar"><span class="bar-track"><span class="bar-fill" :style="{ width: barWidth(row.total_count) }"></span></span></span>
            <span class="col-count">{{ row.total_count }}</span>
            <span class="col-trend">
              <span
                v-for="d in trendDays"
                :key="d"
                class="trend-cell"
                :class="{ active: dayCount(row, d) > 0 }"
                :style="{ opacity: dayCount(row, d) > 0 ? Math.min(1, 0.25 + dayCount(row, d) / maxCount) : 0.15 }"
                :title="`${d}: ${dayCount(row, d)}`"
              ></span>
            </span>
            <span class="col-last">{{ row.last_day }}</span>
          </div>
        </div>
        <p class="stats-dev-tip">提示：Ctrl+Shift+U 开关此面板；统计数据存于本机 SQLite（usage_stats），随备份导出/恢复。</p>
      </template>
    </div>
  </div>
</template>

<style scoped>
.stats-dev-overlay {
  position: fixed;
  inset: 0;
  z-index: 20000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.5);
  padding: 32px;
}

.stats-dev-panel {
  display: flex;
  flex-direction: column;
  width: min(820px, 100%);
  max-height: calc(100vh - 64px);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
}

.stats-dev-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px;
  border-bottom: 1px solid var(--border);
}

.stats-dev-header h2 {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
}

.stats-dev-actions {
  display: flex;
  gap: 6px;
}

.stats-dev-error {
  padding: 12px 16px;
  color: var(--danger);
  font-size: 13px;
}

.stats-dev-empty {
  padding: 40px 16px;
  text-align: center;
  color: var(--text-muted);
  font-size: 13px;
}

.stats-dev-summary {
  display: flex;
  gap: 20px;
  padding: 12px 16px;
  font-size: 13px;
  color: var(--text-muted);
  border-bottom: 1px solid var(--border);
}

.stats-dev-summary b {
  color: var(--text);
}

.stats-dev-table {
  overflow-y: auto;
  flex: 1;
}

.stats-dev-row {
  display: grid;
  grid-template-columns: minmax(140px, 1.4fr) 2fr 64px 110px 90px;
  gap: 10px;
  align-items: center;
  padding: 6px 16px;
  font-size: 12px;
  border-bottom: 1px solid var(--border);
}

.stats-dev-head {
  position: sticky;
  top: 0;
  background: var(--surface);
  color: var(--text-muted);
  font-weight: 600;
  z-index: 1;
}

.col-key {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text);
}

.col-count {
  text-align: right;
  font-variant-numeric: tabular-nums;
  font-weight: 600;
}

.col-last {
  color: var(--text-muted);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.bar-track {
  display: block;
  height: 8px;
  background: var(--bg-subtle);
  border-radius: 4px;
  overflow: hidden;
}

.bar-fill {
  display: block;
  height: 100%;
  background: var(--primary);
  border-radius: 4px;
  min-width: 2px;
}

.col-trend {
  display: flex;
  gap: 3px;
  align-items: flex-end;
  height: 18px;
}

.trend-cell {
  width: 9px;
  height: 100%;
  border-radius: 2px;
  background: var(--primary);
}

.stats-dev-tip {
  margin: 0;
  padding: 10px 16px;
  font-size: 11px;
  color: var(--text-muted);
  border-top: 1px solid var(--border);
}
</style>
