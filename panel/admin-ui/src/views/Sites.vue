<template>
  <div class="page-stack" data-testid="view-sites">
    <div class="surface" data-testid="site-form">
      <div class="surface-header">{{ t('nav.sites') }}</div>
      <div class="surface-body">
        <el-form label-position="top" class="site-form-grid" @submit.prevent>
          <el-form-item :label="t('sites.site_id')">
            <el-input v-model="form.site_id" data-testid="site-id" />
          </el-form-item>
          <el-form-item :label="t('sites.name')">
            <el-input v-model="form.name" data-testid="site-name" />
          </el-form-item>
          <el-form-item :label="t('sites.roots')" class="span-2">
            <el-input v-model="form.roots" data-testid="site-roots" :placeholder="t('sites.roots_ph')" />
          </el-form-item>
          <el-form-item :label="t('sites.fe_load')">
            <el-select v-model="form.fe_load" data-testid="site-fe-load" class="w-full">
              <el-option
                v-for="o in feLoadOpts"
                :key="o"
                :label="t(`sites.opt_${o}`)"
                :value="o"
              />
            </el-select>
          </el-form-item>
          <el-form-item :label="t('sites.edge_mode')">
            <el-select v-model="form.edge_mode" data-testid="site-edge-mode" class="w-full">
              <el-option
                v-for="o in edgeModeOpts"
                :key="o"
                :label="t(`sites.opt_${o}`)"
                :value="o"
              />
            </el-select>
          </el-form-item>
          <el-form-item :label="t('sites.upload_ingest')">
            <el-select v-model="form.upload_ingest" data-testid="site-upload-ingest" class="w-full">
              <el-option
                v-for="o in uploadIngestOpts"
                :key="o"
                :label="t(`sites.opt_${o}`)"
                :value="o"
              />
            </el-select>
          </el-form-item>
          <el-form-item :label="t('sites.poll_method')">
            <el-select v-model="form.poll_method" data-testid="site-poll" class="w-full">
              <el-option
                v-for="o in pollMethodOpts"
                :key="o"
                :label="t(`sites.opt_poll_${o}`)"
                :value="o"
              />
            </el-select>
          </el-form-item>
          <el-form-item :label="t('sites.pv_base')">
            <el-input v-model="form.pv_base" data-testid="site-pv-base" :placeholder="t('sites.pv_base_ph')" />
          </el-form-item>
          <el-form-item :label="t('sites.gv_base')">
            <el-input v-model="form.gv_base" data-testid="site-gv-base" :placeholder="t('sites.gv_base_ph')" />
          </el-form-item>
          <el-form-item :label="t('sites.entry_js')">
            <el-input v-model="form.entry_js" data-testid="site-entry-js" />
          </el-form-item>
          <el-form-item :label="t('sites.cookie_fields')" class="span-2">
            <el-input
              v-model="form.cookie_fields"
              data-testid="site-cookie-fields"
              :placeholder="t('sites.cookie_fields_ph')"
            />
            <div class="muted small">{{ t('sites.cookie_fields_hint') }}</div>
          </el-form-item>
          <el-form-item :label="t('sites.embed_token')" class="span-2">
            <div class="token-row">
              <el-input
                v-model="form.embed_token"
                data-testid="site-embed-token"
                readonly
                :placeholder="t('sites.embed_token_ph')"
              />
              <el-button size="small" :disabled="!form.embed_token" @click="copyToken">{{ t('sites.embed_token_copy') }}</el-button>
              <el-button size="small" :disabled="isNewSite() || !form.site_id" data-testid="site-token-rotate" @click="rotateToken">{{ t('sites.embed_token_rotate') }}</el-button>
            </div>
            <div class="muted small">{{ t('sites.embed_token_hint') }}</div>
          </el-form-item>
          <div v-if="form.upload_ingest === 'first_party'" class="span-2 muted small r3-warn" data-testid="r3-warn">
            {{ t('sites.r3_warn') }}
          </div>
          <div class="form-footer">
            <el-form-item :label="t('sites.collect')">
              <el-switch v-model="form.collect_enabled" data-testid="site-collect" />
            </el-form-item>
            <el-form-item v-if="isNewSite()" :label="t('sites.consent_label')" class="span-2">
              <el-checkbox v-model="form.consent_confirmed" data-testid="site-consent">
                {{ t('sites.consent_text') }}
              </el-checkbox>
            </el-form-item>
            <el-button type="primary" data-testid="site-save" @click="save">{{ t('app.save') }}</el-button>
          </div>
        </el-form>
      </div>
    </div>
    <div class="surface" data-testid="domains-card">
      <div class="surface-header">{{ t('sites.domains_title') }}</div>
      <div class="surface-body">
        <div class="domains-bar">
          <el-select
            v-model="domainSite"
            data-testid="domains-site"
            class="w-domain-site"
            :placeholder="t('sites.domains_pick_site')"
          >
            <el-option
              v-for="s in sites"
              :key="s.site_id"
              :label="(s.name || s.site_id) + ' (' + s.site_id + ')'"
              :value="s.site_id"
            />
          </el-select>
          <el-input
            v-model="newDomain"
            data-testid="domain-host"
            class="w-domain-host"
            :placeholder="t('sites.domains_add_ph')"
            @keyup.enter="addDomain"
          />
          <el-button type="primary" data-testid="domain-add" @click="addDomain">{{ t('sites.domains_add') }}</el-button>
          <el-button data-testid="runtime-apply" @click="applyRuntime">{{ t('sites.domains_apply_sni') }}</el-button>
        </div>
        <el-table :data="domains" data-testid="domains-table" stripe size="small">
          <el-table-column prop="hostname" label="Hostname" min-width="180" />
          <el-table-column :label="t('sites.ssl_status')" width="120">
            <template #default="{ row }">
              <el-tag :type="sslTagType(row.ssl_status)" size="small" effect="dark" round>
                {{ sslLabel(row.ssl_status) }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="t('sites.ssl_expires')" min-width="150">
            <template #default="{ row }">{{ fmtExpires(row.ssl_expires_ms) }}</template>
          </el-table-column>
          <el-table-column :label="t('app.actions')" width="210">
            <template #default="{ row }">
              <el-button size="small" :data-testid="'ssl-mint-' + row.hostname" @click="mintSsl(row)">
                {{ t('sites.ssl_mint') }}
              </el-button>
              <el-button size="small" :data-testid="'ssl-upload-' + row.hostname" @click="openPem(row)">
                {{ t('sites.ssl_upload') }}
              </el-button>
            </template>
          </el-table-column>
        </el-table>
        <div class="muted small direct-hint" data-testid="direct-hint">{{ directHint }}</div>
      </div>
    </div>
    <div class="surface">
      <div class="surface-body">
        <el-table class="sites-table-desktop" :data="sites" data-testid="sites-table" stripe>
          <el-table-column prop="site_id" :label="t('sites.site_id')" min-width="110" />
          <el-table-column prop="name" :label="t('sites.name')" min-width="120" show-overflow-tooltip />
          <el-table-column :label="t('sites.collect')" width="88">
            <template #default="{ row }">
              <el-tag :type="row.collect_enabled ? 'success' : 'info'" size="small" effect="dark" round>
                {{ row.collect_enabled ? t('app.enabled') : t('app.disabled') }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="t('sites.fe_load')" min-width="150" show-overflow-tooltip>
            <template #default="{ row }">{{ optLabel('opt_', row.fe_load) }}</template>
          </el-table-column>
          <el-table-column :label="t('sites.roots')" min-width="180" show-overflow-tooltip>
            <template #default="{ row }">
              <span class="mono muted">{{ (row.root_domains || []).join(', ') }}</span>
            </template>
          </el-table-column>
          <el-table-column :label="t('app.actions')" width="168">
            <template #default="{ row }">
              <el-button size="small" :data-testid="'site-edit-' + row.site_id" @click="edit(row)">{{ t('app.edit') }}</el-button>
              <el-button size="small" type="danger" plain :data-testid="'site-del-' + row.site_id" @click="del(row)">{{ t('app.delete') }}</el-button>
            </template>
          </el-table-column>
        </el-table>
        <div class="site-cards" data-testid="sites-cards">
          <div v-for="row in sites" :key="row.site_id" class="site-card">
            <div class="site-card-top">
              <strong>{{ row.name || row.site_id }}</strong>
              <el-tag :type="row.collect_enabled ? 'success' : 'info'" size="small" effect="dark" round>
                {{ row.collect_enabled ? t('app.enabled') : t('app.disabled') }}
              </el-tag>
            </div>
            <div class="mono muted">{{ row.site_id }} · {{ optLabel('opt_', row.fe_load) }}</div>
            <div class="mono muted">{{ (row.root_domains || []).join(', ') }}</div>
            <div class="site-card-actions">
              <el-button size="small" :data-testid="'site-edit-' + row.site_id" @click="edit(row)">{{ t('app.edit') }}</el-button>
              <el-button size="small" type="danger" plain :data-testid="'site-del-' + row.site_id" @click="del(row)">{{ t('app.delete') }}</el-button>
            </div>
          </div>
        </div>
      </div>
    </div>
    <el-dialog v-model="pemDialog.visible" :title="t('sites.ssl_upload')" width="560" append-to-body>
      <el-form label-position="top">
        <el-form-item :label="t('sites.pem_cert')">
          <el-input v-model="pemDialog.cert" type="textarea" :rows="6" data-testid="pem-cert" />
        </el-form-item>
        <el-form-item :label="t('sites.pem_key')">
          <el-input v-model="pemDialog.key" type="textarea" :rows="6" data-testid="pem-key" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="pemDialog.visible = false">{{ t('app.cancel') }}</el-button>
        <el-button type="primary" data-testid="pem-save" @click="savePem">{{ t('app.save') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, inject, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'
import { ElMessage, ElMessageBox } from 'element-plus'

const { t } = useI18n()
const sites = ref([])

// Direct-bind edge: panel-managed domains + SSL (Pingora SNI, no reverse proxy)
const domains = ref([])
const domainSite = ref('')
const newDomain = ref('')
const pemDialog = reactive({ visible: false, domainId: '', hostname: '', cert: '', key: '' })

const feLoadOpts = ['pv', 'gv', 'first_party']
const edgeModeOpts = ['dual_domain', 'hybrid', 'first_party']
const uploadIngestOpts = ['gv', 'pv', 'first_party']
const pollMethodOpts = ['both', 'poll', 'event']

const form = reactive({
  site_id: '',
  name: '',
  roots: '',
  collect_enabled: true,
  fe_load: 'pv',
  edge_mode: 'dual_domain',
  upload_ingest: 'gv',
  poll_method: 'both',
  pv_base: '',
  gv_base: '',
  entry_js: 'gr.js',
  cookie_fields: '',
  embed_token: '',
  consent_confirmed: false,
})

function isNewSite() {
  return !sites.value.some((s) => s.site_id === form.site_id)
}
const tick = inject('refreshTick', ref(0))

function optLabel(prefix, value) {
  if (!value) return '—'
  const key = `sites.${prefix}${value}`
  const translated = t(key)
  return translated === key ? value : translated
}

async function load() {
  const j = await api('sites')
  sites.value = j.sites || []
}
function edit(row) {
  form.site_id = row.site_id
  form.name = row.name
  form.roots = (row.root_domains || []).join(',')
  form.collect_enabled = !!row.collect_enabled
  form.fe_load = row.fe_load || 'pv'
  form.edge_mode = row.edge_mode || 'dual_domain'
  form.upload_ingest = row.upload_ingest || 'gv'
  form.poll_method = row.poll_method || 'both'
  form.pv_base = row.pv_base || ''
  form.gv_base = row.gv_base || ''
  form.entry_js = row.entry_js || 'gr.js'
  form.cookie_fields = (row.cookie_fields || []).join(',')
  form.embed_token = row.embed_token || ''
}
async function save() {
  const roots = form.roots.split(',').map((s) => s.trim()).filter(Boolean)
  if (!form.site_id || !roots.length) return ElMessage.error(t('sites.require_id_roots'))
  // iss/opus5 02-§5 consent gate: creation is refused server-side without the
  // owner's disclosure confirmation; surface it here before the round-trip.
  if (isNewSite() && !form.consent_confirmed) {
    return ElMessage.error(t('sites.consent_required'))
  }
  try {
    const j = await api('sites', {
      method: 'POST',
      body: JSON.stringify({
        site_id: form.site_id,
        name: form.name || form.site_id,
        root_domains: roots,
        collect_enabled: form.collect_enabled,
        entry_js: form.entry_js || 'gr.js',
        fe_load: form.fe_load || 'pv',
        edge_mode: form.edge_mode || 'dual_domain',
        upload_ingest: form.upload_ingest || 'gv',
        poll_method: form.poll_method || 'both',
        pv_base: (form.pv_base || '').trim(),
        gv_base: (form.gv_base || '').trim(),
        cookie_fields: form.cookie_fields
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean)
          .slice(0, 16),
        embed_token: form.embed_token || '',
        crypto: { suite: 'gr-seal-v2', params: { require_sealed_ingest: false } },
        consent_confirmed: isNewSite() ? form.consent_confirmed : true,
        consent_notice_version: 'v1',
      }),
    })
    if (j.embed_token) form.embed_token = j.embed_token
    ElMessage.success(t('sites.saved'))
    await load()
  } catch (e) {
    const msg = e.message || t('app.failed')
    ElMessage.error(msg === 'upload_ingest_first_party_forbidden' ? t('sites.r3_reject') : msg)
  }
}
async function copyToken() {
  if (!form.embed_token) return
  try {
    await navigator.clipboard.writeText(form.embed_token)
    ElMessage.success(t('sites.embed_token_copied'))
  } catch {
    ElMessage.error(t('app.failed'))
  }
}
async function rotateToken() {
  if (!form.site_id || isNewSite()) return
  await ElMessageBox.confirm(t('sites.embed_token_rotate_confirm'), t('app.confirm'), { type: 'warning' })
  try {
    const j = await api('sites/' + encodeURIComponent(form.site_id) + '/embed_token/rotate', {
      method: 'POST',
      body: '{}',
    })
    form.embed_token = j.embed_token || ''
    ElMessage.success(t('sites.embed_token_rotated'))
    await load()
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  }
}
async function del(row) {
  await ElMessageBox.confirm(t('sites.confirm_delete', { id: row.site_id }), t('app.confirm'), { type: 'warning' })
  await api('sites/delete', { method: 'POST', body: JSON.stringify({ site_id: row.site_id }) })
  ElMessage.success(t('sites.deleted'))
  load()
}

// --- 域名与证书（直连边缘：pv/gv 由服务 TLS 直接承载，无 nginx 反代） ---
const directHint = computed(() => {
  const sid = domainSite.value || (sites.value[0] && sites.value[0].site_id)
  const s = sites.value.find((x) => x.site_id === sid)
  if (!s) return t('sites.domains_pick_site_hint')
  const pv = String(s.pv_base || '').replace(/\/+$/, '')
  const gv = String(s.gv_base || '').replace(/\/+$/, '')
  if (!pv && !gv) return t('sites.direct_hint_unbound')
  const entry = s.entry_js || 'gr.js'
  return t('sites.direct_hint', { script: (pv || gv) + '/' + entry, api: gv || pv })
})

async function loadDomains() {
  if (!domainSite.value) {
    domains.value = []
    return
  }
  try {
    const j = await api('domains?site_id=' + encodeURIComponent(domainSite.value))
    domains.value = j.domains || []
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  }
}
async function addDomain() {
  const host = newDomain.value.trim().toLowerCase()
  if (!domainSite.value || !host) return ElMessage.error(t('sites.domains_pick_site_hint'))
  try {
    await api('domains', {
      method: 'POST',
      body: JSON.stringify({ site_id: domainSite.value, hostname: host, collect_enabled: true }),
    })
    ElMessage.success(t('sites.domains_added'))
    newDomain.value = ''
    await loadDomains()
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  }
}
async function mintSsl(row) {
  try {
    await api('domains/' + encodeURIComponent(row.domain_id) + '/ssl/mint', { method: 'POST', body: '{}' })
    ElMessage.success(t('sites.ssl_minted'))
    await loadDomains()
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  }
}
function openPem(row) {
  pemDialog.domainId = row.domain_id
  pemDialog.hostname = row.hostname
  pemDialog.cert = ''
  pemDialog.key = ''
  pemDialog.visible = true
}
async function savePem() {
  try {
    await api('domains/' + encodeURIComponent(pemDialog.domainId) + '/ssl/pem', {
      method: 'POST',
      body: JSON.stringify({ cert_pem: pemDialog.cert, key_pem: pemDialog.key, set_active: true }),
    })
    ElMessage.success(t('sites.ssl_saved'))
    pemDialog.visible = false
    await loadDomains()
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  }
}
async function applyRuntime() {
  try {
    const j = await api('runtime/apply', { method: 'POST', body: '{}' })
    const notes = (j.notes || []).join(' · ')
    if (j.needs_restart && !j.restart_ran) ElMessage.warning(t('sites.runtime_needs_restart'))
    else ElMessage.success(t('sites.runtime_applied'))
    if (notes) console.log('[runtime/apply]', notes)
    await loadDomains()
  } catch (e) {
    ElMessage.error(e.message || t('app.failed'))
  }
}
function sslTagType(s) {
  if (s === 'uploaded' || s === 'acme') return 'success'
  if (s === 'self_signed') return 'warning'
  return 'info'
}
function sslLabel(s) {
  const key = 'sites.ssl_' + (s || 'none')
  const translated = t(key)
  return translated === key ? (s || 'none') : translated
}
function fmtExpires(ms) {
  if (!ms) return '—'
  try {
    return new Date(ms).toLocaleDateString()
  } catch {
    return '—'
  }
}
watch(domainSite, loadDomains)
onMounted(load)
watch(tick, load)
</script>

<style scoped>
.site-form-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 4px 16px;
}
.site-form-grid .span-2 {
  grid-column: span 2;
}
.w-full {
  width: 100%;
}
.domains-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}
.w-domain-site {
  width: 260px;
}
.w-domain-host {
  width: 240px;
}
.direct-hint {
  margin-top: 10px;
}
.token-row {
  display: flex;
  gap: 8px;
  align-items: center;
  width: 100%;
}
.token-row .el-input {
  flex: 1;
}
.r3-warn {
  color: var(--el-color-warning);
}
@media (max-width: 900px) {
  .w-domain-site,
  .w-domain-host {
    width: 100%;
  }
}
.form-footer {
  grid-column: 1 / -1;
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
  padding-top: 8px;
  margin-top: 4px;
  border-top: 1px solid var(--gv-border);
}
.form-footer .el-form-item {
  margin-bottom: 0;
}
.site-cards {
  display: none;
}
@media (max-width: 900px) {
  .site-form-grid {
    grid-template-columns: 1fr;
  }
  .site-form-grid .span-2 {
    grid-column: auto;
  }
  .sites-table-desktop,
  :deep(.sites-table-desktop) {
    display: none !important;
  }
  .site-cards {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .site-card {
    border: 1px solid var(--gv-border);
    border-radius: 10px;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .site-card-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
  }
  .site-card-actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
}
</style>
