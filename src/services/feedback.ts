import {
  confirm,
  open,
  type ConfirmDialogOptions,
  type OpenDialogOptions,
} from '@tauri-apps/plugin-dialog'

export function extractErrorMessage(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e)
  return raw.replace(/^Error:\s*/, '')
}

export async function confirmAction(
  msg: string,
  options?: string | ConfirmDialogOptions,
): Promise<boolean> {
  return confirm(msg, options)
}

export async function openFileDialog(options?: OpenDialogOptions) {
  return open(options)
}
