import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, type App } from 'vue'

import KeyAllowedModelsEditDialog from '../KeyAllowedModelsEditDialog.vue'
import type { EndpointAPIKey } from '@/api/endpoints'

const endpointMocks = vi.hoisted(() => ({
  getProviderModels: vi.fn(),
  updateProviderKey: vi.fn(),
  fetchModels: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  confirmWarning: vi.fn(),
}))

vi.mock('@/api/endpoints', () => endpointMocks)
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({
    error: endpointMocks.error,
    success: endpointMocks.success,
    warning: vi.fn(),
  }),
}))
vi.mock('@/composables/useConfirm', () => ({
  useConfirm: () => ({
    confirmWarning: endpointMocks.confirmWarning,
  }),
}))
vi.mock('@/features/providers/composables/useUpstreamModelsCache', () => ({
  useUpstreamModelsCache: () => ({
    fetchModels: endpointMocks.fetchModels,
  }),
}))
vi.mock('@/components/ui', async () => {
  const { defineComponent, h } = await import('vue')
  const Dialog = defineComponent({
    name: 'DialogStub',
    setup: (_props, { slots }) => () => h('section', [slots.default?.(), slots.footer?.()]),
  })
  const passthrough = (name: string) => defineComponent({
    name,
    inheritAttrs: false,
    setup: (_props, { attrs, slots }) => () => h('div', attrs, slots.default?.()),
  })
  return {
    Dialog,
    Button: passthrough('ButtonStub'),
    Input: passthrough('InputStub'),
  }
})

const mountedApps: Array<{ app: App, root: HTMLElement }> = []

const apiKey: EndpointAPIKey = {
  id: 'key-1',
  provider_id: 'provider-1',
  api_formats: ['openai:chat'],
  api_key_masked: 'sk-***',
  auth_type: 'api_key',
  name: 'Primary key',
  internal_priority: 0,
  allowed_models: ['gpt-5'],
  cache_ttl_minutes: 0,
  max_probe_interval_minutes: 5,
  health_score: 1,
  consecutive_failures: 0,
  request_count: 0,
  success_count: 0,
  error_count: 0,
  success_rate: 1,
  avg_response_time_ms: 0,
  is_active: true,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
}

async function settle() {
  for (let index = 0; index < 5; index += 1) {
    await Promise.resolve()
    await nextTick()
  }
}

beforeEach(() => {
  endpointMocks.getProviderModels.mockReset()
  endpointMocks.getProviderModels.mockResolvedValue([{
    provider_model_name: 'gpt-5',
    global_model_name: 'gpt-5',
    global_model_display_name: 'GPT-5',
  }])
  endpointMocks.updateProviderKey.mockReset()
  endpointMocks.fetchModels.mockReset()
  endpointMocks.fetchModels.mockResolvedValue({ models: [{ id: 'gpt-5' }] })
  endpointMocks.success.mockReset()
  endpointMocks.error.mockReset()
  endpointMocks.confirmWarning.mockReset()
  endpointMocks.confirmWarning.mockResolvedValue(true)
})

afterEach(() => {
  for (const { app, root } of mountedApps.splice(0)) {
    app.unmount()
    root.remove()
  }
})

describe('KeyAllowedModelsEditDialog loading', () => {
  it('loads saved permissions when lazily mounted in the open state', async () => {
    const root = document.createElement('div')
    document.body.appendChild(root)
    const app = createApp(KeyAllowedModelsEditDialog, {
      open: true,
      apiKey,
      providerId: 'provider-1',
    })
    app.mount(root)
    mountedApps.push({ app, root })

    await settle()

    expect(endpointMocks.getProviderModels).toHaveBeenCalledOnce()
    expect(endpointMocks.getProviderModels).toHaveBeenCalledWith('provider-1', { limit: 1000 })
    expect(root.textContent).toContain('提供商模型')
    expect(root.textContent).toContain('已选 1 个')
    expect(root.querySelector('.bg-primary.border-primary')).toBeTruthy()
  })
})
describe('model permission refresh', () => {
  async function mountDialog(auto: boolean, onSynced = vi.fn()) {
    const root = document.createElement('div')
    document.body.appendChild(root)
    const app = createApp(KeyAllowedModelsEditDialog, {
      open: true, apiKey: { ...apiKey, auto_fetch_models: auto },
      providerId: 'provider-1', onSynced,
    })
    app.mount(root)
    mountedApps.push({ app, root })
    await settle()
    return root
  }

  it('explicitly syncs automatic permissions and reports the persisted whitelist', async () => {
    const synced = vi.fn()
    const root = await mountDialog(true, synced)
    expect(endpointMocks.fetchModels).toHaveBeenCalledWith('provider-1', 'key-1', false, false)
    endpointMocks.fetchModels.mockResolvedValueOnce({
      models: [{ id: 'gpt-5' }, { id: 'new-model' }],
      syncedAllowedModels: ['gpt-5', 'new-model'],
    })
    root.querySelector<HTMLButtonElement>('button[title]')!.click()
    await settle()
    expect(endpointMocks.fetchModels).toHaveBeenLastCalledWith('provider-1', 'key-1', true, true)
    expect(root.textContent).toContain('已选 2 个')
    expect(synced).toHaveBeenCalledWith(['gpt-5', 'new-model'])
    expect(endpointMocks.updateProviderKey).not.toHaveBeenCalled()
  })

  it('retains previous permissions and shows no success after a failed sync', async () => {
    const synced = vi.fn()
    const root = await mountDialog(true, synced)
    endpointMocks.fetchModels.mockResolvedValueOnce({ models: [], error: 'upstream failed' })
    root.querySelector<HTMLButtonElement>('button[title]')!.click()
    await settle()
    expect(root.textContent).toContain('已选 1 个')
    expect(synced).not.toHaveBeenCalled()
    expect(endpointMocks.error).toHaveBeenCalled()
    expect(endpointMocks.success).not.toHaveBeenCalled()
  })

  it('does not discard local automatic locks when discard is cancelled', async () => {
    const root = await mountDialog(true)
    const lock = root.querySelector<HTMLButtonElement>('button.p-1')!
    lock.click()
    await settle()
    endpointMocks.confirmWarning.mockResolvedValueOnce(false)
    root.querySelector<HTMLButtonElement>('button[title]')!.click()
    await settle()
    expect(endpointMocks.confirmWarning).toHaveBeenCalledOnce()
    expect(endpointMocks.fetchModels).toHaveBeenCalledTimes(1)
    expect(root.textContent).toContain('未保存')
  })

  it('preserves unsaved manual selections during a catalog refresh', async () => {
    const root = await mountDialog(false)
    const selected = root.querySelector<HTMLElement>('.bg-primary.border-primary')!
    selected.parentElement!.click()
    await settle()
    expect(root.textContent).toContain('全部模型')
    root.querySelector<HTMLButtonElement>('button[title]')!.click()
    await settle()
    expect(root.textContent).toContain('全部模型')
    expect(endpointMocks.confirmWarning).not.toHaveBeenCalled()
  })

  it('manual refresh reads the catalog without changing saved selections', async () => {
    const root = await mountDialog(false)
    endpointMocks.fetchModels.mockResolvedValueOnce({ models: [{ id: 'other-model' }] })
    root.querySelector<HTMLButtonElement>('button[title]')!.click()
    await settle()
    expect(endpointMocks.fetchModels).toHaveBeenCalledWith('provider-1', 'key-1', true, false)
    expect(root.textContent).toContain('已选 1 个')
    expect(endpointMocks.updateProviderKey).not.toHaveBeenCalled()
  })
})
