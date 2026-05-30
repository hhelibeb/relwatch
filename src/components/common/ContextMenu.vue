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
