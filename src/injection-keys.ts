import type { InjectionKey } from 'vue'

export const ShowToastKey: InjectionKey<(msg: string) => void> = Symbol('showToast')
