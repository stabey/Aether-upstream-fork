<template>
  <Dialog
    :model-value="internalOpen"
    :title="isEditMode ? t('ui.editProvider') : t('guide.nav.concepts.addProvider')"
    :description="isEditMode ? t('ui.updateProviderConfigurationApiEndpointsAndKeysAreManagedSeparately') : t('ui.createANewProviderConfigurationYouCanAddApiEndpoints')"
    :icon="isEditMode ? SquarePen : Server"
    size="xl"
    @update:model-value="handleDialogUpdate"
  >
    <form
      class="space-y-5"
      @submit.prevent="handleSubmit"
    >
      <p
        v-if="!isEditMode && routingGroupId"
        class="rounded-lg bg-muted/40 px-3 py-2 text-sm text-muted-foreground"
      >
        {{ t('ui.theNewProviderIsEnabledOnlyIn') }}{{ routingGroupName || t('ui.theCurrentPolicyGroup') }}{{ t('ui.andIsDisabledByDefaultInOtherPolicyGroups') }}
      </p>
      <!-- 基本信息 -->
      <div class="space-y-3">
        <h3 class="text-sm font-medium border-b pb-2">
          {{ t('ui.basicInformation') }}
        </h3>

        <div class="space-y-1.5">
          <Label for="name">{{ t('ui.name') }}</Label>
          <Input
            id="name"
            v-model="form.name"
            :placeholder="t('ui.exampleOpenaiPrimaryAccount')"
          />
        </div>

        <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
          <div class="space-y-1.5">
            <Label>{{ t('ui.providerType1103e74') }}</Label>
            <Select
              v-model="form.provider_type"
              :disabled="isEditMode"
            >
              <SelectTrigger>
                <SelectValue :placeholder="t('ui.selectda590a8')" />
              </SelectTrigger>
              <SelectContent>
                <!-- 新建模式：允许自定义及各反代类型 -->
                <template v-if="!isEditMode">
                  <SelectItem value="custom">
                    {{ t('ui.custom4eafa9e') }}
                  </SelectItem>
                  <SelectItem value="vertex_ai">
                    Vertex AI
                  </SelectItem>
                  <SelectItem value="claude_code">
                    {{ t('ui.claudeCodeExperimentalFeature') }}
                  </SelectItem>
                  <SelectItem value="codex">
                    Codex
                  </SelectItem>
                  <SelectItem value="chatgpt_web">
                    ChatGPT Web
                  </SelectItem>
                  <SelectItem value="gemini_cli">
                    Gemini CLI
                  </SelectItem>
                  <SelectItem value="grok">
                    Grok
                  </SelectItem>
                  <SelectItem value="xai">
                    xAI
                  </SelectItem>
                  <SelectItem value="kiro">
                    Kiro
                  </SelectItem>
                  <SelectItem value="windsurf">
                    Windsurf
                  </SelectItem>
                  <SelectItem value="antigravity">
                    Antigravity
                  </SelectItem>
                </template>
                <!-- 编辑模式：显示所有类型（兼容已有数据） -->
                <template v-else>
                  <SelectItem value="custom">
                    {{ t('ui.custom4eafa9e') }}
                  </SelectItem>
                  <SelectItem value="vertex_ai">
                    Vertex AI
                  </SelectItem>
                  <SelectItem value="claude_code">
                    {{ t('ui.claudeCodeExperimentalFeature') }}
                  </SelectItem>
                  <SelectItem value="codex">
                    Codex
                  </SelectItem>
                  <SelectItem value="chatgpt_web">
                    ChatGPT Web
                  </SelectItem>
                  <SelectItem value="gemini_cli">
                    Gemini CLI
                  </SelectItem>
                  <SelectItem value="grok">
                    Grok
                  </SelectItem>
                  <SelectItem value="xai">
                    xAI
                  </SelectItem>
                  <SelectItem value="kiro">
                    Kiro
                  </SelectItem>
                  <SelectItem value="windsurf">
                    Windsurf
                  </SelectItem>
                  <SelectItem value="antigravity">
                    Antigravity
                  </SelectItem>
                </template>
              </SelectContent>
            </Select>
            <p
              v-if="!isEditMode && form.provider_type !== 'custom'"
              class="text-xs text-muted-foreground"
            >
              {{ t('ui.reverseProxyProvidersUseFixedEndpointsThatCannotBeModified') }}
            </p>
          </div>
          <div class="space-y-1.5">
            <Label for="website">{{ t('ui.websiteUrl') }}</Label>
            <Input
              id="website"
              v-model="form.website"
              :placeholder="t('ui.httpsExampleComOptional')"
            />
          </div>
        </div>
      </div>

      <!-- 请求配置 -->
      <div class="space-y-3">
        <h3 class="text-sm font-medium border-b pb-2">
          {{ t('ui.requestConfig') }}
        </h3>

        <!-- 超时配置 -->
        <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
          <div class="space-y-1.5">
            <Label>
              {{ t('ui.streamingFirstByteTimeouta94bd47') }}
              <span class="text-xs text-muted-foreground">{{ t('ui.seconds') }}</span>
            </Label>
            <Input
              :model-value="form.stream_first_byte_timeout ?? ''"
              type="number"
              min="1"
              max="300"
              step="1"
              placeholder="30"
              @update:model-value="(v) => form.stream_first_byte_timeout = parseNumberInput(v)"
            />
          </div>
          <div class="space-y-1.5">
            <Label>
              {{ t('ui.nonStreamingRequestTimeoutc0f0f63') }}
              <span class="text-xs text-muted-foreground">{{ t('ui.seconds') }}</span>
            </Label>
            <Input
              :model-value="form.request_timeout ?? ''"
              type="number"
              min="1"
              max="1200"
              step="1"
              placeholder="300"
              @update:model-value="(v) => form.request_timeout = parseNumberInput(v)"
            />
          </div>
        </div>

        <!-- 提供商内转移限制 -->
        <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
          <div class="min-w-0 space-y-1.5">
            <Label
              for="max-transfer-count"
              class="text-xs sm:text-sm"
            >
              {{ t('ui.maxTransfers') }}
            </Label>
            <Input
              id="max-transfer-count"
              :model-value="form.max_transfer_count === 0 ? '' : form.max_transfer_count"
              type="number"
              min="0"
              step="1"
              :placeholder="t('ui.0Unlimited5eb4cbb')"
              @update:model-value="(v) => form.max_transfer_count = parseNumberInput(v, { min: 0 }) ?? 0"
            />
          </div>
          <div class="min-w-0 space-y-1.5">
            <Label
              for="max-transfer-timeout-seconds"
              class="text-xs sm:text-sm"
            >
              {{ t('ui.maxTransferTimeout') }}
              <span class="text-xs text-muted-foreground">{{ t('ui.seconds') }}</span>
            </Label>
            <Input
              id="max-transfer-timeout-seconds"
              :model-value="form.max_transfer_timeout_seconds === 0 ? '' : form.max_transfer_timeout_seconds"
              type="number"
              min="0"
              step="1"
              :placeholder="t('ui.0Unlimited5eb4cbb')"
              @update:model-value="(v) => form.max_transfer_timeout_seconds = parseNumberInput(v, { min: 0 }) ?? 0"
            />
          </div>
        </div>
      </div>

      <!-- 额度与消费限制 -->
      <div
        class="space-y-3"
        data-testid="provider-billing-config-section"
      >
        <h3 class="text-sm font-medium border-b pb-2">
          {{ t('providers.billing.sectionTitle') }}
        </h3>

        <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
          <div class="space-y-1.5">
            <Label for="provider-billing-mode">{{ t('providers.billing.modeLabel') }}</Label>
            <Select v-model="form.billing_mode">
              <SelectTrigger id="provider-billing-mode">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="pay_as_you_go">
                  {{ t('providers.billing.payAsYouGo') }}
                </SelectItem>
                <SelectItem value="free_tier">
                  {{ t('providers.billing.freeTier') }}
                </SelectItem>
                <SelectItem value="monthly_quota">
                  {{ t('providers.billing.monthlyQuota') }}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div
            v-if="form.billing_mode !== 'pay_as_you_go'"
            class="space-y-1.5"
          >
            <Label for="monthly-quota-usd">{{ t('providers.billing.quotaLimitUsd') }}</Label>
            <Input
              id="monthly-quota-usd"
              :model-value="form.monthly_quota_usd ?? ''"
              type="number"
              min="0"
              step="0.01"
              placeholder="400"
              @update:model-value="(v) => form.monthly_quota_usd = parseNumberInput(v, { min: 0 })"
            />
          </div>
        </div>

        <div
          v-if="form.billing_mode === 'monthly_quota'"
          class="grid grid-cols-1 sm:grid-cols-2 gap-4"
        >
          <div class="space-y-1.5">
            <Label for="quota-reset-day">{{ t('providers.billing.resetCycleDays') }}</Label>
            <Input
              id="quota-reset-day"
              :model-value="form.quota_reset_day ?? ''"
              type="number"
              min="1"
              max="365"
              step="1"
              placeholder="30"
              @update:model-value="(v) => form.quota_reset_day = parseNumberInput(v, { min: 1, max: 365 })"
            />
          </div>
        </div>

        <div
          v-if="isEditMode && (form.billing_mode !== 'pay_as_you_go' || (props.provider?.monthly_quota_usd ?? 0) > 0)"
          class="flex items-center justify-between gap-3 rounded-lg border bg-muted/40 px-3 py-2.5"
          data-testid="provider-form-quota-usage"
        >
          <div class="space-y-0.5 text-xs">
            <span class="text-muted-foreground">{{ t('providers.billing.currentUsedUsd') }}：</span>
            <span class="font-semibold text-foreground">
              ${{ currentUsedUsd.toFixed(2) }} / ${{ Number(form.monthly_quota_usd ?? props.provider?.monthly_quota_usd ?? 0).toFixed(2) }}
            </span>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            class="h-7 px-2.5 text-xs"
            :disabled="resettingQuota || loading"
            data-testid="provider-form-reset-quota-button"
            @click="handleResetQuotaInDialog"
          >
            <RotateCcw
              class="w-3.5 h-3.5 mr-1"
              :class="{ 'animate-spin': resettingQuota }"
            />
            {{ t('providers.billing.resetUsedQuota') }}
          </Button>
        </div>
      </div>

      <!-- 功能开关 -->
      <div class="space-y-3">
        <h3 class="text-sm font-medium border-b pb-2">
          {{ t('ui.featureToggles') }}
        </h3>

        <div class="flex items-center justify-between p-3 border rounded-lg bg-muted/50">
          <div class="space-y-0.5">
            <span class="text-sm font-medium">{{ t('ui.poolSchedulingMode') }}</span>
            <p class="text-xs text-muted-foreground">
              {{ t('ui.whenEnabledThisProviderSKeysAreScheduledByThe') }}
            </p>
          </div>
          <Switch
            :model-value="form.pool_mode_enabled"
            @update:model-value="(v: boolean) => form.pool_mode_enabled = v"
          />
        </div>

        <div
          v-if="form.provider_type === 'kiro'"
          class="flex items-center justify-between p-3 border rounded-lg bg-muted/50"
        >
          <div class="space-y-0.5">
            <span class="text-sm font-medium">{{ t('ui.simulatedCacheMode') }}</span>
            <p class="text-xs text-muted-foreground leading-relaxed">
              {{ t('ui.whenEnabledOnlyKiroRequestsSimulatePromptCacheReadWrite') }}
            </p>
          </div>
          <Switch
            :model-value="form.kiro_simulated_cache_enabled"
            @update:model-value="(v: boolean) => form.kiro_simulated_cache_enabled = v"
          />
        </div>

        <div
          v-if="form.provider_type === 'codex'"
          class="flex items-center justify-between gap-4 p-3 border rounded-lg bg-muted/50"
          data-testid="codex-fingerprint-convergence-setting"
        >
          <div class="space-y-0.5">
            <div class="flex items-center gap-1">
              <Label
                for="codex-fingerprint-convergence"
                class="text-sm font-medium"
              >
                {{ t('ui.codexFingerprintConvergenceRecommended') }}
              </Label>
              <HelpHint
                :label="t('ui.codexFingerprintConvergenceRecommended')"
                :text="t('ui.whenEnabledAetherGeneratesAndWritesConsistentIdentifiersForOutbound')"
              />
            </div>
            <p class="text-xs text-muted-foreground leading-relaxed">
              {{ t('ui.useConsistentDeviceAndSessionIdentifiersForTheSameCodex') }}
            </p>
          </div>
          <Switch
            id="codex-fingerprint-convergence"
            :model-value="form.codex_fingerprint_convergence_enabled"
            :aria-label="t('ui.codexFingerprintConvergenceRecommended')"
            @update:model-value="(v: boolean) => form.codex_fingerprint_convergence_enabled = v"
          />
        </div>

        <div
          class="flex items-center justify-between p-3 border rounded-lg bg-muted/50"
          data-testid="responses-websocket-setting"
        >
          <div class="space-y-0.5">
            <Label
              for="responses-websocket-enabled"
              class="text-sm font-medium"
            >
              {{ t('ui.responsesWebsocketMode') }}
            </Label>
            <p class="text-xs text-muted-foreground leading-relaxed">
              {{ t('ui.allowThisProviderToHandleStandardResponsesApiWebsocketRequests') }}
            </p>
          </div>
          <Switch
            id="responses-websocket-enabled"
            :model-value="form.responses_websocket_enabled"
            :aria-label="t('ui.responsesWebsocketMode')"
            @update:model-value="(v: boolean) => form.responses_websocket_enabled = v"
          />
        </div>

        <div class="flex items-center justify-between gap-4 p-3 border rounded-lg bg-muted/50">
          <div class="space-y-0.5">
            <span class="text-sm font-medium">{{ t('ui.sensitiveInformationProtection') }}</span>
            <p class="text-xs text-muted-foreground leading-relaxed">
              {{ t('ui.configureDetailedRulesInModulesSensitiveInformationProtection') }}
            </p>
          </div>
        </div>
      </div>
    </form>

    <template #footer>
      <Button
        type="button"
        variant="outline"
        :disabled="loading"
        @click="handleCancel"
      >
        {{ t('common.cancel') }}
      </Button>
      <Button
        :disabled="loading || !form.name"
        @click="handleSubmit"
      >
        {{ submitLabel }}
      </Button>
    </template>
  </Dialog>
</template>

<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import {
  Dialog,
  Button,
  Input,
  Label,
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
  Switch,
} from '@/components/ui'
import { RotateCcw, Server, SquarePen } from 'lucide-vue-next'
import HelpHint from '@/components/common/HelpHint.vue'
import { useToast } from '@/composables/useToast'
import { useFormDialog } from '@/composables/useFormDialog'
import { useI18n } from '@/i18n'
import {
  createProvider,
  normalizePoolAdvancedConfig,
  updateProvider,
  resetProviderQuota,
  type ProviderType,
  type ProviderWithEndpointsSummary,
} from '@/api/endpoints'
import { parseApiError } from '@/utils/errorParser'
import { parseNumberInput } from '@/utils/form'

const props = defineProps<{
  modelValue: boolean
  provider?: ProviderWithEndpointsSummary | null  // 编辑模式时传入
  routingGroupId?: string
  routingGroupName?: string
  maxPriority?: number  // 当前已有的最大优先级值
}>()

const emit = defineEmits<{
  'update:modelValue': [value: boolean]
  'providerCreated': []
  'providerUpdated': [provider: ProviderWithEndpointsSummary]
}>()

const { t } = useI18n()

const { success, error: showError } = useToast()

const loading = ref(false)
const resettingQuota = ref(false)
const currentUsedUsd = ref(0)
const initialBillingSnapshot = ref<{
  billing_mode: 'pay_as_you_go' | 'free_tier' | 'monthly_quota'
  monthly_quota_usd: number | undefined
  quota_reset_day: number | undefined
}>({
  billing_mode: 'pay_as_you_go',
  monthly_quota_usd: undefined,
  quota_reset_day: 30,
})

// 内部状态
const internalOpen = computed(() => props.modelValue)

// 计算新建时的默认优先级
const defaultPriority = computed(() => {
  if (props.maxPriority != null) {
    return Math.min(props.maxPriority + 10, 10000)
  }
  return 100
})

const submitLabel = computed(() => {
  if (loading.value) {
    return isEditMode.value ? t('ui.saving') : t('ui.creating')
  }
  return isEditMode.value ? t('ui.save') : t('ui.create')
})

// 表单数据
const form = ref({
  name: '',
  provider_type: 'custom' as ProviderType,
  description: '',
  website: '',
  provider_priority: 100,
  // 状态配置
  is_active: true,
  rate_limit: undefined as number | undefined,
  concurrent_limit: undefined as number | undefined,
  // 请求配置
  max_retries: undefined as number | undefined,
  max_transfer_count: 0,
  max_transfer_timeout_seconds: 0,
  // 额度与消费限制
  billing_mode: 'pay_as_you_go' as 'pay_as_you_go' | 'free_tier' | 'monthly_quota',
  monthly_quota_usd: undefined as number | undefined,
  quota_reset_day: 30 as number | undefined,
  // 超时配置（秒）
  stream_first_byte_timeout: undefined as number | undefined,
  request_timeout: undefined as number | undefined,
  // 号池模式
  pool_mode_enabled: false,
  // Codex 专属配置
  codex_fingerprint_convergence_enabled: false,
  // Kiro 专属配置
  kiro_simulated_cache_enabled: false,
  // Responses WebSocket 配置
  responses_websocket_enabled: false,
})

// 重置表单
function resetForm() {
  form.value = {
    name: '',
    provider_type: 'custom',
    description: '',
    website: '',
    provider_priority: defaultPriority.value,
    is_active: true,
    rate_limit: undefined,
    concurrent_limit: undefined,
    // 请求配置
    max_retries: undefined,
    max_transfer_count: 0,
    max_transfer_timeout_seconds: 0,
    // 额度与消费限制
    billing_mode: 'pay_as_you_go',
    monthly_quota_usd: undefined,
    quota_reset_day: 30,
    // 超时配置
    stream_first_byte_timeout: undefined,
    request_timeout: undefined,
    // 号池模式
    pool_mode_enabled: false,
    // Codex 专属配置
    codex_fingerprint_convergence_enabled: false,
    // Kiro 专属配置
    kiro_simulated_cache_enabled: false,
    // Responses WebSocket 配置
    responses_websocket_enabled: false,
  }
  currentUsedUsd.value = 0
  initialBillingSnapshot.value = {
    billing_mode: 'pay_as_you_go',
    monthly_quota_usd: undefined,
    quota_reset_day: 30,
  }
}

// 加载提供商数据（编辑模式）
function loadProviderData() {
  if (!props.provider) return
  const poolAdvanced = normalizePoolAdvancedConfig(props.provider.pool_advanced)
  const existingQuota = props.provider.monthly_quota_usd
  const billingMode: 'pay_as_you_go' | 'free_tier' | 'monthly_quota' =
    props.provider.billing_type === 'monthly_quota'
      ? 'monthly_quota'
      : props.provider.billing_type === 'free_tier'
          ? 'free_tier'
          : 'pay_as_you_go'
  currentUsedUsd.value = props.provider.monthly_used_usd ?? 0
  initialBillingSnapshot.value = {
    billing_mode: billingMode,
    monthly_quota_usd: existingQuota ?? undefined,
    quota_reset_day: props.provider.quota_reset_day ?? 30,
  }
  form.value = {
    name: props.provider.name,
    provider_type: props.provider.provider_type || 'custom',
    description: props.provider.description || '',
    website: props.provider.website || '',
    provider_priority: props.provider.provider_priority || 999,
    is_active: props.provider.is_active,
    rate_limit: undefined,
    concurrent_limit: undefined,
    // 请求配置
    max_retries: props.provider.max_retries ?? undefined,
    max_transfer_count: props.provider.max_transfer_count ?? 0,
    max_transfer_timeout_seconds: props.provider.max_transfer_timeout_seconds ?? 0,
    // 额度与消费限制
    billing_mode: billingMode,
    monthly_quota_usd: existingQuota ?? undefined,
    quota_reset_day: props.provider.quota_reset_day ?? 30,
    // 超时配置
    stream_first_byte_timeout: props.provider.stream_first_byte_timeout ?? undefined,
    request_timeout: props.provider.request_timeout ?? undefined,
    // 号池模式
    pool_mode_enabled: poolAdvanced !== null,
    // Codex 专属配置
    codex_fingerprint_convergence_enabled: props.provider.codex_fingerprint_convergence_enabled ?? false,
    // Kiro 专属配置
    kiro_simulated_cache_enabled: props.provider.kiro_simulated_cache_enabled ?? false,
    // Responses WebSocket 配置
    responses_websocket_enabled: props.provider.responses_websocket_enabled ?? false,
  }
}

// 使用 useFormDialog 统一处理对话框逻辑
const { isEditMode, handleDialogUpdate, handleCancel } = useFormDialog({
  isOpen: () => props.modelValue,
  entity: () => props.provider,
  isLoading: loading,
  onClose: () => emit('update:modelValue', false),
  loadData: loadProviderData,
  resetForm,
})

// 新建模式下切换 provider_type 时不自动开启号池模式
watch(() => form.value.provider_type, () => {
  if (!isEditMode.value) {
    form.value.pool_mode_enabled = false
  }
  if (form.value.provider_type !== 'kiro') {
    form.value.kiro_simulated_cache_enabled = false
  }
  if (form.value.provider_type !== 'codex') {
    form.value.codex_fingerprint_convergence_enabled = false
  }
})


async function handleResetQuotaInDialog() {
  if (!props.provider || resettingQuota.value) return
  resettingQuota.value = true
  try {
    await resetProviderQuota(props.provider.id)
    currentUsedUsd.value = 0
    success(t('providers.billing.resetQuotaSuccess'))
    emit('providerUpdated', {
      ...props.provider,
      monthly_used_usd: 0,
    })
  } catch (error: unknown) {
    showError(parseApiError(error, t('ui.resetFailed')), t('ui.error'))
  } finally {
    resettingQuota.value = false
  }
}
// 提交表单
const handleSubmit = async () => {
  loading.value = true
  try {
    const currentPoolAdvanced = normalizePoolAdvancedConfig(props.provider?.pool_advanced)
    const basePayload = {
      name: form.value.name,
      provider_type: form.value.provider_type,
      description: form.value.description || undefined,
      website: form.value.website || undefined,
      responses_websocket_enabled: form.value.responses_websocket_enabled,
      is_active: form.value.is_active,
      // 请求配置
      max_retries: form.value.max_retries ?? undefined,
      max_transfer_count: form.value.max_transfer_count,
      max_transfer_timeout_seconds: form.value.max_transfer_timeout_seconds,
      // 超时配置（null 表示清除，使用全局配置）
      stream_first_byte_timeout: form.value.stream_first_byte_timeout ?? null,
      request_timeout: form.value.request_timeout ?? null,
      pool_advanced: form.value.pool_mode_enabled
        ? (currentPoolAdvanced ?? {})
        : null,
      ...(form.value.provider_type === 'codex'
        ? {
            codex_fingerprint_convergence_enabled: form.value.codex_fingerprint_convergence_enabled,
          }
        : {}),
      ...(form.value.provider_type === 'kiro'
        ? {
            config: {
              kiro: {
                simulated_cache_enabled: form.value.kiro_simulated_cache_enabled,
              },
            },
          }
        : {}),
    }
    const billingChanged =
      form.value.billing_mode !== initialBillingSnapshot.value.billing_mode
      || form.value.monthly_quota_usd !== initialBillingSnapshot.value.monthly_quota_usd
      || (
        form.value.billing_mode === 'monthly_quota'
        && form.value.quota_reset_day !== initialBillingSnapshot.value.quota_reset_day
      )
    const billingPayload = !billingChanged
      ? {}
      : form.value.billing_mode === 'monthly_quota'
        ? {
            billing_type: 'monthly_quota' as const,
            monthly_quota_usd: form.value.monthly_quota_usd ?? 0,
            quota_reset_day: form.value.quota_reset_day ?? 30,
          }
        : form.value.billing_mode === 'free_tier'
          ? {
              billing_type: 'free_tier' as const,
              monthly_quota_usd: form.value.monthly_quota_usd ?? 0,
              ...(isEditMode.value ? { quota_reset_day: null } : {}),
            }
          : {
              billing_type: 'pay_as_you_go' as const,
              ...(isEditMode.value ? { monthly_quota_usd: null, quota_reset_day: null } : {}),
            }

    if (isEditMode.value && props.provider) {
      // 更新提供商
      const updated = await updateProvider(props.provider.id, {
        ...basePayload,
        ...billingPayload,
        provider_priority: form.value.provider_priority,
      })
      success(t('ui.providerUpdated'))
      emit('providerUpdated', updated)
    } else {
      // 创建提供商（优先级由后端自动置顶）
      await createProvider({
        ...basePayload,
        ...billingPayload,
        routing_group_id: props.routingGroupId,
      })
      success(t('ui.providerCreatedYouCanAddEndpointsAndKeysAndAdjust'), t('ui.createdSuccessfully'))
      emit('providerCreated')
    }

    emit('update:modelValue', false)
  } catch (error: unknown) {
    const action = isEditMode.value ? t('ui.update') : t('ui.create')
    showError(parseApiError(error, t('ui.failedToValueProvider', { value: action })), t('ui.failedToValue', { value: action }))
  } finally {
    loading.value = false
  }
}
</script>
