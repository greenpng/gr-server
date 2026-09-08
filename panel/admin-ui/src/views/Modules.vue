<template>
  <div class="page-stack" data-testid="view-modules">
    <div class="kpi-grid cols-3">
      <div class="kpi-card">
        <div class="label">{{ t('modules.release_card') }}</div>
        <div class="mono hint wrap">{{ ota.release_base_url || '—' }}</div>
      </div>
      <div class="kpi-card">
        <div class="label">{{ t('modules.pubkey') }}</div>
        <div class="hint">{{ ota.pubkey_configured ? t('modules.pubkey_ok') : t('modules.pubkey_missing') }}</div>
      </div>
      <div class="kpi-card">
        <div class="label">{{ t('modules.control_version') }}</div>
        <div class="value" style="font-size:22px">{{ ota.control_version || '—' }}</div>
      </div>
    </div>

    <div class="surface" data-testid="ota-full-card">
      <div class="surface-header">
        <span>{{ t('modules.full_title') }}</span>
      </div>
      <div class="surface-body">
        <el-form label-position="top" data-testid="ota-full-form" class="ota-form">
          <el-form-item :label="t('modules.release_url')">
            <div class="url-row">
              <el-input
                v-model="full.release_url"
                placeholder="https://…/releases/download/v6.0.x"
                data-testid="ota-release-url"
                class="url-input"
              />
              <el-button data-testid="ota-set-url" :loading="busy" @click="setReleaseUrl">{{ t('modules.set_url') }}</el-button>
            </div>
          </el-form-item>
          <el-form-item :label="t('modules.include')">
            <div class="chk-wrap">
              <el-checkbox v-model="full.install_modules" data-testid="ota-chk-modules">{{ t('modules.chk_modules') }}</el-checkbox>
              <el-checkbox v-model="full.install_fe" data-testid="ota-chk-fe">{{ t('modules.chk_fe') }}</el-checkbox>
              <el-checkbox v-model="full.install_runtime" data-testid="ota-chk-runtime">{{ t('modules.chk_runtime') }}</el-checkbox>
              <el-checkbox v-model="full.restart_runtime" :disabled="!full.install_runtime" data-testid="ota-chk-restart">{{ t('modules.chk_restart') }}</el-checkbox>
              <el-checkbox v-model="full.auto_apply" data-testid="ota-chk-auto">{{ t('modules.chk_auto') }}</el-checkbox>
            </div>
          </el-form-item>
          <el-form-item v-if="full.auto_apply" :label="t('modules.auto_window')" data-testid="ota-window-item">
            <div class="url-row">
              <el-input
                v-model="full.window"
                placeholder="04:00-05:00"
                data-testid="ota-window-input"
                class="url-input"
                style="max-width: 220px"
              />
              <span class="hint-inline">{{ t('modules.auto_window_hint') }}</span>
            </div>
          </el-form-item>
          <div class="toolbar">
            <el-button type="primary" data-testid="ota-full-upgrade" :loading="busy" @click="fullUpgrade">{{ t('modules.full_upgrade') }}</el-button>
            <el-button data-testid="ota-install-fe-only" :loading="busy" @click="installFe">{{ t('modules.fe_only') }}</el-button>
            <el-button type="warning" data-testid="ota-install-runtime-only" :loading="busy" @click="installRuntime">{{ t('modules.runtime_only') }}</el-button>
          </div>
        </el-form>
        <el-alert class="mt" type="info" :closable="false" show-icon :title="t('modules.layers_title')" :description="t('modules.layers_desc')" />
        <pre v-if="fullOut" class="mono muted out" data-testid="ota-full-out">{{ fullOut }}</pre>
      </div>
    </div>

    <div class="surface" data-testid="ota-auto-card">
      <div class="surface-header">
        <span>{{ t('modules.auto_title') }}</span>
        <div class="toolbar">
          <el-button
            v-if="!(autoState.timer_state?.hold)"
            data-testid="ota-auto-hold-btn"
            @click="setAutoHold(true)"
          >{{ t('modules.auto_hold') }}</el-button>
          <el-button
            v-else
            type="warning"
            data-testid="ota-auto-unhold-btn"
            @click="setAutoHold(false)"
          >{{ t('modules.auto_unhold') }}</el-button>
        </div>
      </div>
      <div class="surface-body">
        <div class="kpi-grid cols-3" data-testid="ota-auto-kpis">
          <div class="kpi-card">
            <div class="label">{{ t('modules.auto_hot_mode') }}</div>
            <div class="value mono" data-testid="ota-auto-mode">{{ autoState.hot_mode || '—' }}</div>
          </div>
          <div class="kpi-card">
            <div class="label">{{ t('modules.auto_desired') }}</div>
            <div class="value mono" data-testid="ota-auto-desired">
              {{ autoState.desired?.desired?.version || '—' }}
              <el-tag v-if="autoState.desired?.desired?.auto_apply" type="success" size="small" style="margin-left:6px">{{ t('modules.auto_on') }}</el-tag>
              <el-tag v-else type="info" size="small" style="margin-left:6px">{{ t('modules.auto_off') }}</el-tag>
            </div>
            <div class="hint mono" v-if="autoState.desired?.desired?.window" data-testid="ota-auto-window">{{ autoState.desired.desired.window }}</div>
          </div>
          <div class="kpi-card">
            <div class="label">{{ t('modules.auto_state') }}</div>
            <div class="value mono" data-testid="ota-auto-last">
              {{ autoState.timer_state?.last_result || t('modules.auto_state_none') }}
            </div>
            <div class="hint mono" v-if="autoState.timer_state?.last_run" data-testid="ota-auto-last-run">{{ autoState.timer_state.last_run }}</div>
          </div>
        </div>
        <pre v-if="autoState.timer_state?.last_error" class="mono muted out" data-testid="ota-auto-error">{{ autoState.timer_state.last_error }}</pre>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">
        <span>{{ t('modules.remote') }}</span>
        <div class="toolbar">
          <el-button data-testid="ota-remote" @click="loadRemote">{{ t('modules.pull_manifest') }}</el-button>
          <el-button type="primary" data-testid="ota-install-all" :loading="busy" @click="installAll">{{ t('modules.install_all') }}</el-button>
        </div>
      </div>
      <div class="surface-body">
        <div class="table-scroll">
          <el-table v-if="remoteModules.length" :data="remoteModules" size="small" data-testid="ota-remote-table">
            <el-table-column prop="name" :label="t('modules.col_module')" width="140" />
            <el-table-column prop="version" :label="t('modules.col_version')" width="100" />
            <el-table-column prop="domain" :label="t('modules.col_domain')" width="120" />
            <el-table-column prop="asset" :label="t('modules.col_asset')" min-width="160" show-overflow-tooltip />
            <el-table-column :label="t('app.actions')" width="160">
              <template #default="{ row }">
                <el-button size="small" type="primary" :data-testid="'ota-install-' + row.name" @click="installOne(row)">{{ t('modules.install_activate') }}</el-button>
              </template>
            </el-table-column>
          </el-table>
        </div>
        <pre v-if="remoteOut" class="mono muted out" data-testid="ota-remote-out">{{ remoteOut }}</pre>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">{{ t('modules.local_versions') }}</div>
      <div class="surface-body">
        <div class="table-scroll">
          <el-table :data="ota.local || []" data-testid="modules-table">
            <el-table-column prop="name" :label="t('modules.col_module')" width="140" />
            <el-table-column prop="version" :label="t('modules.col_version')" width="120" />
            <el-table-column :label="t('modules.col_status')" width="110">
              <template #default="{ row }">
                <el-tag :type="row.active ? 'success' : 'info'" size="small">{{ row.active ? t('modules.status_active') : t('modules.status_staged') }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="path" :label="t('modules.col_path')" min-width="200" show-overflow-tooltip />
            <el-table-column :label="t('app.actions')" width="140">
              <template #default="{ row }">
                <el-button size="small" type="primary" :disabled="row.active" :data-testid="'mod-activate-' + row.name + '-' + row.version" @click="activate(row)">{{ t('modules.activate') }}</el-button>
              </template>
            </el-table-column>
          </el-table>
        </div>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">{{ t('modules.stage_lab') }}</div>
      <div class="surface-body">
        <el-form :inline="false" data-testid="stage-form" class="stage-form" label-position="top">
          <el-form-item label="name"><el-input v-model="stage.name" data-testid="mod-name" /></el-form-item>
          <el-form-item label="version"><el-input v-model="stage.version" data-testid="mod-ver" /></el-form-item>
          <el-form-item label="path"><el-input v-model="stage.path" data-testid="mod-path" /></el-form-item>
          <el-form-item>
            <el-button data-testid="mod-stage" @click="doStage">{{ t('modules.stage') }}</el-button>
          </el-form-item>
        </el-form>
      </div>
    </div>
  </div>
</template>

<script setup>
import { inject, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'
import { ElMessage, ElMessageBox } from 'element-plus'

const { t } = useI18n()
const ota = ref({ local: [], release_base_url: '', pubkey_configured: false, control_version: '' })
const remoteModules = ref([])
const remoteOut = ref('')
const fullOut = ref('')
const busy = ref(false)
const stage = reactive({ name: 'analyze', version: '6.0.1-lab', path: '' })
const autoState = ref({})
const full = reactive({
  release_url: '',
  install_modules: true,
  install_fe: true,
  install_runtime: true,
  restart_runtime: true,
  auto_apply: false,
  window: '',
})
const tick = inject('refreshTick', ref(0))

async function load() {
  ota.value = await api('ota/local')
  if (ota.value.release_base_url && !full.release_url) {
    full.release_url = ota.value.release_base_url
  }
}

// iss/ota-unattended-auto-upgrade-design: unattended status card — the
// hot thread view plus the L3 root timer state file.
async function loadAutoState() {
  try {
    autoState.value = await api('ota/auto-state')
    const d = autoState.value?.desired?.desired
    if (d) {
      full.auto_apply = !!d.auto_apply
      full.window = d.window || ''
    }
  } catch {
    autoState.value = {}
  }
}

async function setAutoHold(hold) {
  busy.value = true
  try {
    await api('ota/auto-hold', { method: 'POST', body: JSON.stringify({ hold }) })
    ElMessage.success(hold ? t('modules.auto_hold_ok') : t('modules.auto_unhold_ok'))
    await loadAutoState()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    busy.value = false
  }
}
async function loadRemote() {
  const j = await api('ota/remote')
  remoteOut.value = JSON.stringify(j, null, 2)
  remoteModules.value = j.manifest?.modules || []
  ElMessage[j.ok ? 'success' : 'warning'](j.ok ? t('modules.pulled') : (j.error || t('modules.remote_fail')))
}
async function setReleaseUrl() {
  if (!full.release_url.trim()) {
    ElMessage.warning(t('modules.need_url'))
    return
  }
  busy.value = true
  try {
    const j = await api('ota/set-release-url', {
      method: 'POST',
      body: JSON.stringify({ url: full.release_url.trim() }),
    })
    fullOut.value = JSON.stringify(j, null, 2)
    ElMessage.success(t('modules.set_url_ok'))
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    busy.value = false
  }
}
async function fullUpgrade() {
  try {
    await ElMessageBox.confirm(t('modules.confirm_full'), t('modules.full_upgrade'), { type: 'warning' })
  } catch {
    return
  }
  busy.value = true
  try {
    const body = {
      release_url: full.release_url.trim() || undefined,
      install_modules: full.install_modules,
      install_fe: full.install_fe,
      install_runtime: full.install_runtime,
      restart_runtime: full.restart_runtime,
      activate: true,
      // Declare unattended auto-apply with this release (None-merge server side:
      // these are always sent explicitly from the form).
      auto_apply: full.auto_apply,
      window: full.window.trim(),
    }
    const j = await api('ota/full-upgrade', { method: 'POST', body: JSON.stringify(body) })
    fullOut.value = JSON.stringify(j, null, 2)
    ElMessage[j.ok ? 'success' : 'warning'](j.ok ? t('modules.full_ok') : t('modules.full_partial'))
    await load()
    await loadAutoState()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    busy.value = false
  }
}
async function installFe() {
  busy.value = true
  try {
    if (full.release_url.trim() && full.release_url.trim() !== ota.value.release_base_url) {
      await api('ota/set-release-url', {
        method: 'POST',
        body: JSON.stringify({ url: full.release_url.trim() }),
      })
    }
    const j = await api('ota/install-fe', { method: 'POST', body: '{}' })
    fullOut.value = JSON.stringify(j, null, 2)
    ElMessage.success(t('modules.fe_ok'))
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    busy.value = false
  }
}
async function installRuntime() {
  try {
    await ElMessageBox.confirm(t('modules.confirm_runtime'), t('modules.runtime_only'), { type: 'warning' })
  } catch {
    return
  }
  busy.value = true
  try {
    if (full.release_url.trim() && full.release_url.trim() !== ota.value.release_base_url) {
      await api('ota/set-release-url', {
        method: 'POST',
        body: JSON.stringify({ url: full.release_url.trim() }),
      })
    }
    const j = await api('ota/install-runtime', {
      method: 'POST',
      body: JSON.stringify({ restart: true }),
    })
    fullOut.value = JSON.stringify(j, null, 2)
    ElMessage[j.ok ? 'success' : 'warning'](j.restarted ? t('modules.runtime_ok') : (j.note || t('modules.full_partial')))
    setTimeout(() => load().catch(() => {}), 3000)
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    busy.value = false
  }
}
async function installOne(row) {
  busy.value = true
  try {
    const j = await api('ota/install', {
      method: 'POST',
      body: JSON.stringify({ name: row.name, version: row.version, activate: true }),
    })
    ElMessage.success(`${j.name}@${j.version}`)
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    busy.value = false
  }
}
async function installAll() {
  busy.value = true
  try {
    const j = await api('ota/install-all', { method: 'POST', body: JSON.stringify({ activate: true }) })
    remoteOut.value = JSON.stringify(j, null, 2)
    ElMessage[j.ok ? 'success' : 'warning'](j.ok ? t('modules.full_ok') : t('modules.full_partial'))
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    busy.value = false
  }
}
async function activate(row) {
  await api('modules/activate', { method: 'POST', body: JSON.stringify({ name: row.name, version: row.version }) })
  ElMessage.success(`${row.name}@${row.version}`)
  load()
}
async function doStage() {
  await api('modules/stage', {
    method: 'POST',
    body: JSON.stringify({ name: stage.name, version: stage.version, domain: stage.name, path: stage.path }),
  })
  ElMessage.success(t('modules.staged_ok'))
  load()
}
onMounted(() => { load(); loadAutoState() })
watch(tick, () => { load(); loadAutoState() })
</script>

<style scoped>
.wrap { word-break: break-all; margin-top: 8px; }
.ota-form { max-width: 860px; }
.url-row {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  align-items: center;
}
.url-row .url-input,
.url-row .el-input { flex: 1 1 360px; min-width: 0; width: 100%; }
.chk-wrap {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 16px;
}
.mt { margin-top: 14px; }
.out {
  margin: 12px 0 0;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 240px;
  overflow: auto;
}
.stage-form {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 8px 16px;
  max-width: 860px;
}
.hint { margin-top: 8px; color: var(--gv-text-muted); }
.hint-inline { color: var(--gv-text-muted); font-size: 12px; }
</style>
