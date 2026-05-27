import { invokeI18n } from './client'

export interface LogEntry {
  id: number
  level: string
  message: string
  created_at: string
  message_key: string | null
  message_args: string | null
}

export interface LogSearchResult {
  entries: LogEntry[]
  total: number
  page: number
  page_size: number
}

export async function getLogs(limit: number): Promise<LogEntry[]> {
  return invokeI18n<LogEntry[]>('get_logs', { limit })
}

export async function searchLogs(keyword: string, page: number, pageSize: number): Promise<LogSearchResult> {
  return invokeI18n<LogSearchResult>('search_logs', { keyword, page, pageSize })
}

export async function clearLogs(): Promise<void> {
  return invokeI18n('clear_logs')
}
