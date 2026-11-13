<template>
  <div
    ref="rootElement"
    class="relative"
  >
    <button
      ref="triggerElement"
      type="button"
      :class="
        cn(
          'flex h-10 w-full items-center justify-between rounded-lg border bg-background px-3 text-left transition-colors',
          disabled ? 'cursor-not-allowed opacity-60 hover:bg-background' : 'hover:bg-muted/50',
          triggerClass,
        )
      "
      :disabled="disabled"
      :aria-expanded="isOpen"
      aria-haspopup="listbox"
      @click="toggleDropdown"
    >
      <span
        :class="modelValue.length ? 'text-foreground' : 'text-muted-foreground'"
        class="truncate text-sm"
      >
        {{ displayText }}
        <span
          v-if="invalidItems.length"
          class="text-destructive"
        >{{ invalidSummaryText }}</span>
      </span>
      <ChevronDown
        class="h-4 w-4 shrink-0 text-muted-foreground transition-transform"
        :class="isOpen ? 'rotate-180' : ''"
      />
    </button>
    <Teleport
      to="body"
      :disabled="!teleport"
    >
      <div
        v-if="isOpen"
        ref="dropdownElement"
        data-multiselect-dropdown
        :data-side="dropdownSide"
        class="z-[210] overflow-hidden rounded-2xl border border-border bg-card text-foreground shadow-2xl backdrop-blur-xl"
        :class="
          teleport
            ? 'fixed'
            : dropdownSide === 'top'
              ? 'absolute bottom-full mb-1 w-full'
              : 'absolute top-full mt-1 w-full'
        "
        :style="dropdownStyle"
      >
        <div
          v-if="showSearch"
          class="sticky top-0 z-10 border-b border-border/60 bg-card/95 p-1 backdrop-blur supports-[backdrop-filter]:bg-card/85"
        >
          <div class="relative">
            <Search
              class="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground"
            />
            <Input
              v-model="searchQuery"
              :placeholder="localizedSearchPlaceholder"
              class="h-9 rounded-xl border-border/60 bg-background/80 pl-9 pr-3 text-sm"
              @keydown.stop
            />
          </div>
        </div>

        <div
          class="overflow-y-auto overscroll-contain p-1"
          :style="{ maxHeight: `${listMaxHeight}px` }"
        >
          <div
            v-if="hasOptions"
            class="sticky top-0 z-10 flex cursor-pointer items-center gap-2 rounded-lg border-b border-border/60 bg-card/95 px-3 py-2 backdrop-blur hover:bg-muted/50 supports-[backdrop-filter]:bg-card/85"
            @click="toggleAll"
          >
            <input
              type="checkbox"
              :checked="isAllSelected"
              :indeterminate="isPartiallySelected"
              :aria-label="t('ui.selectAll')"
              class="h-4 w-4 shrink-0 cursor-pointer rounded border-border/60 bg-card/80 text-primary shadow-sm accent-primary focus:ring-2 focus:ring-primary/40 focus:ring-offset-1"
              @click.stop
              @change="toggleAll"
            >
            <span class="min-w-0 truncate text-sm">{{ t('ui.selectAll') }}</span>
            <span class="ml-auto shrink-0 text-xs text-muted-foreground">
              {{ selectedOptionCount }}/{{ options.length }}
            </span>
          </div>

          <div
            v-for="item in filteredInvalidItems"
            :key="'invalid-' + item"
            class="flex cursor-pointer items-center gap-2 rounded-lg bg-destructive/5 px-3 py-2 hover:bg-muted/50"
            @click="remove(item)"
          >
            <input
              type="checkbox"
              :checked="true"
              class="h-4 w-4 shrink-0 cursor-pointer rounded border-border/60 bg-card/80 text-primary shadow-sm accent-primary focus:ring-2 focus:ring-primary/40 focus:ring-offset-1"
              @click.stop
              @change="remove(item)"
            >
            <span class="min-w-0 truncate text-sm text-destructive">{{ item }}</span>
            <span class="shrink-0 text-xs text-destructive/70">{{ t('ui.invalidf12eecc') }}</span>
          </div>

          <div
            v-for="item in filteredOptions"
            :key="item.value"
            class="flex cursor-pointer items-center gap-2 rounded-lg px-3 py-2 hover:bg-muted/50"
            @click="toggle(item.value)"
          >
            <input
              type="checkbox"
              :checked="modelValue.includes(item.value)"
              class="h-4 w-4 shrink-0 cursor-pointer rounded border-border/60 bg-card/80 text-primary shadow-sm accent-primary focus:ring-2 focus:ring-primary/40 focus:ring-offset-1"
              @click.stop
              @change="toggle(item.value)"
            >
            <span class="min-w-0 truncate text-sm">{{ item.label }}</span>
          </div>
          <div
            v-if="filteredOptions.length === 0 && filteredInvalidItems.length === 0"
            class="px-3 py-2 text-sm text-muted-foreground"
          >
            {{ searchQuery.trim() ? localizedNoResultsText : localizedEmptyText }}
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { ChevronDown, Search } from 'lucide-vue-next'
import { Input } from '@/components/ui'
import { useEscapeKey } from '@/composables/useEscapeKey'
import { cn } from '@/lib/utils'
import { matchesSearchQuery } from '@/utils/search'
import { useI18n } from '@/i18n'
export interface MultiSelectOption {
  value: string
  label: string
}

const props = withDefaults(
  defineProps<{
    modelValue: string[]
    options: MultiSelectOption[]
    placeholder?: string
    emptyText?: string
    noResultsText?: string
    triggerClass?: string
    dropdownMinWidth?: string
    disabled?: boolean
    searchable?: boolean
    searchThreshold?: number
    searchPlaceholder?: string
    teleport?: boolean
  }>(),
  {
    placeholder: undefined,
    emptyText: undefined,
    noResultsText: undefined,
    triggerClass: '',
    dropdownMinWidth: undefined,
    disabled: false,
    searchable: true,
    searchThreshold: 8,
    searchPlaceholder: undefined,
    teleport: true,
  },
)

const emit = defineEmits<{
  'update:modelValue': [value: string[]]
}>()
const { t } = useI18n()

const DROPDOWN_OFFSET = 4
const VIEWPORT_PADDING = 12
const DEFAULT_LIST_MAX_HEIGHT = 320
const MIN_LIST_MAX_HEIGHT = 96
const SEARCH_BAR_HEIGHT = 46
const ESTIMATED_ROW_HEIGHT = 36
const SCROLLABLE_OVERFLOW_RE = /(auto|scroll|hidden|clip)/

const isOpen = ref(false)
const searchQuery = ref('')
const rootElement = ref<HTMLElement | null>(null)
const triggerElement = ref<HTMLElement | null>(null)
const dropdownElement = ref<HTMLElement | null>(null)
const dropdownSide = ref<'top' | 'bottom'>('bottom')
const listMaxHeight = ref(DEFAULT_LIST_MAX_HEIGHT)
const dropdownPosition = ref<Record<string, string>>({})

const { addToStack: addEscapeListener, removeFromStack: removeEscapeListener } = useEscapeKey(
  () => {
    if (!isOpen.value) return false
    isOpen.value = false
    return true
  },
  { disableOnInput: false },
)

onMounted(() => {
  if (!isOpen.value) {
    removeEscapeListener()
  }
})

const validValues = computed(() => new Set(props.options.map(o => o.value)))

const invalidItems = computed(() =>
  props.modelValue.filter(v => !validValues.value.has(v)),
)

const totalCount = computed(() => props.options.length + invalidItems.value.length)

const hasOptions = computed(() => props.options.length > 0)

const selectedOptionCount = computed(
  () => new Set(props.modelValue.filter(v => validValues.value.has(v))).size,
)

const isAllSelected = computed(
  () => hasOptions.value && selectedOptionCount.value === props.options.length,
)

const isPartiallySelected = computed(
  () => selectedOptionCount.value > 0 && !isAllSelected.value,
)

const showSearch = computed(
  () => props.searchable && totalCount.value >= props.searchThreshold,
)

const localizedPlaceholder = computed(() => props.placeholder ?? t('ui.selectda590a8'))
const localizedEmptyText = computed(() => props.emptyText ?? t('ui.noOptions'))
const localizedNoResultsText = computed(() => props.noResultsText ?? t('common.noSearchResults'))
const localizedSearchPlaceholder = computed(() => props.searchPlaceholder ?? t('common.searchPlaceholder'))
const invalidSummaryText = computed(() => {
  const count = invalidItems.value.length
  return t('feedback.select.invalidSummary', { count })
})

const filteredInvalidItems = computed(() => {
  if (!showSearch.value || !searchQuery.value.trim()) {
    return invalidItems.value
  }
  return invalidItems.value.filter((item) =>
    matchesSearchQuery(searchQuery.value, item),
  )
})

const filteredOptions = computed(() => {
  if (!showSearch.value || !searchQuery.value.trim()) {
    return props.options
  }

  return props.options.filter((item) =>
    matchesSearchQuery(searchQuery.value, item.label, item.value),
  )
})

const displayText = computed(() => {
  if (props.modelValue.length === 0) return localizedPlaceholder.value
  if (props.modelValue.length <= 2) {
    return props.modelValue
      .map((v) => props.options.find((o) => o.value === v)?.label ?? v)
      .join(', ')
  }
  return t('feedback.select.selectedCount', { count: props.modelValue.length })
})
const dropdownStyle = computed(() => {
  const style: Record<string, string> = props.teleport ? { ...dropdownPosition.value } : {}
  if (props.dropdownMinWidth) {
    style.minWidth = `min(${props.dropdownMinWidth}, calc(100vw - ${VIEWPORT_PADDING * 2}px))`
  }
  return style
})

function isEventInsideComponent(event: Event): boolean {
  const rootEl = rootElement.value
  const dropdownEl = dropdownElement.value
  const path = typeof event.composedPath === 'function' ? event.composedPath() : []
  if ((rootEl && path.includes(rootEl)) || (dropdownEl && path.includes(dropdownEl))) {
    return true
  }
  const target = event.target
  if (target instanceof Node) {
    return Boolean(rootEl?.contains(target) || dropdownEl?.contains(target))
  }
  return false
}

function isTriggerClippedByScrollParent(rect: { top: number; bottom: number }): boolean {
  if (typeof window === 'undefined' || !triggerElement.value) return false
  let parent = triggerElement.value.parentElement
  while (parent && parent !== document.body && parent !== document.documentElement) {
    const style = window.getComputedStyle(parent)
    if (SCROLLABLE_OVERFLOW_RE.test(style.overflowY) || SCROLLABLE_OVERFLOW_RE.test(style.overflow)) {
      const parentRect = parent.getBoundingClientRect()
      if (parentRect.height > 0 && (rect.bottom <= parentRect.top || rect.top >= parentRect.bottom)) {
        return true
      }
    }
    parent = parent.parentElement
  }
  return false
}

function updateDropdownPosition(recalculateSide = false) {
  if (typeof window === 'undefined' || !triggerElement.value) return
  const rect = triggerElement.value.getBoundingClientRect()
  const viewportWidth = window.innerWidth || 0
  const viewportHeight = window.innerHeight || 0
  const searchHeight = showSearch.value ? SEARCH_BAR_HEIGHT : 0
  const rowCount =
    (hasOptions.value ? 1 : 0) +
    filteredInvalidItems.value.length +
    Math.max(1, filteredOptions.value.length)
  const desiredListHeight = Math.min(DEFAULT_LIST_MAX_HEIGHT, rowCount * ESTIMATED_ROW_HEIGHT + 8)
  const desiredTotalHeight = searchHeight + desiredListHeight

  if (viewportHeight > 0) {
    const spaceBelow = viewportHeight - rect.bottom - DROPDOWN_OFFSET - VIEWPORT_PADDING
    const spaceAbove = rect.top - DROPDOWN_OFFSET - VIEWPORT_PADDING

    if (recalculateSide) {
      dropdownSide.value = spaceBelow < desiredTotalHeight && spaceAbove > spaceBelow ? 'top' : 'bottom'
    } else if (
      dropdownSide.value === 'bottom' &&
      spaceBelow < MIN_LIST_MAX_HEIGHT + searchHeight &&
      spaceAbove > spaceBelow
    ) {
      dropdownSide.value = 'top'
    } else if (
      dropdownSide.value === 'top' &&
      spaceAbove < MIN_LIST_MAX_HEIGHT + searchHeight &&
      spaceBelow > spaceAbove
    ) {
      dropdownSide.value = 'bottom'
    }

    const availableSideSpace = dropdownSide.value === 'top' ? spaceAbove : spaceBelow
    listMaxHeight.value = Math.min(
      DEFAULT_LIST_MAX_HEIGHT,
      Math.max(MIN_LIST_MAX_HEIGHT, Math.floor(availableSideSpace - searchHeight)),
    )
  } else {
    dropdownSide.value = 'bottom'
    listMaxHeight.value = DEFAULT_LIST_MAX_HEIGHT
  }

  if (!props.teleport) {
    dropdownPosition.value = {}
    return
  }

  const renderedWidth = Math.max(rect.width, dropdownElement.value?.offsetWidth ?? 0)
  const clampedLeft =
    viewportWidth > 0 && renderedWidth > 0
      ? Math.min(
        Math.max(VIEWPORT_PADDING, rect.left),
        Math.max(VIEWPORT_PADDING, viewportWidth - renderedWidth - VIEWPORT_PADDING),
      )
      : rect.left

  const nextPosition: Record<string, string> = {
    left: `${clampedLeft}px`,
    maxWidth: `calc(100vw - ${VIEWPORT_PADDING * 2}px)`,
  }
  if (rect.width > 0) {
    nextPosition.width = `${rect.width}px`
  }
  if (viewportHeight > 0) {
    nextPosition.maxHeight = `${Math.max(120, viewportHeight - VIEWPORT_PADDING * 2)}px`
  }
  if (dropdownSide.value === 'top' && viewportHeight > 0) {
    nextPosition.bottom = `${Math.max(VIEWPORT_PADDING, viewportHeight - rect.top + DROPDOWN_OFFSET)}px`
  } else {
    nextPosition.top = `${Math.max(VIEWPORT_PADDING, rect.bottom + DROPDOWN_OFFSET)}px`
  }
  dropdownPosition.value = nextPosition
}

function handleWindowResize() {
  updateDropdownPosition(true)
}

function handleViewportScroll(event: Event) {
  const target = event.target
  if (dropdownElement.value && target instanceof Node && dropdownElement.value.contains(target)) {
    return
  }
  if (triggerElement.value) {
    const rect = triggerElement.value.getBoundingClientRect()
    if ((rect.width > 0 || rect.height > 0) && isTriggerClippedByScrollParent(rect)) {
      isOpen.value = false
      return
    }
  }
  updateDropdownPosition(false)
}

function handleOutsideInteraction(event: Event) {
  if (!isOpen.value || isEventInsideComponent(event)) return
  isOpen.value = false
}

async function toggleDropdown() {
  if (props.disabled) return
  const nextOpen = !isOpen.value
  if (nextOpen) {
    updateDropdownPosition(true)
  }
  isOpen.value = nextOpen
  if (nextOpen) {
    await nextTick()
    updateDropdownPosition(false)
  }
}

function removeOpenListeners() {
  if (typeof window === 'undefined') return
  window.removeEventListener('resize', handleWindowResize)
  window.removeEventListener('scroll', handleViewportScroll, true)
  document.removeEventListener('pointerdown', handleOutsideInteraction, true)
  document.removeEventListener('click', handleOutsideInteraction, true)
  removeEscapeListener()
}
watch(
  () => props.disabled,
  (disabled) => {
    if (disabled && isOpen.value) {
      isOpen.value = false
    }
  },
)

watch(isOpen, (open) => {
  if (open && typeof window !== 'undefined') {
    window.addEventListener('resize', handleWindowResize)
    window.addEventListener('scroll', handleViewportScroll, true)
    document.addEventListener('pointerdown', handleOutsideInteraction, true)
    document.addEventListener('click', handleOutsideInteraction, true)
    removeEscapeListener()
    addEscapeListener()
  } else {
    searchQuery.value = ''
    removeOpenListeners()
  }
})

onBeforeUnmount(removeOpenListeners)

function toggle(value: string) {
  const newValue = [...props.modelValue]
  const index = newValue.indexOf(value)
  if (index === -1) {
    newValue.push(value)
  } else {
    newValue.splice(index, 1)
  }
  emit('update:modelValue', newValue)
}

function remove(value: string) {
  emit('update:modelValue', props.modelValue.filter(v => v !== value))
}

function toggleAll() {
  const invalidValues = props.modelValue.filter(v => !validValues.value.has(v))

  if (isAllSelected.value) {
    emit('update:modelValue', invalidValues)
    return
  }

  emit('update:modelValue', [
    ...invalidValues,
    ...props.options.map(option => option.value),
  ])
}
</script>
