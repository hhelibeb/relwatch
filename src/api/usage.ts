import { invokeI18n } from './client'

/** 单日计数（按本地时区 YYYY-MM-DD 分桶）。 */
export interface UsageDaily {
  day: string
  count: number
}

/** 单个事件的聚合统计行（total_count = 累计次数，daily = 按天趋势）。 */
export interface UsageStatRow {
  key: string
  total_count: number
  last_day: string
  daily: UsageDaily[]
}

/** 批量上报点击计数（前端 5s 节流聚合后调用）。 */
export async function recordUsage(events: [string, number][]): Promise<void> {
  if (events.length === 0) return
  await invokeI18n('record_usage', { events })
}

/** 查询统计（按累计次数降序）。仅开发者工具调用，不进入任何用户 UI。 */
export async function getUsageStats(days?: number): Promise<UsageStatRow[]> {
  return invokeI18n<UsageStatRow[]>('get_usage_stats', days !== undefined ? { days } : undefined)
}

/** 清空全部统计。 */
export async function clearUsageStats(): Promise<void> {
  return invokeI18n('clear_usage_stats')
}
