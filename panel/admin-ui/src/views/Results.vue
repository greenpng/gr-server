<template>
  <div class="page-stack" data-testid="view-results">
    <el-alert type="info" :closable="false" show-icon :title="t('results.note')" />

    <div class="surface" data-testid="results-filters">
      <div class="surface-body toolbar">
        <el-select v-model="siteId" clearable filterable style="width:200px" data-testid="results-site" :placeholder="t('results.site_filter')">
          <el-option :label="t('results.all_sites')" value="" />
          <el-option v-for="s in sites" :key="s.site_id" :label="`${s.site_id} · ${s.name || ''}`" :value="s.site_id" />
        </el-select>
        <el-select v-model="hours" style="width:110px" data-testid="results-hours">
          <el-option v-for="h in [1, 6, 24, 72, 168]" :key="h" :label="`${h}h`" :value="h" />
        </el-select>
        <el-button type="primary" data-testid="results-load" :loading="loading" @click="load">{{ t('results.load') }}</el-button>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">{{ t('results.summary') }}</div>
      <div class="surface-body">
        <div class="table-scroll">
        <el-table :data="summaryRows" stripe data-testid="results-summary-table" empty-text="—">
          <el-table-column prop="site_id" :label="t('sites.site_id')" width="120" />
          <el-table-column prop="product_version" :label="t('results.product_version')" width="120" />
          <el-table-column prop="sessions" :label="t('results.sessions_n')" width="100" />
          <el-table-column prop="main_complete" :label="t('results.main_complete')" width="120" />
          <el-table-column :label="t('results.main_rate')" width="140">
            <template #default="{ row }">
              <el-progress :percentage="ratePct(row.main_complete_rate)" :stroke-width="8" style="max-width:140px" />
            </template>
          </el-table-column>
          <el-table-column prop="gateway_only" :label="t('results.gateway_only')" width="110" />
          <el-table-column :label="t('results.missing_rate')" width="120">
            <template #default="{ row }">{{ pct(row.missing_probe_rate) }}</template>
          </el-table-column>
        </el-table>
        </div>
        <p v-if="summaryErr" class="err">{{ summaryErr }}</p>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">{{ t('results.sessions') }}</div>
      <div class="surface-body">
        <div class="table-scroll">
        <el-table :data="sessionRows" stripe data-testid="results-sessions-table" max-height="420">
          <el-table-column prop="session_id" label="session" min-width="160" show-overflow-tooltip />
          <el-table-column prop="site_id" :label="t('sites.site_id')" width="100" />
          <el-table-column prop="product_version" :label="t('results.product_version')" width="110" />
          <el-table-column prop="device_id" :label="t('results.device_id')" min-width="140" show-overflow-tooltip />
          <el-table-column prop="bot_verdict" :label="t('results.bot_verdict')" width="110" />
          <el-table-column prop="real_band" :label="t('results.real_band')" width="100" />
          <el-table-column prop="client_ip" :label="t('results.client_ip')" width="120" />
        </el-table>
        </div>
        <p v-if="sessionNote" class="muted">{{ sessionNote }}</p>
        <p v-if="sessionErr" class="err">{{ sessionErr }}</p>
      </div>
    </div>
  </div>
</template>

<script setup>
import { inject, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'

const { t } = useI18n()
const tick = inject('refreshTick', ref(0))
const sites = ref([])
const siteId = ref('')
const hours = ref(24)
const loading = ref(false)
const summaryRows = ref([])
const summaryErr = ref('')
const sessionRows = ref([])
const sessionErr = ref('')
const sessionNote = ref('')

function pct(v) {
  const n = Number(v)
  if (!Number.isFinite(n)) return '—'
  return n <= 1 ? `${(n * 100).toFixed(1)}%` : `${n.toFixed(1)}%`
}
function ratePct(v) {
  const n = Number(v)
  if (!Number.isFinite(n)) return 0
  const p = n <= 1 ? n * 100 : n
  return Math.round(Math.max(0, Math.min(100, p)) * 10) / 10
}

function normalizeSessions(payload) {
  const raw = payload?.sessions
  if (Array.isArray(raw)) return raw
  if (raw && Array.isArray(raw.rows)) return raw.rows
  if (Array.isArray(payload?.rows)) return payload.rows
  if (Array.isArray(payload?.items)) return payload.items
  return []
}

async function loadSites() {
  try {
    const j = await api('sites')
    sites.value = j.sites || []
  } catch {
    sites.value = []
  }
}

async function load() {
  loading.value = true
  summaryErr.value = ''
  sessionErr.value = ''
  sessionNote.value = ''
  try {
    const qs = new URLSearchParams({ hours: String(hours.value) })
    if (siteId.value) qs.set('site_id', siteId.value)
    const sum = await api(`results/summary?${qs}`)
    summaryRows.value = sum.by_site_version || []
    if (sum.error) summaryErr.value = sum.error
  } catch (e) {
    summaryRows.value = []
    summaryErr.value = e.message || t('results.proxy_error')
  }
  try {
    const qs = new URLSearchParams({ limit: '40' })
    if (siteId.value) qs.set('site_id', siteId.value)
    const ses = await api(`results/sessions?${qs}`)
    sessionRows.value = normalizeSessions(ses)
    if (ses.error) sessionErr.value = ses.error
    if (ses.panel_note) sessionNote.value = ses.panel_note
    if (!sessionRows.value.length && !sessionErr.value) sessionNote.value = t('results.empty')
  } catch (e) {
    sessionRows.value = []
    sessionErr.value = e.message || t('results.proxy_error')
  } finally {
    loading.value = false
  }
}

onMounted(async () => {
  await loadSites()
  await load()
})
watch([siteId, hours], () => {
  load()
})
watch(tick, load)
</script>

<style scoped>
.err { color: #f87171; margin-top: 8px; }
.muted { color: #93a4c3; font-size: 13px; margin-top: 8px; }
</style>
