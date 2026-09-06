<template>
  <div class="page-stack" data-testid="view-strategies">
    <el-alert type="info" :closable="false" show-icon :title="t('strategies.note')" />

    <div class="surface" data-testid="strategy-config">
      <div class="surface-header">{{ t('strategies.default') }}</div>
      <div class="surface-body toolbar">
        <el-select v-model="defaultId" style="width:320px" data-testid="strategy-default">
          <el-option
            v-for="p in presets"
            :key="p.strategy_id"
            :label="presetLabel(p)"
            :value="p.strategy_id"
          />
        </el-select>
        <el-button type="primary" data-testid="strategy-save" :loading="saving" @click="save">{{ t('app.save') }}</el-button>
      </div>
    </div>

    <div class="surface" data-testid="result-policy-config">
      <div class="surface-header">{{ t('strategies.result_policy') }}</div>
      <div class="surface-body policy-grid">
        <label class="policy-field">
          <span>{{ t('strategies.rpa_collect') }}</span>
          <el-switch v-model="resultPolicy.rpa_collect" data-testid="result-rpa-collect" />
        </label>
        <label class="policy-field">
          <span>{{ t('strategies.primary_device_lane') }}</span>
          <el-select v-model="resultPolicy.primary_device_lane" data-testid="result-device-lane">
            <el-option v-for="lane in deviceLanes" :key="lane" :label="lane" :value="lane" />
          </el-select>
        </label>
        <label class="policy-field">
          <span>{{ t('strategies.response_profile') }}</span>
          <el-select v-model="resultPolicy.response_profile" data-testid="result-response-profile">
            <el-option v-for="profile in responseProfiles" :key="profile" :label="profile" :value="profile" />
          </el-select>
        </label>
        <label class="policy-field">
          <span>{{ t('strategies.include_signals') }}</span>
          <el-switch v-model="resultPolicy.include_signals" data-testid="result-include-signals" />
        </label>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">
        <span>{{ t('strategies.site_overrides') }}</span>
        <el-button type="primary" data-testid="strategy-save-sites" :loading="saving" @click="save">{{ t('app.save') }}</el-button>
      </div>
      <div class="surface-body">
        <div class="table-scroll">
        <el-table :data="siteRows" data-testid="strategy-site-table">
          <el-table-column prop="site_id" :label="t('strategies.pick_site')" width="140" />
          <el-table-column prop="name" :label="t('sites.name')" width="160" />
          <el-table-column :label="t('strategies.pick_strategy')">
            <template #default="{ row }">
              <el-select v-model="siteMap[row.site_id]" clearable :placeholder="t('strategies.none')" style="width:min(280px, 100%)">
                <el-option
                  v-for="p in presets"
                  :key="p.strategy_id"
                  :label="presetLabel(p)"
                  :value="p.strategy_id"
                />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column :label="t('strategies.response_profile')" min-width="170">
            <template #default="{ row }">
              <el-select v-model="siteResultMap[row.site_id].response_profile" clearable :placeholder="t('strategies.none')">
                <el-option v-for="profile in responseProfiles" :key="profile" :label="profile" :value="profile" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column :label="t('strategies.rpa_collect')" min-width="150">
            <template #default="{ row }">
              <el-select v-model="siteResultMap[row.site_id].rpa_collect" clearable :placeholder="t('strategies.none')">
                <el-option :label="t('strategies.enabled')" :value="true" />
                <el-option :label="t('strategies.disabled')" :value="false" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column :label="t('strategies.primary_device_lane')" min-width="150">
            <template #default="{ row }">
              <el-select v-model="siteResultMap[row.site_id].primary_device_lane" clearable :placeholder="t('strategies.none')">
                <el-option v-for="lane in deviceLanes" :key="lane" :label="lane" :value="lane" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column :label="t('strategies.include_signals')" min-width="150">
            <template #default="{ row }">
              <el-select v-model="siteResultMap[row.site_id].include_signals" clearable :placeholder="t('strategies.none')">
                <el-option :label="t('strategies.enabled')" :value="true" />
                <el-option :label="t('strategies.disabled')" :value="false" />
              </el-select>
            </template>
          </el-table-column>
        </el-table>
        </div>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">{{ t('strategies.presets') }}</div>
      <div class="surface-body">
        <div class="table-scroll">
        <el-table :data="presets" stripe data-testid="strategy-presets">
          <el-table-column prop="strategy_id" :label="t('strategies.strategy_id')" width="160" />
          <el-table-column :label="t('strategies.label')" min-width="200">
            <template #default="{ row }">{{ presetLabel(row) }}</template>
          </el-table-column>
          <el-table-column prop="family" :label="t('strategies.family')" width="140" />
          <el-table-column prop="sensitivity" :label="t('strategies.sensitivity')" width="120" />
          <el-table-column :label="t('strategies.shadow')" width="100">
            <template #default="{ row }">{{ row.shadow_mode ? '✓' : '—' }}</template>
          </el-table-column>
          <el-table-column :label="t('strategies.thresholds')" min-width="220">
            <template #default="{ row }">
              <span class="mono muted">{{ formatTh(row.thresholds) }}</span>
            </template>
          </el-table-column>
        </el-table>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { inject, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'
import { ElMessage } from 'element-plus'

const { t, locale } = useI18n()
const tick = inject('refreshTick', ref(0))
const presets = ref([])
const siteRows = ref([])
const siteMap = reactive({})
const siteResultMap = reactive({})
const defaultId = ref('balanced')
const saving = ref(false)
const deviceLanes = ['dv0', 'dv4', 'dv5', 'dv6']
const responseProfiles = ['basic', 'standard', 'advanced']
const resultPolicy = reactive({
  rpa_collect: true,
  primary_device_lane: 'dv0',
  response_profile: 'standard',
  include_signals: true,
})

function presetLabel(p) {
  if (!p) return ''
  if (locale.value === 'zh') return p.label_zh || p.label_en || p.strategy_id
  return p.label_en || p.label_zh || p.strategy_id
}

function formatTh(th) {
  if (!th || typeof th !== 'object') return '—'
  return Object.entries(th)
    .map(([k, v]) => `${k}=${v}`)
    .join(' · ')
}

async function load() {
  const j = await api('strategies')
  const p = j.presets || {}
  if (Array.isArray(p)) presets.value = p
  else if (Array.isArray(p.strategies)) presets.value = p.strategies
  else if (Array.isArray(p.items)) presets.value = p.items
  else presets.value = []

  const cfg = j.config || {}
  defaultId.value = cfg.default_strategy_id || 'balanced'
  Object.assign(resultPolicy, {
    rpa_collect: cfg.result_policy?.rpa_collect !== false,
    primary_device_lane: deviceLanes.includes(cfg.result_policy?.primary_device_lane)
      ? cfg.result_policy.primary_device_lane
      : 'dv0',
    response_profile: responseProfiles.includes(cfg.result_policy?.response_profile)
      ? cfg.result_policy.response_profile
      : 'standard',
    include_signals: cfg.result_policy?.include_signals !== false,
  })
  siteRows.value = (j.sites || []).map((s) => ({
    site_id: s.site_id || '',
    name: s.name || '',
  }))
  Object.keys(siteMap).forEach((k) => delete siteMap[k])
  Object.keys(siteResultMap).forEach((k) => delete siteResultMap[k])
  const ss = cfg.site_strategies || {}
  const sr = cfg.result_policy?.site_overrides || {}
  for (const s of siteRows.value) {
    if (s.site_id && ss[s.site_id]) siteMap[s.site_id] = ss[s.site_id]
    if (s.site_id) {
      siteResultMap[s.site_id] = {
        response_profile: sr[s.site_id]?.response_profile || undefined,
        rpa_collect: typeof sr[s.site_id]?.rpa_collect === 'boolean' ? sr[s.site_id].rpa_collect : undefined,
        primary_device_lane: sr[s.site_id]?.primary_device_lane || undefined,
        include_signals: typeof sr[s.site_id]?.include_signals === 'boolean' ? sr[s.site_id].include_signals : undefined,
      }
    }
  }
}

async function save() {
  saving.value = true
  try {
    const site_strategies = {}
    for (const [k, v] of Object.entries(siteMap)) {
      if (v) site_strategies[k] = v
    }
    const site_overrides = {}
    for (const [k, v] of Object.entries(siteResultMap)) {
      const patch = {}
      if (v.response_profile) patch.response_profile = v.response_profile
      if (typeof v.rpa_collect === 'boolean') patch.rpa_collect = v.rpa_collect
      if (v.primary_device_lane) patch.primary_device_lane = v.primary_device_lane
      if (typeof v.include_signals === 'boolean') patch.include_signals = v.include_signals
      if (Object.keys(patch).length) site_overrides[k] = patch
    }
    await api('strategies', {
      method: 'POST',
      body: JSON.stringify({
        default_strategy_id: defaultId.value,
        site_strategies,
        result_policy: { ...resultPolicy, site_overrides },
      }),
    })
    ElMessage.success(t('strategies.saved'))
    await load()
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  } finally {
    saving.value = false
  }
}

onMounted(load)
watch(tick, load)
</script>

<style scoped>
.muted { color: #93a4c3; font-size: 12px; }
.policy-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 16px; }
.policy-field { display: flex; flex-direction: column; gap: 8px; color: #93a4c3; font-size: 13px; }
</style>
