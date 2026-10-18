<script setup lang="ts">
/**
 * 空状态
 *
 * 「无数据」有三种成因，文案与出口各不相同：首次为空（引导添加）、搜索无结果、
 * 筛选无结果。调用方按成因给 title/description，并把出口放进 actions 插槽，
 * 本组件只负责同一套骨架（48px 描边图标 + 标题 + 说明）。样式在 style.css 的
 * `.empty*`，与骨架屏、错误块共用一份全局定义。
 */
withDefaults(defineProps<{
  title: string
  description?: string
  /** 装饰图标：48px。紧凑场景（弹窗内、容器空位）由 compact 一并关掉。 */
  icon?: boolean
  /** 紧凑档：收窄留白并给一层浅底，避免在小容器里顶出视区。 */
  compact?: boolean
}>(), {
  description: '',
  icon: true,
  compact: false,
})
</script>

<template>
  <div class="empty" :class="{ 'empty-compact': compact }">
    <span v-if="icon && !compact" class="empty-icon" aria-hidden="true"></span>
    <p class="empty-title">{{ title }}</p>
    <p v-if="description" class="empty-desc">{{ description }}</p>
    <div v-if="$slots.actions" class="empty-actions">
      <slot name="actions" />
    </div>
  </div>
</template>
