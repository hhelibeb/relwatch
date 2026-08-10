/**
 * 主题应用单例：`dark` / `light` / `system`（跟随系统 prefers-color-scheme）。
 *
 * 收敛了原先 App.vue `applyTheme` 与 SettingsTab 主题预览/恢复（setThemePreview /
 * clearThemePreview）中逐段复制的同一分支逻辑——主题判定只有这一份实现。
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
