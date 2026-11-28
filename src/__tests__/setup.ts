import { afterEach, beforeEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { config } from '@vue/test-utils'
import { closeAllContextMenus, endTransientUiInvalidation } from '../composables/contextMenuBus'

beforeEach(() => {
  const pinia = createPinia()
  setActivePinia(pinia)
  config.global.plugins = [pinia]
})

afterEach(() => {
  closeAllContextMenus()
  endTransientUiInvalidation()
})
