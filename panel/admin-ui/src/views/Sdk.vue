<template>
  <div class="page-stack" data-testid="view-sdk">
    <div class="surface">
      <div class="surface-header">{{ t('sdk.title') }}</div>
      <div class="surface-body">
        <el-form label-position="top" @submit.prevent="create">
          <el-form-item :label="t('sdk.site')">
            <el-select v-model="siteId" filterable class="w-full">
              <el-option v-for="s in sites" :key="s.site_id" :label="`${s.site_id} · ${s.name || ''}`" :value="s.site_id" />
            </el-select>
          </el-form-item>
          <el-form-item :label="t('sdk.kind')">
            <el-radio-group v-model="kind">
              <el-radio-button label="backend">{{ t('sdk.backend') }}</el-radio-button>
              <el-radio-button label="fe_embed">{{ t('sdk.embed') }}</el-radio-button>
            </el-radio-group>
          </el-form-item>
          <el-form-item v-if="kind === 'fe_embed'" :label="t('sdk.origins')">
            <el-input v-model="origins" :placeholder="t('sdk.origins_hint')" />
          </el-form-item>
          <el-button type="primary" native-type="submit" :loading="busy" :disabled="!siteId">{{ t('sdk.create') }}</el-button>
        </el-form>
      </div>
    </div>
    <el-alert v-if="secret" type="warning" :closable="false" :title="t('sdk.secret_once')" :description="secret" />
    <div class="surface">
      <div class="surface-header">{{ t('sdk.keys') }}</div>
      <div class="surface-body">
        <el-table :data="keys" stripe>
          <el-table-column prop="site_id" :label="t('sdk.site')" />
          <el-table-column prop="kind" :label="t('sdk.kind')" />
          <el-table-column prop="secret_prefix" :label="t('sdk.prefix')" />
          <el-table-column prop="status" :label="t('sdk.status')" />
          <el-table-column :label="t('app.actions')" width="180">
            <template #default="{ row }">
              <el-button size="small" @click="rotate(row)">{{ t('sdk.rotate') }}</el-button>
              <el-button size="small" type="danger" plain @click="revoke(row)">{{ t('sdk.revoke') }}</el-button>
            </template>
          </el-table-column>
        </el-table>
      </div>
    </div>
    <div class="surface">
      <div class="surface-header">{{ t('sdk.embed_code') }}</div>
      <div class="surface-body">
        <el-radio-group v-model="methodId" class="mb" @change="pickMethod">
          <el-radio-button label="l1">L1</el-radio-button>
          <el-radio-button label="l2">L2</el-radio-button>
          <el-radio-button label="l3">L3</el-radio-button>
          <el-radio-button label="l4">L4</el-radio-button>
          <el-radio-button label="l5">L5</el-radio-button>
        </el-radio-group>
        <div class="muted small mb">{{ methodHint }}</div>
        <el-input v-model="embed" type="textarea" :rows="10" readonly data-testid="sdk-embed" />
        <el-button class="mt" :disabled="!siteId" @click="loadEmbed">{{ t('sdk.load_embed') }}</el-button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { inject, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api'
import { ElMessage, ElMessageBox } from 'element-plus'

const { t } = useI18n()
const tick = inject('refreshTick', ref(0))
const sites = ref([])
const keys = ref([])
const siteId = ref('')
const kind = ref('backend')
const origins = ref('')
const secret = ref('')
const embed = ref('')
const pack = ref(null)
const methodId = ref('l1')
const methodHint = ref('')
const busy = ref(false)

async function load() {
  const s = await api('sites')
  sites.value = s.sites || []
  if (!siteId.value && sites.value[0]) siteId.value = sites.value[0].site_id
  const k = await api(`sdk/keys${siteId.value ? `?site_id=${encodeURIComponent(siteId.value)}` : ''}`)
  keys.value = k.keys || []
}
async function create() {
  busy.value = true
  secret.value = ''
  try {
    const j = await api('sdk/keys', {
      method: 'POST',
      body: JSON.stringify({ site_id: siteId.value, kind: kind.value, allowed_origins: origins.value.split(',').map(x => x.trim()).filter(Boolean) }),
    })
    secret.value = j.key?.secret || ''
    await load()
  } catch (e) { ElMessage.error(e.message || t('app.failed')) } finally { busy.value = false }
}
async function revoke(row) {
  await ElMessageBox.confirm(t('sdk.revoke_confirm'), t('app.confirm'), { type: 'warning' })
  await api(`sdk/keys/${row.key_id}/revoke`, { method: 'POST', body: '{}' })
  await load()
}
async function rotate(row) {
  const j = await api(`sdk/keys/${row.key_id}/rotate`, {
    method: 'POST',
    body: JSON.stringify({ site_id: row.site_id, kind: row.kind, allowed_origins: JSON.parse(row.allowed_origins_json || '[]') }),
  })
  secret.value = j.key?.secret || ''
  await load()
}
function snippetFrom(j) {
  const methods = j.load_methods && j.load_methods.inject && j.load_methods.inject.methods
  pack.value = methods || []
  pickMethod()
  if (!embed.value) embed.value = j.snippet || j.code || JSON.stringify(j, null, 2)
}
function pickMethod() {
  const methods = pack.value || []
  const map = {
    l1: 'l1_direct_pv',
    l2: 'l2_nginx_proxy',
    l3: 'l3_nginx_head',
    l4: 'l4_site_app',
    l5: 'l5_cf_worker',
  }
  const id = map[methodId.value]
  const m = methods.find((x) => x.id === id)
  if (m) {
    methodHint.value = m.note || m.when || m.name || ''
    embed.value = m.snippet || m.doc || JSON.stringify(m, null, 2)
  } else if (methodId.value === 'l5') {
    methodHint.value = t('sdk.l5_hint')
  }
}
async function loadEmbed() {
  const j = await api(`sdk/embed?site_id=${encodeURIComponent(siteId.value)}`)
  snippetFrom(j)
}
onMounted(load)
watch([siteId, tick], load)
</script>

<style scoped>
.w-full { width: 100%; max-width: 520px; }
.mt { margin-top: 10px; }
.mb { margin-bottom: 10px; }
</style>
