<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  name: string
  size?: number
}>()

const iconStyle = computed(() =>
  props.size ? { width: `${props.size}px`, height: `${props.size}px` } : undefined,
)
</script>

<template>
  <svg class="app-icon" :style="iconStyle" aria-hidden="true">
    <use :href="`/icons.svg#${name}`" />
  </svg>
</template>

<style scoped>
.app-icon {
  display: inline-block;
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  vertical-align: middle;
}
</style>
