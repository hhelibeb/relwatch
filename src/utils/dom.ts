const TEXT_INPUT_TYPES = new Set([
  'text',
  'search',
  'number',
  'password',
  'url',
  'email',
  'tel',
  '',
])

function isContentEditableElement(el: HTMLElement): boolean {
  // jsdom 未实现 isContentEditable（恒为 undefined），改判特性与 contentEditable 属性
  const attr = el.getAttribute('contenteditable')
  return attr === '' || attr === 'true' || attr === 'plaintext-only' || el.contentEditable === 'true'
}

export function isTextField(el: Element | null): el is HTMLElement {
  if (!(el instanceof HTMLElement)) return false
  const tag = el.tagName
  if (tag === 'TEXTAREA' || tag === 'SELECT') return true
  if (tag === 'INPUT') {
    return TEXT_INPUT_TYPES.has((el as HTMLInputElement).type.toLowerCase())
  }
  return isContentEditableElement(el)
}

export function isEditableTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false
  const tag = el.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true
  return isContentEditableElement(el)
}
