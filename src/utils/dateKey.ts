export function toDateKey(date: Date): string {
  const y = date.getFullYear()
  const m = String(date.getMonth() + 1).padStart(2, '0')
  const d = String(date.getDate()).padStart(2, '0')
  return `${y}-${m}-${d}`
}

export function parseDateKey(key: string): Date {
  const [y, m, d] = key.split('-').map(Number)
  return new Date(y, m - 1, d)
}

export const CALENDAR_MIN_YEAR = 2010

export function calendarMonthAhead(year: number, month: number): boolean {
  const now = new Date()
  return year > now.getFullYear() || (year === now.getFullYear() && month > now.getMonth() + 1)
}

export function clampCalendarMonth(year: number, month: number): { year: number; month: number } {
  const total = year * 12 + (month - 1)
  const now = new Date()
  const min = CALENDAR_MIN_YEAR * 12
  const max = now.getFullYear() * 12 + now.getMonth()
  const clamped = Math.min(Math.max(total, min), max)
  return { year: Math.floor(clamped / 12), month: (clamped % 12) + 1 }
}
