<script setup lang="ts">
import { t } from '../../i18n'

defineProps<{ x: number; y: number }>()
defineEmits<{ open: []; copy: [] }>()
</script>

<template>
  <div class="context-menu" :style="{ left: x + 'px', top: y + 'px' }" @click.stop>
    <button @click="$emit('open')">{{ t('context.open') }}</button>
    <button @click="$emit('copy')">{{ t('context.copy_link') }}</button>
  </div>
</template>
