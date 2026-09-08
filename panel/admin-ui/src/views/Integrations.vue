<template>
  <div class="page-stack" data-testid="view-integrations">
    <el-alert type="info" :closable="false" show-icon :title="t('integrations.note')" />

    <div class="surface" data-testid="ip-enrich-card">
      <div class="surface-header">
        <span>{{ t('integrations.ip_enrichment') }}</span>
        <el-button type="primary" data-testid="integrations-save" :loading="saving" @click="save">{{ t('app.save') }}</el-button>
      </div>
      <div class="surface-body">
      <el-form label-position="top" class="integ-form">
        <el-form-item :label="t('integrations.enabled')">
          <el-switch v-model="ip.enabled" data-testid="ip-enabled" />
        </el-form-item>
        <el-form-item :label="t('integrations.provider')">
          <el-select v-model="ip.provider" class="w-full" style="max-width:360px" data-testid="ip-provider">
            <el-option
              v-for="p in providerOptions"
              :key="p.id"
              :label="providerLabel(p)"
              :value="p.id"
            />
          </el-select>
        </el-form-item>

        <template v-if="ip.provider === 'maxmind_geolite2'">
          <el-form-item :label="t('integrations.account_id')">
            <el-input v-model="providers.maxmind_geolite2.account_id" data-testid="mm-account" />
          </el-form-item>
          <el-form-item :label="t('integrations.license_key')">
            <el-input v-model="providers.maxmind_geolite2.license_key" type="password" show-password data-testid="mm-key" />
          </el-form-item>
          <el-form-item :label="t('integrations.asn_path')">
            <el-input v-model="providers.maxmind_geolite2.asn_mmdb_path" />
          </el-form-item>
          <el-form-item :label="t('integrations.country_path')">
            <el-input v-model="providers.maxmind_geolite2.country_mmdb_path" />
          </el-form-item>
        </template>

        <template v-else-if="ip.provider === 'ipinfo'">
          <el-form-item :label="t('integrations.token')">
            <el-input v-model="providers.ipinfo.token" type="password" show-password data-testid="ipinfo-token" />
          </el-form-item>
          <el-form-item :label="t('integrations.url')">
            <el-input v-model="providers.ipinfo.base_url" />
          </el-form-item>
        </template>

        <template v-else-if="ip.provider === 'custom_http'">
          <el-form-item :label="t('integrations.url')">
            <el-input v-model="providers.custom_http.url" data-testid="custom-url" />
          </el-form-item>
          <el-form-item :label="t('integrations.header_auth')">
            <el-input v-model="providers.custom_http.header_auth" type="password" show-password />
          </el-form-item>
        </template>

        <template v-else>
          <el-form-item :label="t('integrations.asn_path')">
            <el-input v-model="providers.dbip_lite.asn_mmdb_path" placeholder="GR_GEOIP_ASN_MMDB (旧 GV5_)" />
          </el-form-item>
          <el-form-item :label="t('integrations.country_path')">
            <el-input v-model="providers.dbip_lite.country_mmdb_path" placeholder="GR_GEOIP_COUNTRY_MMDB (旧 GV5_)" />
          </el-form-item>
        </template>

        <el-form-item :label="t('integrations.vpn_source')">
          <el-input v-model="ip.vpn_proxy_source" placeholder="none | ip2proxy | spur | …" />
        </el-form-item>

        <el-form-item :label="t('integrations.test_query')">
          <div class="test-row" data-testid="ip-test-row">
            <el-input
              v-model="testIp"
              :placeholder="t('integrations.test_ip_ph')"
              style="max-width: 280px"
              data-testid="ip-test-input"
            />
            <el-button
              type="primary"
              plain
              :loading="testing"
              data-testid="ip-test-run"
              @click="runTest"
            >{{ t('integrations.test_run') }}</el-button>
          </div>
        </el-form-item>
        <el-form-item v-if="testResult">
          <pre class="mono muted" data-testid="ip-test-result" style="margin:0;white-space:pre-wrap">{{ testResult }}</pre>
        </el-form-item>
      </el-form>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">
        <span>{{ t('integrations.webhooks') }}</span>
        <el-button type="primary" data-testid="integrations-save-wh" :loading="saving" @click="save">{{ t('app.save') }}</el-button>
      </div>
      <div class="surface-body">
      <el-form label-position="top" class="integ-form">
        <el-form-item :label="t('integrations.enabled')">
          <el-switch v-model="webhooks.enabled" />
        </el-form-item>
        <el-form-item :label="t('integrations.webhook_url')">
          <el-input v-model="webhooks.result_url" data-testid="webhook-url" />
        </el-form-item>
        <el-form-item :label="t('integrations.webhook_secret')">
          <el-input v-model="webhooks.secret" type="password" show-password />
        </el-form-item>
      </el-form>
      </div>
    </div>

    <div class="surface">
      <div class="surface-header">{{ t('integrations.runtime_env') }}</div>
      <div class="surface-body">
        <pre class="mono muted" style="margin:0;white-space:pre-wrap">{{ runtimeEnv }}</pre>
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
const runtimeEnv = ref('')
const providerOptions = ref([
  { id: 'dbip_lite' },
  { id: 'maxmind_geolite2' },
  { id: 'ipinfo' },
  { id: 'custom_http' },
])

const ip = reactive({ enabled: true, provider: 'dbip_lite', vpn_proxy_source: 'none' })
const providers = reactive({
  dbip_lite: { asn_mmdb_path: '', country_mmdb_path: '', label: 'DB-IP Lite' },
  maxmind_geolite2: { account_id: '', license_key: '', asn_mmdb_path: '', country_mmdb_path: '', label: 'MaxMind' },
  ipinfo: { token: '', base_url: 'https://ipinfo.io', label: 'IPinfo' },
  custom_http: { url: '', header_auth: '', label: 'Custom HTTP' },
})
const webhooks = reactive({ enabled: false, result_url: '', secret: '' })
const testIp = ref('8.8.8.8')
const testing = ref(false)
const testResult = ref('')

// Panel QA P3 (2026-09-08): pre-save provider connectivity check — runs the
// enrichment pipeline on the plane against the CURRENT FORM config (masked
// visitor IPs like 183.192.38.0/24 are accepted and resolve to the /24
// network address server-side).
async function runTest() {
  testing.value = true
  testResult.value = ''
  try {
    const j = await api('integrations/test', {
      method: 'POST',
      body: JSON.stringify({
        ip: testIp.value.trim(),
        config: {
          ip_enrichment: {
            enabled: ip.enabled,
            provider: ip.provider,
            vpn_proxy_source: ip.vpn_proxy_source,
            providers: { ...providers },
          },
        },
      }),
    })
    testResult.value = JSON.stringify(j, null, 2)
  } catch (e) {
    testResult.value = JSON.stringify({ ok: false, error: e.message || 'failed' }, null, 2)
    ElMessage.error(e.message || t('app.failed'))
  } finally {
    testing.value = false
  }
}

function providerLabel(p) {
  const meta = providers[p.id]
  const raw = String(meta?.label || p.label || p.id)
  return raw.replace(/\s*\([^)]*\)\s*$/, '')
}

async function load() {
  const j = await api('integrations')
  const cfg = j.config || {}
  const ie = cfg.ip_enrichment || {}
  ip.enabled = ie.enabled !== false
  ip.provider = ie.provider || 'dbip_lite'
  ip.vpn_proxy_source = ie.vpn_proxy_source || 'none'
  const pr = ie.providers || {}
  for (const k of Object.keys(providers)) {
    if (pr[k]) Object.assign(providers[k], pr[k])
  }
  if (cfg.webhooks) Object.assign(webhooks, cfg.webhooks)
  runtimeEnv.value = JSON.stringify(j.runtime_env || {}, null, 2)
  if (Array.isArray(j.catalog) && j.catalog.length) providerOptions.value = j.catalog
}

async function save() {
  saving.value = true
  try {
    await api('integrations', {
      method: 'POST',
      body: JSON.stringify({
        config: {
          ip_enrichment: {
            enabled: ip.enabled,
            provider: ip.provider,
            vpn_proxy_source: ip.vpn_proxy_source,
            providers: { ...providers },
          },
          webhooks: { ...webhooks },
        },
      }),
    })
    ElMessage.success(t('integrations.saved'))
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
.integ-form {
  max-width: 720px;
}
.w-full { width: 100%; }
.test-row { display: flex; gap: 10px; align-items: center; }
</style>
