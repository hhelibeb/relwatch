import { ref, watch, nextTick, onUnmounted } from 'vue'

/**
 * 悬停预览下拉（语言/主题等小型选择器）的完整交互状态机：
 * 打开/关闭、外点关闭、键盘导航（方向键移动+预览、Enter/Space 选中、
 * Esc 取消、Home/End 首尾）、焦点管理、监听器生命周期。
 *
 * 预览与选中行为通过回调注入：`preview(value)` 在悬停/键盘移动时生效
 * （如语言切换 setLocale、主题应用 applyTheme），`restore()` 在退出预览时
 * 恢复已保存值，`onSelect(value)` 在确认选中时落地（写 form + track）。
 *
 * 收敛了 SettingsTab 中语言/主题两份逐段镜像的复制粘贴实现。
 */
export function usePreviewSelect(opts: {
  preview: (value: string) => void
  restore: () => void
  onSelect: (value: string) => void
}) {
  const dropdownOpen = ref(false)
  const previewValue = ref<string | null>(null)
  const selectRef = ref<HTMLElement | null>(null)
  let outsideClickHandler: ((e: MouseEvent) => void) | null = null

  function clearPreview() {
    previewValue.value = null
    opts.restore()
  }

  function setPreview(val: string) {
    previewValue.value = val
    opts.preview(val)
  }

  function handleOutsideClick(e: MouseEvent) {
    if (selectRef.value && !selectRef.value.contains(e.target as Node)) {
      dropdownOpen.value = false
      clearPreview()
    }
  }

  watch(dropdownOpen, (isOpen) => {
    if (isOpen) {
      nextTick(() => {
        // 守卫：若下拉在 nextTick 执行前已被快速关闭（如同一微任务内再次 toggle），
        // 不应再向 document 注册 outsideClick 监听器，避免监听器泄漏
        if (!dropdownOpen.value) return
        outsideClickHandler = handleOutsideClick
        document.addEventListener('click', outsideClickHandler)
      })
    } else {
      if (outsideClickHandler) {
        document.removeEventListener('click', outsideClickHandler)
        outsideClickHandler = null
      }
    }
  })

  onUnmounted(() => {
    if (outsideClickHandler) {
      document.removeEventListener('click', outsideClickHandler)
      outsideClickHandler = null
    }
  })

  /** 确认选中：落地 onSelect 后关闭下拉，焦点移回触发器。 */
  function select(val: string) {
    opts.onSelect(val)
    previewValue.value = null
    setTimeout(() => {
      dropdownOpen.value = false
      // 下拉关闭后把焦点移回触发器，避免 v-if 移除聚焦选项后焦点回退到 body
      nextTick(() => {
        const trigger = selectRef.value?.querySelector('.theme-select-trigger') as HTMLElement | null
        trigger?.focus()
      })
    }, 0)
  }

  function toggle() {
    dropdownOpen.value = !dropdownOpen.value
    if (!dropdownOpen.value) {
      clearPreview()
    } else {
      // 打开时聚焦第一个选项
      nextTick(() => {
        const dropdown = selectRef.value?.querySelector('.theme-select-dropdown')
        const firstOption = dropdown?.querySelector('.theme-select-option') as HTMLElement | null
        firstOption?.focus()
      })
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    // 当下拉关闭时，只处理打开操作
    if (!dropdownOpen.value) {
      if (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ') {
        e.preventDefault()
        toggle()
      }
      return
    }

    const dropdown = selectRef.value?.querySelector('.theme-select-dropdown') as HTMLElement | null
    if (!dropdown) return

    const options = Array.from(dropdown.querySelectorAll('.theme-select-option')) as HTMLElement[]
    const currentIndex = options.findIndex((opt) => opt === document.activeElement)

    switch (e.key) {
      case 'ArrowDown': {
        e.preventDefault()
        const nextIndex = currentIndex < options.length - 1 ? currentIndex + 1 : 0
        options[nextIndex]?.focus()
        const nextVal = options[nextIndex]?.getAttribute('data-value')
        if (nextVal) setPreview(nextVal)
        break
      }
      case 'ArrowUp': {
        e.preventDefault()
        const prevIndex = currentIndex > 0 ? currentIndex - 1 : options.length - 1
        options[prevIndex]?.focus()
        const prevVal = options[prevIndex]?.getAttribute('data-value')
        if (prevVal) setPreview(prevVal)
        break
      }
      case 'Enter':
      case ' ':
        e.preventDefault()
        if (currentIndex >= 0) {
          const option = options[currentIndex] as HTMLElement
          const value = option.getAttribute('data-value')
          if (value) select(value)
        }
        break
      case 'Escape': {
        e.preventDefault()
        dropdownOpen.value = false
        clearPreview()
        const trigger = selectRef.value?.querySelector('.theme-select-trigger') as HTMLElement | null
        trigger?.focus()
        break
      }
      case 'Home': {
        e.preventDefault()
        options[0]?.focus()
        const firstVal = options[0]?.getAttribute('data-value')
        if (firstVal) setPreview(firstVal)
        break
      }
      case 'End': {
        e.preventDefault()
        options[options.length - 1]?.focus()
        const lastVal = options[options.length - 1]?.getAttribute('data-value')
        if (lastVal) setPreview(lastVal)
        break
      }
    }
  }

  return {
    dropdownOpen,
    previewValue,
    selectRef,
    toggle,
    handleKeydown,
    clearPreview,
  }
}
