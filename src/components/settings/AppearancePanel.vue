<script setup lang="ts">
import { computed, ref } from 'vue'
import type { AppSettings } from '../../api/settings'
import { t, setLocale, languages } from '../../i18n'
import { applyTheme } from '../../composables/useTheme'
import { applyFontScale } from '../../composables/useFontScale'
import { track } from '../../composables/useUsageTracking'
import BaseSelect from '../ui/BaseSelect.vue'

const form = defineModel<AppSettings>('form', { required: true })
defineProps<{
  dirtyFields: Set<string>
}>()

const themeSelectRef = ref<InstanceType<typeof BaseSelect> | null>(null)
const fontScaleValues = [90, 100, 110, 125, 150]

const languageOptions = computed(() =>
  languages.map((lang) => ({ value: lang.value, label: lang.label })),
)

const themeOptions = computed(() => [
  { value: 'system', label: t('settings.theme_system') },
  { value: 'light', label: t('settings.theme_light') },
  { value: 'dark', label: t('settings.theme_dark') },
])

const fontScaleOptions = computed(() =>
  fontScaleValues.map((v) => ({ value: String(v), label: t(`settings.font_scale_${v}`) })),
)

const normalizedFontScale = computed(() =>
  String(fontScaleValues.includes(form.value.font_scale) ? form.value.font_scale : 100),
)

function onSelectLang(val: string) {
  form.value.language = val
  track('settings.lang')
  setLocale(val)
}

function onSelectTheme(val: string) {
  form.value.theme = val
  track('settings.theme')
  applyTheme(val)
}

function onSelectFontScale(val: string) {
  form.value.font_scale = Number(val)
  track('settings.font_scale')
  applyFontScale(form.value.font_scale)
}

function clearThemePreview() {
  themeSelectRef.value?.clearPreview()
}

defineExpose({ clearThemePreview })
</script>

<template>
  <div class="settings-form">
    <div class="setting-row">
      <span class="setting-label" :data-dirty="dirtyFields.has('language') || null">
        {{ t('settings.language') }}<svg class="label-icon"><use href="/icons.svg#language-icon" /></svg>
      </span>
      <BaseSelect
        :model-value="form.language"
        :options="languageOptions"
        preview-on-hover
        @preview="setLocale($event)"
        @restore="setLocale(form.language)"
        @select="onSelectLang"
      />
    </div>
    <div class="setting-row">
      <span class="setting-label" :data-dirty="dirtyFields.has('theme') || null">{{ t('settings.theme') }}</span>
      <BaseSelect
        ref="themeSelectRef"
        :model-value="form.theme"
        :options="themeOptions"
        preview-on-hover
        @preview="applyTheme($event)"
        @restore="applyTheme(form.theme)"
        @select="onSelectTheme"
      />
    </div>
    <div class="setting-row">
      <span class="setting-label" :data-dirty="dirtyFields.has('font_scale') || null">{{ t('settings.font_scale') }}</span>
      <BaseSelect
        :model-value="normalizedFontScale"
        :options="fontScaleOptions"
        @select="onSelectFontScale"
      />
    </div>
    <label class="setting-row setting-row-checkbox">
      <input type="checkbox" v-model="form.minimize_to_tray" />
      <span class="setting-label" :data-dirty="dirtyFields.has('minimize_to_tray') || null">{{ t('settings.minimize_tray') }}</span>
    </label>
    <label class="setting-row setting-row-checkbox">
      <input type="checkbox" v-model="form.show_source_type_icons" />
      <span class="setting-label" :data-dirty="dirtyFields.has('show_source_type_icons') || null">{{ t('settings.show_source_type_icons') }}</span>
    </label>
  </div>
</template>
