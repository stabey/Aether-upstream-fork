import { computed, ref, watch, type ComputedRef, type Ref } from 'vue'

type ActiveElapsedStatus = string | null | undefined

export interface UseActiveElapsedDisplayClockOptions<TRecord> {
  records: Ref<TRecord[]> | ComputedRef<TRecord[]>
  isPageVisible: Ref<boolean> | ComputedRef<boolean>
  serverClockOffsetMs: Ref<number> | ComputedRef<number>
  hasServerClockOffset: Ref<boolean> | ComputedRef<boolean>
  resolveStatus: (record: TRecord) => ActiveElapsedStatus
  intervalMs?: number
  now?: () => number
}

export function useActiveElapsedDisplayClock<TRecord>(
  options: UseActiveElapsedDisplayClockOptions<TRecord>
) {
  const intervalMs = options.intervalMs ?? 250
  const now = options.now ?? Date.now
  const displayNowMs = ref(now())

  let activeElapsedDisplayTimer: ReturnType<typeof setInterval> | null = null

  const hasVisibleActiveRecords = computed(() => {
    return options.records.value.some((record) => {
      const displayStatus = options.resolveStatus(record)
      return displayStatus === 'pending' || displayStatus === 'streaming'
    })
  })

  const calibratedDisplayNowMs = computed(() => {
    return options.hasServerClockOffset.value
      ? displayNowMs.value + options.serverClockOffsetMs.value
      : displayNowMs.value
  })

  function tickActiveElapsedDisplay() {
    displayNowMs.value = now()
  }

  function startActiveElapsedDisplayTimer() {
    if (activeElapsedDisplayTimer) return
    if (!options.isPageVisible.value || !hasVisibleActiveRecords.value) return
    tickActiveElapsedDisplay()
    activeElapsedDisplayTimer = setInterval(tickActiveElapsedDisplay, intervalMs)
  }

  function stopActiveElapsedDisplayTimer() {
    if (activeElapsedDisplayTimer) {
      clearInterval(activeElapsedDisplayTimer)
      activeElapsedDisplayTimer = null
    }
  }

  function syncActiveElapsedDisplayTimer() {
    if (options.isPageVisible.value && hasVisibleActiveRecords.value) {
      startActiveElapsedDisplayTimer()
    } else {
      stopActiveElapsedDisplayTimer()
    }
  }

  const stopActiveElapsedDisplayWatch = watch(
    [hasVisibleActiveRecords, options.isPageVisible],
    syncActiveElapsedDisplayTimer,
    { immediate: true }
  )

  function stopActiveElapsedDisplayClock() {
    stopActiveElapsedDisplayWatch()
    stopActiveElapsedDisplayTimer()
  }

  return {
    displayNowMs,
    calibratedDisplayNowMs,
    hasVisibleActiveRecords,
    tickActiveElapsedDisplay,
    startActiveElapsedDisplayTimer,
    stopActiveElapsedDisplayTimer,
    syncActiveElapsedDisplayTimer,
    stopActiveElapsedDisplayClock,
  }
}
