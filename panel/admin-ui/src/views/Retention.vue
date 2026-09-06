<template>
  <div class="page-stack" data-testid="view-retention">
    <el-alert type="info" :closable="false" show-icon :title="t('retention.note')" />

    <div class="surface" data-testid="retention-config">
      <div class="surface-header">{{ t('retention.title') }}</div>
      <div class="surface-body form-grid">
        <div class="field">
          <label>{{ t('retention.enabled') }}</label>
          <el-switch v-model="form.enabled" data-testid="retention-enabled" />
        </div>
        <div class="field">
          <label>{{ t('retention.analysis_days') }}</label>
          <el-input-number
            v-model="form.analysis_retention_days"
            :min="1"
            :max="3650"
            data-testid="retention-analysis-days"
          />
        </div>
        <div class="field">
          <label>{{ t('retention.session_days') }}</label>
          <el-input-number
            v-model="form.session_retention_days"
            :min="1"
            :max="3650"
            data-testid="retention-session-days"
          />
        </div>
        <div class="field">
          <label>{{ t('retention.velocity_days') }}</label>
          <el-input-number
            v-model="form.velocity_retention_days"
            :min="1"
            :max="90"
            data-testid="retention-velocity-days"
          />
        </div>
        <div class="field">
          <label>{{ t('retention.cold_days') }}</label>
          <el-input-number
            v-model="form.cold_ttl_days"
            :min="1"
            :max="365"
            data-testid="retention-cold-days"
          />
        </div>
        <div class="field">
          <label>{{ t('retention.batch_limit') }}</label>
          <el-input-number
            v-model="form.batch_delete_limit"
            :min="10"
            :max="5000"
            :step="50"
            data-testid="retention-batch-limit"
          />
        </div>
        <div class="field">
          <label>{{ t('retention.interval_sec') }}</label>
          <el-input-number
            v-model="form.purge_interval_sec"
            :min="30"
            :max="86400"
            :step="30"
            data-testid="retention-interval"
          />
        </div>
        <div class="toolbar" style="grid-column: 1 / -1">
          <el-button type="primary" data-testid="retention-save" :loading="saving" @click="save">
            {{ t('app.save') }}
          </el-button>
          <el-button data-testid="retention-purge-now" :loading="purging" @click="purgeNow">
            {{ t('retention.purge_now') }}
          </el-button>
        </div>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">{{ t('retention.storage') }}</div>
      <div class="surface-body">
        <div class="sys-row" v-for="(v, k) in storageNote" :key="k">
          <span>{{ noteLabel(k) }}</span>
          <strong class="note-val">{{ noteText(k, v) }}</strong>
        </div>
        <div v-if="!Object.keys(storageNote).length" class="empty-state">{{ t('app.loading') }}</div>
      </div>
    </div>

    <div v-if="lastPurge" class="surface" data-testid="retention-last-purge">
      <div class="surface-header">{{ t('retention.last_purge') }}</div>
      <div class="surface-body">
        <pre class="mono purge-out">{{ lastPurge }}</pre>
      </div>
    </div>

    <div class="surface" data-testid="dsar-panel">
      <div class="surface-header">{{ t('dsar.title') }}</div>
      <div class="surface-body form-grid">
        <div class="field">
          <label>{{ t('dsar.kind') }}</label>
          <el-select v-model="dsar.kind" data-testid="dsar-kind" style="width: 100%">
            <el-option value="visitor_terminal_id" label="visitor_terminal_id" />
            <el-option value="device_id" label="device_id" />
            <el-option value="client_ip" label="client_ip" />
            <el-option value="site_id" label="site_id" />
          </el-select>
        </div>
        <div class="field" style="grid-column: span 2">
          <label>{{ t('dsar.value') }}</label>
          <el-input
            v-model="dsar.value"
            :placeholder="t('dsar.value_ph')"
            data-testid="dsar-value"
            clearable
          />
        </div>
        <div class="toolbar" style="grid-column: 1 / -1">
          <el-button data-testid="dsar-export" :loading="dsarBusy === 'export'" @click="dsarExport">
            {{ t('dsar.export') }}
          </el-button>
          <el-button
            type="danger"
            data-testid="dsar-erase"
            :loading="dsarBusy === 'erase'"
            @click="dsarErase"
          >
            {{ t('dsar.erase') }}
          </el-button>
        </div>
        <div class="dsar-note" style="grid-column: 1 / -1">{{ t('dsar.note') }}</div>
      </div>
    </div>

    <div v-if="dsarOut" class="surface" data-testid="dsar-result">
      <div class="surface-header">{{ t('dsar.result') }}</div>
      <div class="surface-body">
        <pre class="mono purge-out">{{ dsarOut }}</pre>
      </div>
    </div>
  </div>
</template>

<script setup>
import { inject, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'
import { ElMessage, ElMessageBox } from 'element-plus'

const { t, locale } = useI18n()
const tick = inject('refreshTick', ref(0))
const saving = ref(false)
const purging = ref(false)
const storageNote = ref({})
const lastPurge = ref('')
const dsar = reactive({ kind: 'visitor_terminal_id', value: '' })
const dsarBusy = ref('')
const dsarOut = ref('')
const form = reactive({
  enabled: true,
  analysis_retention_days: 30,
  session_retention_days: 30,
  cold_ttl_days: 7,
  batch_delete_limit: 200,
  purge_interval_sec: 300,
  velocity_retention_days: 7,
})

const NOTE_I18N = {
  analysis_encoding: 'retention.note_encoding',
  cold_archive: 'retention.note_cold',
  pg_performance: 'retention.note_pg',
  purge: 'retention.note_purge',
}
const NOTE_LABEL = {
  analysis_encoding: { en: 'Result storage', zh: '结果存储' },
  cold_archive: { en: 'Cold archive', zh: '冷归档' },
  pg_performance: { en: 'Dashboard speed', zh: '看板速度' },
  purge: { en: 'Cleanup', zh: '清理' },
}

function noteLabel(k) {
  const row = NOTE_LABEL[k]
  if (!row) return k
  return locale.value === 'zh' ? row.zh : row.en
}
function noteText(k, v) {
  const key = NOTE_I18N[k]
  if (!key) return String(v)
  const tr = t(key)
  return tr === key ? String(v) : tr
}

async function load() {
  const j = await api('retention')
  const r = j.retention || {}
  form.enabled = r.enabled !== false
  form.analysis_retention_days = r.analysis_retention_days ?? 30
  form.session_retention_days = r.session_retention_days ?? 30
  form.cold_ttl_days = r.cold_ttl_days ?? 7
  form.batch_delete_limit = r.batch_delete_limit ?? 200
  form.purge_interval_sec = r.purge_interval_sec ?? 300
  form.velocity_retention_days = r.velocity_retention_days ?? 7
  storageNote.value = (j.policy && j.policy.storage_note) || {}
}

async function save() {
  saving.value = true
  try {
    await api('retention', {
      method: 'POST',
      body: JSON.stringify({
        retention: {
          enabled: form.enabled,
          analysis_retention_days: form.analysis_retention_days,
          session_retention_days: form.session_retention_days,
          cold_ttl_days: form.cold_ttl_days,
          batch_delete_limit: form.batch_delete_limit,
          purge_interval_sec: form.purge_interval_sec,
          velocity_retention_days: form.velocity_retention_days,
        },
      }),
    })
    ElMessage.success(t('retention.saved'))
    await load()
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  } finally {
    saving.value = false
  }
}

async function purgeNow() {
  try {
    await ElMessageBox.confirm(t('retention.purge_confirm'), t('retention.purge_now'), {
      type: 'warning',
      confirmButtonText: t('app.confirm'),
      cancelButtonText: t('app.cancel'),
    })
  } catch {
    return
  }
  purging.value = true
  try {
    const j = await api('retention/purge', {
      method: 'POST',
      body: JSON.stringify({ batches: 1 }),
    })
    lastPurge.value = JSON.stringify(j.result || j, null, 2)
    ElMessage.success(t('retention.purge_done'))
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  } finally {
    purging.value = false
  }
}

function dsarValidate() {
  if (!dsar.value.trim()) {
    ElMessage.warning(t('dsar.value_required'))
    return false
  }
  return true
}

async function dsarExport() {
  if (!dsarValidate()) return
  dsarBusy.value = 'export'
  try {
    const j = await api(
      `dsar/export?kind=${encodeURIComponent(dsar.kind)}&value=${encodeURIComponent(dsar.value.trim())}`
    )
    dsarOut.value = JSON.stringify(j.result || j, null, 2)
    // Download as JSON file for handover to the requester.
    const blob = new Blob([dsarOut.value], { type: 'application/json' })
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob)
    a.download = `dsar_export_${dsar.kind}_${Date.now()}.json`
    a.click()
    URL.revokeObjectURL(a.href)
    ElMessage.success(t('dsar.export_done'))
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  } finally {
    dsarBusy.value = ''
  }
}

async function dsarErase() {
  if (!dsarValidate()) return
  const val = dsar.value.trim()
  try {
    await ElMessageBox.confirm(t('dsar.erase_confirm', { kind: dsar.kind, value: val }), t('dsar.erase'), {
      type: 'error',
      confirmButtonText: t('dsar.erase'),
      cancelButtonText: t('app.cancel'),
    })
    // Type-to-confirm second gate for irreversible erasure.
    await ElMessageBox.prompt(t('dsar.erase_type_prompt', { value: val }), t('dsar.erase'), {
      type: 'error',
      confirmButtonText: t('dsar.erase'),
      cancelButtonText: t('app.cancel'),
      inputValidator: (v) => (v || '').trim() === val || t('dsar.erase_type_mismatch'),
    })
  } catch {
    return
  }
  dsarBusy.value = 'erase'
  try {
    const j = await api('dsar/erase', {
      method: 'POST',
      body: JSON.stringify({ kind: dsar.kind, value: val }),
    })
    dsarOut.value = JSON.stringify(j, null, 2)
    ElMessage.success(t('dsar.erase_done'))
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  } finally {
    dsarBusy.value = ''
  }
}

onMounted(load)
watch(tick, load)
</script>

<style scoped>
.form-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 16px 20px;
}
.field label {
  display: block;
  font-size: 12px;
  color: var(--gv-text-muted);
  margin-bottom: 6px;
}
.sys-row {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  padding: 10px 0;
  border-bottom: 1px solid var(--gv-border);
  color: var(--gv-text-muted);
  font-size: 13px;
}
.sys-row:last-child { border-bottom: none; }
.note-val {
  color: var(--gv-text);
  font-weight: 500;
  text-align: right;
  max-width: 70%;
}
.purge-out {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 12px;
  color: var(--gv-text-muted);
  max-height: 320px;
  overflow: auto;
}
.mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
.dsar-note {
  font-size: 12px;
  color: var(--gv-text-muted);
  line-height: 1.5;
}
</style>
