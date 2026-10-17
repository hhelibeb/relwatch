<script setup lang="ts">
/**
 * 统一错误块
 *
 * 替代「整行红字」与「只弹 Toast」两种旧做法：原因留在原位（Toast 会消失、红字
 * 说不出下一步），右侧固定一个重试出口。错误文案原样透传，是否翻译由调用方决定。
 */
defineProps<{
  message: string
  retryLabel: string
}>()

defineEmits<{ retry: [] }>()
</script>

<template>
  <div class="error-block" role="alert">
    <span class="error-block-msg">{{ message }}</span>
    <button type="button" class="btn-sm error-block-retry" @click="$emit('retry')">{{ retryLabel }}</button>
  </div>
</template>
