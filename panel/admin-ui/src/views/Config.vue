<template>
  <div class="page-stack" data-testid="view-config">
    <el-alert type="info" :closable="false" show-icon :title="t('config.note')" />

    <div class="surface">
      <div class="surface-header">
        {{ t('nav.config') }}
        <span class="ver-chip mono" data-testid="config-version">
          v{{ draft.version || 0 }} · {{ t('config.updated') }} {{ fmtTime(draft.updated_ms) }}
        </span>
      </div>
      <div class="surface-body form-grid">
        <div
          v-for="g in groups"
          :key="g.key"
          class="field-group"
          :data-testid="`config-group-${g.key}`"
        >
          <div class="group-title">{{ t(`config.group_${g.key}`) }}</div>
          <div class="form-grid inner">
            <div v-for="f in g.fields" :key="f.key" class="field">
              <label>{{ t(`config.${f.key}`) }}</label>
              <el-switch
                v-if="f.kind === 'bool'"
                v-model="form[f.key]"
                :data-testid="`cfg-${f.key}`"
              />
              <el-input-number
                v-else
                v-model="form[f.key]"
                :min="f.min"
                :max="f.max"
                :step="f.step || 1"
                :data-testid="`cfg-${f.key}`"
              />
              <div class="hint">{{ t(`config.${f.key}_hint`) }}</div>
            </div>
          </div>
        </div>
        <div class="toolbar" style="grid-column: 1 / -1">
          <el-button data-testid="config-save" :loading="saving" @click="save">
            {{ t('config.save_draft') }}
          </el-button>
          <el-button type="primary" data-testid="config-publish" :loading="publishing" @click="publish">
            {{ t('config.publish') }}
          </el-button>
          <el-button data-testid="config-reload" @click="load">{{ t('app.refresh') }}</el-button>
        </div>
      </div>
    </div>

    <div class="surface" data-testid="config-live">
      <div class="surface-header">{{ t('config.live_title') }}</div>
      <div class="surface-body">
        <div class="sys-row">
          <span>{{ t('config.live_version') }}</span>
          <strong class="mono">{{ live.version || 0 }}</strong>
        </div>
        <div class="sys-row">
          <span>{{ t('config.hot_hint_label') }}</span>
          <strong>{{ t('config.hot_hint') }}</strong>
        </div>
        <pre v-if="lastPublish" class="mono muted" style="margin:8px 0 0">{{ lastPublish }}</pre>
      </div>
    </div>
  </div>
</template>

<script setup>
import { inject, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'
import { ElMessage } from 'element-plus'

const { t } = useI18n()
const form = ref({})
const draft = ref({})
const live = ref({})
const saving = ref(false)
const publishing = ref(false)
const lastPublish = ref('')
const tick = inject('refreshTick', ref(0))

// Field groups — keys mirror gr-probe-store runtime_cfg global_to_json.
const groups = [
  {
    key: 'rate',
    fields: [
      { key: 'rate_limit_open_per_min', min: 0, max: 1000000 },
      { key: 'rate_limit_ingest_per_min', min: 0, max: 1000000 },
      { key: 'rate_limit_analyze_per_min', min: 0, max: 1000000 },
      { key: 'rate_limit_complete_per_min', min: 0, max: 1000000 },
      { key: 'rate_limit_result_per_min', min: 0, max: 1000000 },
      { key: 'rate_limit_client_event_per_min', min: 0, max: 1000000 },
      // v1.0.14: per-IP telemetry cap (default 100) — caps one flooding IP
      // without touching other visitors; independent of the site total.
      { key: 'rate_limit_client_event_per_ip_per_min', min: 0, max: 1000000 },
    ],
  },
  {
    key: 'flood',
    fields: [
      { key: 'robot_fastlane_enabled', kind: 'bool' },
      { key: 'hot_max_vts', min: 0, max: 4000000, step: 256 },
      { key: 'arm_sweep_interval_ms', min: 1000, max: 600000, step: 1000 },
      { key: 'arm_sweep_cap', min: 1, max: 10000, step: 16 },
      { key: 'analyze_claim_batch_flood', min: 1, max: 32 },
    ],
  },
  {
    key: 'cycle',
    fields: [
      { key: 'cycle_cool_ms', min: 60000, max: 604800000, step: 60000 },
      { key: 'cold_ttl_ms', min: 3600000, max: 7776000000, step: 3600000 },
      { key: 'hot_idle_ms', min: 60000, max: 86400000, step: 60000 },
      { key: 'cold_promote_window_ms', min: 60000, max: 604800000, step: 60000 },
    ],
  },
  {
    key: 'analyze',
    fields: [
      { key: 'analyze_idle_upload_ms', min: 5000, max: 600000, step: 5000 },
      { key: 'analyze_debounce_ms', min: 10, max: 5000, step: 10 },
      { key: 'upload_concurrency', min: 2, max: 24 },
      { key: 'complete_on_commercial_silicon', kind: 'bool' },
    ],
  },
]

async function load() {
  const j = await api('config')
  draft.value = j.global || {}
  live.value = j.live || {}
  const f = {}
  for (const g of groups) {
    for (const fd of g.fields) {
      const cur = draft.value[fd.key]
      f[fd.key] = fd.kind === 'bool' ? cur !== false : Number(cur ?? 0)
    }
  }
  form.value = f
}

async function save() {
  saving.value = true
  try {
    await api('config', { method: 'POST', body: JSON.stringify({ ...form.value }) })
    ElMessage.success(t('config.draft_saved'))
    await load()
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  } finally {
    saving.value = false
  }
}

async function publish() {
  publishing.value = true
  try {
    const j = await api('config/publish', { method: 'POST', body: '{}' })
    lastPublish.value = JSON.stringify(j, null, 2)
    ElMessage.success(t('config.published'))
    await load()
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  } finally {
    publishing.value = false
  }
}

function fmtTime(ms) {
  if (!ms) return '—'
  try {
    return new Date(ms).toLocaleString()
  } catch {
    return String(ms)
  }
}

onMounted(load)
</script>

<style scoped>
.field-group {
  grid-column: 1 / -1;
  border-top: 1px solid var(--gv-border, #e5e7eb);
  padding-top: 12px;
}
.field-group:first-child {
  border-top: none;
  padding-top: 0;
}
.group-title {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 10px;
}
.form-grid.inner {
  grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
}
.ver-chip {
  float: right;
  font-size: 12px;
  color: var(--gv-text-dim, #9ca3af);
  font-weight: 400;
}
.field label {
  display: block;
  font-size: 12px;
  color: var(--gv-text-muted, #6b7280);
  margin-bottom: 6px;
}
.field .hint {
  margin-top: 6px;
  font-size: 12px;
  color: var(--gv-text-dim, #9ca3af);
}
.sys-row {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 4px 0;
  font-size: 13px;
}
</style>
