<script setup lang="ts">
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
defineEmits<{
  open: []
  copy: []
  action: [id: string]
}>()
</script>

<template>
  <div class="context-menu" :style="{ left: x + 'px', top: y + 'px' }" @click.stop>
    <template v-if="items">
      <button v-for="item in items" :key="item.id" @click="$emit('action', item.id)">
        {{ item.label }}
      </button>
    </template>
    <template v-else>
      <button @click="$emit('open')">{{ t('context.open') }}</button>
      <button @click="$emit('copy')">{{ t('context.copy_link') }}</button>
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
