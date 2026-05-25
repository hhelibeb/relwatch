import { invokeI18n } from './client'

export interface LogEntry {
  id: number
  level: string
  message: string
  created_at: string
  message_key: string | null
  message_args: string | null
}

export async function getLogs(limit: number): Promise<LogEntry[]> {
  return invokeI18n<LogEntry[]>('get_logs', { limit })
}

export async function clearLogs(): Promise<void> {
  return invokeI18n('clear_logs')
}
