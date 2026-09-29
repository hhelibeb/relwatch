/**
 * 主题应用单例：`dark` / `light` / `system`（跟随系统 prefers-color-scheme）。
 *
 * 主题判定只有这一份实现：App.vue 启动/重载与 SettingsTab 选中、预览、恢复都调这里。
 */
export function applyTheme(theme: string): void {
  if (theme === 'dark') {
    document.documentElement.dataset.theme = 'dark'
  } else if (theme === 'light') {
    document.documentElement.dataset.theme = 'light'
  } else {
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
    document.documentElement.dataset.theme = prefersDark ? 'dark' : 'light'
  }
}
