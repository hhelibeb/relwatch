<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick } from 'vue'
import { registerOverlayActive } from '../../composables/contextMenuBus'

export interface ContextMenuItem {
  id: string
  label: string
  /** 前缀图标 href（如 '/icons.svg#flag-tag-icon'），由调用方指定，通用组件不耦合业务图标 */
  iconHref?: string
  /** 前缀图标颜色（配合 iconHref，经 currentColor 注入） */
  color?: string
  /** 右侧选中 ✓（旗标当前色等单选语义） */
  checked?: boolean
  /** 渲染为分隔线（label/color 被忽略） */
  divider?: boolean
}

const props = defineProps<{
  x: number
  y: number
  items: ContextMenuItem[]
}>()
const emit = defineEmits<{
  action: [id: string]
  close: []
}>()

const menuRef = ref<HTMLElement | null>(null)
// 视口钳位后的实际坐标，避免菜单在窗口右/下边缘溢出视口
const adjustedX = ref(props.x)
const adjustedY = ref(props.y)

let unregisterOverlay: (() => void) | null = null

// 打开后钳位到视口内并自动聚焦第一个菜单项；
// 同时向全局覆盖层总线注册（组件挂载即视为覆盖层打开，供 useEscapeToTray 判定 Esc 拦截）
onMounted(async () => {
  unregisterOverlay = registerOverlayActive(() => true)
  await nextTick()
  const el = menuRef.value
  if (el) {
    const rect = el.getBoundingClientRect()
    let x = props.x
    let y = props.y
    if (x + rect.width > window.innerWidth - 4) {
      x = Math.max(4, window.innerWidth - rect.width - 4)
    }
    if (y + rect.height > window.innerHeight - 4) {
      y = Math.max(4, window.innerHeight - rect.height - 4)
    }
    adjustedX.value = x
    adjustedY.value = y
  }
  const firstButton = menuRef.value?.querySelector('button') as HTMLButtonElement | null
  firstButton?.focus()
})

onUnmounted(() => {
  unregisterOverlay?.()
  unregisterOverlay = null
})

// 键盘导航
function handleKeydown(e: KeyboardEvent) {
  const buttons = Array.from(menuRef.value?.querySelectorAll('button') || []) as HTMLButtonElement[]
  const currentIndex = buttons.findIndex(btn => btn === document.activeElement)
  
  switch (e.key) {
    case 'ArrowDown': {
      e.preventDefault()
      const nextIndex = currentIndex < buttons.length - 1 ? currentIndex + 1 : 0
      buttons[nextIndex]?.focus()
      break
    }
    case 'ArrowUp': {
      e.preventDefault()
      const prevIndex = currentIndex > 0 ? currentIndex - 1 : buttons.length - 1
      buttons[prevIndex]?.focus()
      break
    }
    case 'Escape':
      e.preventDefault()
      emit('close')
      break
    case 'Enter':
    case ' ':
      // 让默认的 click 事件处理
      break
  }
}
</script>

<template>
  <div ref="menuRef" class="context-menu" role="menu" tabindex="-1" :style="{ left: adjustedX + 'px', top: adjustedY + 'px' }" @click.stop @keydown="handleKeydown">
    <template v-if="items">
      <template v-for="item in items" :key="item.id">
        <div v-if="item.divider" class="context-menu-divider" role="separator"></div>
        <button v-else role="menuitem" tabindex="0" @click="$emit('action', item.id)">
          <span v-if="item.iconHref" class="context-menu-icon" :style="item.color ? { color: item.color } : undefined"><svg><use :href="item.iconHref"/></svg></span>
          <span class="context-menu-label">{{ item.label }}</span>
          <span v-if="item.checked" class="context-menu-check">✓</span>
        </button>
      </template>
    </template>
  </div>
</template>
<style scoped>
.context-menu {
  position: fixed;
  z-index: 10000;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 4px;
  box-shadow: var(--shadow-lg);
  display: flex;
  flex-direction: column;
  min-width: 120px;
}
.context-menu button {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border: none;
  background: transparent;
  color: var(--text);
  font-size: 13px;
  cursor: pointer;
  text-align: left;
  border-radius: 4px;
}
.context-menu button:hover {
  background: var(--bg-subtle);
}
.context-menu-icon {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}
.context-menu-icon svg {
  width: 12px;
  height: 12px;
}
.context-menu-check {
  margin-left: auto;
  padding-left: 12px;
  color: var(--primary);
}
.context-menu-divider {
  height: 1px;
  margin: 3px 6px;
  background: var(--border);
}
</style>
