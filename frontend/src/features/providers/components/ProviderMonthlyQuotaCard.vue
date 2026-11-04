<template>
  <Card
    v-if="quota > 0"
    class="p-4"
    data-testid="provider-monthly-quota-card"
  >
    <div class="space-y-3">
      <div class="flex items-center justify-between gap-2">
        <h3 class="text-sm font-semibold">
          {{ billingType === 'free_tier' ? t('providers.billing.sectionTitle') : t('ui.subscriptionQuota') }}
        </h3>
        <div class="flex items-center gap-2">
          <Badge
            variant="secondary"
            class="text-xs"
            data-testid="provider-monthly-quota-percent"
          >
            {{ usedPercent.toFixed(1) }}%
          </Badge>
          <Button
            v-if="showResetButton"
            type="button"
            variant="outline"
            size="sm"
            class="h-6 px-2 text-xs"
            :disabled="resetting"
            data-testid="provider-monthly-quota-reset-button"
            @click="$emit('reset')"
          >
            <RotateCcw
              class="w-3 h-3 mr-1"
              :class="{ 'animate-spin': resetting }"
            />
            {{ t('providers.billing.resetUsedQuota') }}
          </Button>
        </div>
      </div>
      <div class="relative w-full h-2 bg-border rounded-full overflow-hidden">
        <div
          class="absolute left-0 top-0 h-full transition-all duration-300"
          :class="barClass"
          :style="{ width: `${cappedUsedPercent}%` }"
        />
      </div>
      <div class="flex items-center justify-between text-xs">
        <span
          class="font-semibold"
          data-testid="provider-monthly-quota-amount"
        >
          ${{ used.toFixed(2) }} / ${{ quota.toFixed(2) }}
        </span>
        <span
          v-if="billingType === 'free_tier'"
          class="text-muted-foreground"
          data-testid="provider-monthly-quota-reset"
        >
          {{ t('providers.billing.freeTierNoAutoReset') }}
        </span>
        <span
          v-else-if="resetDay"
          class="text-muted-foreground"
          data-testid="provider-monthly-quota-reset"
        >
          {{ t('ui.everyMonthOnDay') }} {{ resetDay }} {{ t('ui.reset87d368d') }}
        </span>
      </div>
    </div>
  </Card>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { RotateCcw } from 'lucide-vue-next'
import Badge from '@/components/ui/badge.vue'
import Button from '@/components/ui/button.vue'
import Card from '@/components/ui/card.vue'
import { useI18n } from '@/i18n'

const props = withDefaults(defineProps<{
  used?: number | null
  quota?: number | null
  resetDay?: number | null
  billingType?: string | null
  showResetButton?: boolean
  resetting?: boolean
}>(), {
  used: 0,
  quota: 0,
  resetDay: null,
  billingType: 'monthly_quota',
  showResetButton: false,
  resetting: false,
})

defineEmits<{
  'reset': []
}>()

const { t } = useI18n()

const used = computed(() => Number.isFinite(Number(props.used)) ? Number(props.used) : 0)
const quota = computed(() => Number.isFinite(Number(props.quota)) ? Number(props.quota) : 0)
const usedPercent = computed(() => quota.value > 0 ? (used.value / quota.value) * 100 : 0)
const cappedUsedPercent = computed(() => Math.min(Math.max(usedPercent.value, 0), 100))
const barClass = computed(() => {
  const ratio = quota.value > 0 ? used.value / quota.value : 0
  if (ratio >= 0.9) return 'bg-red-500'
  if (ratio >= 0.7) return 'bg-yellow-500'
  return 'bg-green-500'
})
</script>
