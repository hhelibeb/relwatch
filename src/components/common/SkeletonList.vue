<script setup lang="ts">
/**
 * 骨架屏
 *
 * 只在「列表为空 + 加载中」时渲染：已有数据刷新时不重放，避免翻页/轮询把整屏闪成灰条。
 * 出现时机由 style.css 的 `skeleton-in ... 100ms both` 控制（本地请求常在此之内返回，
 * 早于 100ms 白屏比闪一帧骨架更顺）。
 */
withDefaults(defineProps<{
  /** 骨架条数 */
  rows?: number
  /** row：列表行（头像位 + 两行文本）；card：版本卡（两行文本 + 更宽留白） */
  variant?: 'row' | 'card'
  /** 无障碍名：role=status 需要可读文本，骨架本身无可读内容 */
  label: string
}>(), {
  rows: 3,
  variant: 'row',
})
</script>

<template>
  <div class="skeleton" :class="`skeleton-${variant}`" role="status" aria-busy="true" :aria-label="label">
    <div v-for="i in rows" :key="i" class="skeleton-item">
      <span v-if="variant === 'row'" class="skeleton-avatar"></span>
      <span class="skeleton-lines">
        <span class="skeleton-line skeleton-line-lg"></span>
        <span class="skeleton-line skeleton-line-sm"></span>
      </span>
    </div>
  </div>
</template>
