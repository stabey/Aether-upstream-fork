import { createApp, h, nextTick, ref, type App } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createI18n } from '@/i18n'
import Dialog from '@/components/ui/dialog/Dialog.vue'
import MultiSelect, { type MultiSelectOption } from '../MultiSelect.vue'

const OPTIONS: MultiSelectOption[] = [
  { value: 'unspecified', label: 'Default (Unspecified)' },
  { value: 'none', label: 'None' },
  { value: 'minimal', label: 'Minimal' },
  { value: 'low', label: 'Low' },
  { value: 'medium', label: 'Medium' },
  { value: 'high', label: 'High' },
  { value: 'xhigh', label: 'XHigh' },
]

const mounted: Array<{ app: App; root: HTMLElement }> = []

afterEach(() => {
  vi.restoreAllMocks()
  for (const { app, root } of mounted.splice(0)) {
    app.unmount()
    root.remove()
  }
})

describe('MultiSelect', () => {
  it('teleports dropdown and flips upward when opened near the bottom of the viewport without a scroll-blocking backdrop', async () => {
    vi.spyOn(window, 'innerHeight', 'get').mockReturnValue(780)
    vi.spyOn(window, 'innerWidth', 'get').mockReturnValue(480)

    const selected = ref<string[]>([])
    const root = document.createElement('div')
    document.body.append(root)
    const app = createApp({
      setup() {
        return () =>
          h(MultiSelect, {
            modelValue: selected.value,
            options: OPTIONS,
            'onUpdate:modelValue': (val: string[]) => {
              selected.value = val
            },
          })
      },
    })
    app.use(createI18n())
    app.mount(root)
    mounted.push({ app, root })

    const trigger = root.querySelector('button')!
    vi.spyOn(trigger, 'getBoundingClientRect').mockReturnValue({
      top: 650,
      bottom: 686,
      left: 24,
      right: 456,
      width: 432,
      height: 36,
      x: 24,
      y: 650,
      toJSON: () => ({}),
    })

    trigger.click()
    await nextTick()

    const dropdown = document.body.querySelector<HTMLElement>('[data-multiselect-dropdown]')
    expect(dropdown).not.toBeNull()
    expect(root.querySelector('[data-multiselect-dropdown]')).toBeNull()
    expect(document.body.querySelector('.fixed.inset-0')).toBeNull()
    expect(dropdown?.dataset.side).toBe('top')
    expect(dropdown?.style.bottom).toBe('134px')

    const optionRows = dropdown!.querySelectorAll<HTMLElement>('div.cursor-pointer')
    // index 0 is Select All row, index 1 is first option row
    optionRows[1]?.click()
    await nextTick()
    expect(selected.value).toEqual(['unspecified'])
    expect(document.body.querySelector('[data-multiselect-dropdown]')).not.toBeNull()

    document.body.click()
    await nextTick()
    expect(document.body.querySelector('[data-multiselect-dropdown]')).toBeNull()
  })

  it('closes only MultiSelect on Escape without dismissing an enclosing Dialog', async () => {
    const dialogOpen = ref(true)
    const selected = ref<string[]>([])
    const root = document.createElement('div')
    document.body.append(root)

    const app = createApp({
      setup() {
        return () =>
          h(
            Dialog,
            {
              open: dialogOpen.value,
              title: 'Add Model Mapping',
              'onUpdate:open': (val: boolean) => {
                dialogOpen.value = val
              },
            },
            {
              default: () =>
                h(MultiSelect, {
                  modelValue: selected.value,
                  options: OPTIONS,
                  'onUpdate:modelValue': (val: string[]) => {
                    selected.value = val
                  },
                }),
            },
          )
      },
    })
    app.use(createI18n())
    app.mount(root)
    mounted.push({ app, root })
    await nextTick()

    const trigger = document.body.querySelector<HTMLButtonElement>('button[aria-haspopup="listbox"]')!
    trigger.click()
    await nextTick()

    expect(document.body.querySelector('[data-multiselect-dropdown]')).not.toBeNull()
    expect(dialogOpen.value).toBe(true)

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await nextTick()

    expect(document.body.querySelector('[data-multiselect-dropdown]')).toBeNull()
    expect(dialogOpen.value).toBe(true)

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await nextTick()

    expect(dialogOpen.value).toBe(false)
  })
})
