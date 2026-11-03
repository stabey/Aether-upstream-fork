import { beforeEach, describe, expect, it, vi } from 'vitest'

const adminApiMocks = vi.hoisted(() => ({
  queryProviderModels: vi.fn(),
  queryProviderModelsForKeys: vi.fn(),
}))

vi.mock('@/api/admin', () => ({ adminApi: adminApiMocks }))

import { useUpstreamModelsCache } from '../useUpstreamModelsCache'

function response(modelId: string) {
  return {
    success: true,
    data: { models: [{ id: modelId }] },
    provider: { id: 'provider-1', name: 'Provider', display_name: 'Provider' },
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

describe('useUpstreamModelsCache', () => {
  beforeEach(() => {
    adminApiMocks.queryProviderModels.mockReset()
    adminApiMocks.queryProviderModelsForKeys.mockReset()
  })

  it('deduplicates equivalent multi-key model requests', async () => {
    const request = deferred<ReturnType<typeof response>>()
    adminApiMocks.queryProviderModelsForKeys.mockReturnValue(request.promise)
    const { fetchModelsForKeys } = useUpstreamModelsCache()

    const first = fetchModelsForKeys('provider-1', ['key-b', 'key-a', 'key-a'])
    const second = fetchModelsForKeys('provider-1', ['key-a', 'key-b'])
    expect(adminApiMocks.queryProviderModelsForKeys).toHaveBeenCalledTimes(1)
    expect(adminApiMocks.queryProviderModelsForKeys).toHaveBeenCalledWith(
      'provider-1',
      ['key-a', 'key-b'],
      false,
    )

    request.resolve(response('gpt-5.6-sol'))
    await expect(first).resolves.toMatchObject({ models: [{ id: 'gpt-5.6-sol' }] })
    await expect(second).resolves.toMatchObject({ models: [{ id: 'gpt-5.6-sol' }] })
  })

  it('keeps the loading state owned by the latest forced request', async () => {
    const firstRequest = deferred<ReturnType<typeof response>>()
    const forcedRequest = deferred<ReturnType<typeof response>>()
    adminApiMocks.queryProviderModels
      .mockReturnValueOnce(firstRequest.promise)
      .mockReturnValueOnce(forcedRequest.promise)
    const { fetchModels, isLoading } = useUpstreamModelsCache()

    const first = fetchModels('provider-1', 'key-a')
    const forced = fetchModels('provider-1', 'key-a', true)
    expect(isLoading('provider-1', 'key-a')).toBe(true)

    firstRequest.resolve(response('gpt-5.6-sol'))
    await first
    expect(isLoading('provider-1', 'key-a')).toBe(true)

    forcedRequest.resolve(response('gpt-5.6-luna'))
    await forced
    expect(isLoading('provider-1', 'key-a')).toBe(false)
  })
  it('keeps permission sync separate from reads and forwards persisted permissions', async () => {
    const read = deferred<ReturnType<typeof response>>()
    adminApiMocks.queryProviderModels.mockReturnValueOnce(read.promise).mockResolvedValueOnce({
      ...response('ultra-model'),
      data: { models: [{ id: 'ultra-model' }], synced_allowed_models: ['ultra-model', 'locked-model'] },
    })
    const { fetchModels } = useUpstreamModelsCache()
    const pendingRead = fetchModels('provider-1', 'ultra-key')
    const synced = await fetchModels('provider-1', 'ultra-key', true, true)
    expect(adminApiMocks.queryProviderModels).toHaveBeenLastCalledWith('provider-1', 'ultra-key', true, true)
    expect(synced.syncedAllowedModels).toEqual(['ultra-model', 'locked-model'])
    read.resolve(response('old-model'))
    await expect(pendingRead).resolves.toMatchObject({ models: [{ id: 'old-model' }] })
  })

  it('does not merge model catalogs across credential scopes', async () => {
    adminApiMocks.queryProviderModels.mockImplementation(async (_provider: string, key: string) => response(key))
    const { fetchModels } = useUpstreamModelsCache()
    const [first, second] = await Promise.all([
      fetchModels('provider-1', 'key-free'), fetchModels('provider-1', 'key-ultra'),
    ])
    expect(first.models.map(model => model.id)).toEqual(['key-free'])
    expect(second.models.map(model => model.id)).toEqual(['key-ultra'])
  })

})
