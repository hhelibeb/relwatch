<script setup lang="ts">
import { ref, onMounted, nextTick } from 'vue'
import { t } from '../../i18n'

export interface ContextMenuItem {
  id: string
  label: string
}

defineProps<{
  x: number
  y: number
  items?: ContextMenuItem[]
}>()
const emit = defineEmits<{
  open: []
  copy: []
  action: [id: string]
  close: []
}>()

const menuRef = ref<HTMLElement | null>(null)

// 打开后自动聚焦第一个菜单项
onMounted(async () => {
  await nextTick()
  const firstButton = menuRef.value?.querySelector('button') as HTMLButtonElement | null
  firstButton?.focus()
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
  <div ref="menuRef" class="context-menu" role="menu" tabindex="-1" :style="{ left: x + 'px', top: y + 'px' }" @click.stop @keydown="handleKeydown">
    <template v-if="items">
      <button v-for="item in items" :key="item.id" role="menuitem" tabindex="0" @click="$emit('action', item.id)">
        {{ item.label }}
      </button>
    </template>
    <template v-else>
      <button role="menuitem" tabindex="0" @click="$emit('open')">{{ t('context.open') }}</button>
      <button role="menuitem" tabindex="0" @click="$emit('copy')">{{ t('context.copy_link') }}</button>
    </template>
  </div>
</template>
<style scoped>
.context-menu {
  position: fixed;
  z-index: 10000;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 4px;
  box-shadow: 0 4px 16px rgba(0,0,0,0.12);
  display: flex;
  flex-direction: column;
  min-width: 120px;
}
.context-menu button {
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
  background: var(--bg);
}

:global([data-theme="dark"] .context-menu) {
  box-shadow: 0 6px 20px rgba(0,0,0,0.4);
}
</style>
