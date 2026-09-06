<template>
  <div class="page-stack" data-testid="view-loadbalance">
    <div class="surface">
      <div class="surface-header">
        <span>{{ t('lb.title') }}</span>
        <el-button
          type="primary"
          data-testid="lb-save"
          :loading="saving"
          @click="save"
        >{{ t('app.save') }}</el-button>
      </div>
      <div class="surface-body">
        <div class="form-grid">
          <div class="fld">
            <label>{{ t('lb.enabled') }}</label>
            <el-switch v-model="cfg.enabled" data-testid="lb-enabled" />
          </div>
          <div class="fld">
            <label>{{ t('lb.mode') }}</label>
            <el-select v-model="cfg.mode" style="width: 220px" data-testid="lb-mode">
              <el-option label="proxy" value="proxy" />
              <el-option label="redirect (302)" value="redirect" />
              <el-option label="internal_ip" value="internal_ip" />
            </el-select>
          </div>
          <div class="fld">
            <label>{{ t('lb.strategy') }}</label>
            <el-select v-model="cfg.strategy" style="width: 220px" data-testid="lb-strategy">
              <el-option label="round_robin (weighted)" value="round_robin" />
              <el-option label="least_inflight" value="least_inflight" />
              <el-option label="ip_hash" value="ip_hash" />
            </el-select>
          </div>
          <div class="fld">
            <label>{{ t('lb.url_scheme') }}</label>
            <el-select v-model="cfg.url_scheme" style="width: 140px" data-testid="lb-scheme">
              <el-option label="http" value="http" />
              <el-option label="https" value="https" />
            </el-select>
          </div>
          <div class="fld">
            <label>{{ t('lb.sticky_cookie') }}</label>
            <el-switch v-model="cfg.sticky_cookie" data-testid="lb-sticky" />
          </div>
          <div class="fld">
            <label>{{ t('lb.active_health_check') }}</label>
            <el-switch v-model="cfg.active_health_check" data-testid="lb-health" />
          </div>
          <div class="fld">
            <label>{{ t('lb.health_interval') }}</label>
            <el-input-number v-model="cfg.health_check_interval_ms" :min="1000" :max="300000" :step="1000" />
          </div>
          <div class="fld">
            <label>{{ t('lb.health_timeout') }}</label>
            <el-input-number v-model="cfg.health_check_timeout_ms" :min="100" :max="30000" :step="100" />
          </div>
          <div class="fld">
            <label>{{ t('lb.health_threshold') }}</label>
            <el-input-number v-model="cfg.health_check_fail_threshold" :min="1" :max="20" />
          </div>
          <div class="fld">
            <label>{{ t('lb.tls_upstream') }}</label>
            <el-switch v-model="cfg.tls_upstream" data-testid="lb-tls" />
          </div>
        </div>
        <div class="fld">
          <label>{{ t('lb.node_weights') }} (JSON: {"node_id": weight})</label>
          <el-input v-model="weightsText" type="textarea" :rows="3" class="mono" data-testid="lb-weights" />
        </div>
        <div class="fld">
          <label>{{ t('lb.internal_cidrs') }} (JSON: [{"cidr": "10.0.0.0/8", "nodes": ["n1"]}])</label>
          <el-input v-model="cidrsText" type="textarea" :rows="4" class="mono" data-testid="lb-cidrs" />
        </div>
        <div class="muted small">{{ t('lb.mode_hint') }}</div>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">{{ t('lb.applied') }}</div>
      <div class="surface-body">
        <div class="load-bits">
          <span class="load-chip">{{ t('lb.applied_enabled') }}: {{ applied.enabled ? '✓' : '—' }}</span>
          <span class="load-chip">{{ t('lb.applied_mode') }}: {{ applied.mode }}</span>
          <span class="load-chip">{{ t('lb.applied_strategy') }}: {{ applied.strategy }}</span>
          <span class="load-chip">healthy: {{ applied.healthy_count }}/{{ applied.total_nodes }}</span>
        </div>
        <div class="table-scroll">
          <el-table :data="applied.nodes || []" stripe empty-text="—" data-testid="lb-nodes">
            <el-table-column prop="node_id" :label="t('cluster.node')" min-width="140" show-overflow-tooltip />
            <el-table-column prop="advertise" :label="t('cluster.advertise')" min-width="140" show-overflow-tooltip />
            <el-table-column prop="internal_addr" label="internal" min-width="140" show-overflow-tooltip>
              <template #default="{ row }">{{ row.internal_addr || '—' }}</template>
            </el-table-column>
            <el-table-column :label="t('lb.probe')" width="110">
              <template #default="{ row }">
                <span :class="row.healthy ? 'ok' : 'bad'">{{ row.healthy ? '✓' : '✗' }}</span>
                <span class="muted small" v-if="row.probe_fails">({{ row.probe_fails }})</span>
              </template>
            </el-table-column>
            <el-table-column prop="load_inflight" :label="t('cluster.inflight')" width="110" />
            <el-table-column :label="t('lb.degraded')" width="100">
              <template #default="{ row }">{{ row.degraded ? '✓' : '—' }}</template>
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

const { t } = useI18n()
const tick = inject('refreshTick', ref(0))
const saving = ref(false)
const applied = ref({})
const weightsText = ref('{}')
const cidrsText = ref('[]')

function defaultCfg() {
  return {
    enabled: false,
    mode: 'proxy',
    strategy: 'round_robin',
    url_scheme: 'http',
    sticky_cookie: false,
    node_weights: {},
    internal_cidrs: [],
    active_health_check: true,
    health_check_interval_ms: 5000,
    health_check_timeout_ms: 2000,
    health_check_fail_threshold: 3,
    tls_upstream: false,
  }
}
const cfg = reactive(defaultCfg())

async function load() {
  try {
    const s = await api('lb/status')
    applied.value = s.applied || {}
    const c = s.config || defaultCfg()
    Object.assign(cfg, c)
    weightsText.value = JSON.stringify(c.node_weights || {}, null, 1)
    cidrsText.value = JSON.stringify(c.internal_cidrs || [], null, 1)
  } catch (e) {
    ElMessage.error(t('app.load_failed') + ' ' + e.message)
  }
}

async function save() {
  let node_weights = {}
  let internal_cidrs = []
  try {
    node_weights = JSON.parse(weightsText.value || '{}')
    internal_cidrs = JSON.parse(cidrsText.value || '[]')
    if (typeof node_weights !== 'object' || Array.isArray(node_weights)) throw new Error('weights')
    if (!Array.isArray(internal_cidrs)) throw new Error('cidrs')
  } catch (e) {
    ElMessage.error(t('lb.bad_json'))
    return
  }
  saving.value = true
  try {
    await api('lb/config', {
      method: 'POST',
      body: JSON.stringify({
        ...cfg,
        node_weights,
        internal_cidrs,
      }),
    })
    ElMessage.success(t('app.saved'))
    await load()
  } catch (e) {
    ElMessage.error((e && e.message) || t('lb.save_failed'))
  } finally {
    saving.value = false
  }
}

onMounted(load)
watch(tick, load)
</script>
<style scoped>
.form-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  gap: 14px;
  margin-bottom: 16px;
}
.fld {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.fld label {
  font-size: 12px;
  color: var(--gv-text-muted);
}
.load-bits {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 12px;
}
.load-chip {
  font-size: 12px;
  color: var(--gv-text-muted);
  background: rgba(148, 163, 184, 0.08);
  border: 1px solid var(--gv-border);
  border-radius: 999px;
  padding: 2px 8px;
}
.ok { color: #34c77b; }
.bad { color: #f56c6c; }
.small { font-size: 12px; }
.muted { color: var(--gv-text-muted); }
.mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
</style>
