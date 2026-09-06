<template>
  <div class="login-page" data-testid="login-screen">
    <div class="login-panel surface">
      <div class="login-brand">
        <div class="brand-mark lg">gr</div>
        <h1>{{ t('login.title') }}</h1>
        <p>{{ t('login.subtitle') }}</p>
      </div>
      <div class="login-lang">
        <el-select v-model="lang" size="small" style="width:118px" @change="onLang">
          <el-option label="English" value="en" />
          <el-option label="中文" value="zh" />
        </el-select>
      </div>
      <div class="login-form">
        <el-form @submit.prevent="onLocalLogin">
          <el-form-item :label="t('login.username')">
            <el-input v-model="username" autocomplete="username" data-testid="login-username" />
          </el-form-item>
          <el-form-item :label="t('login.password')">
            <el-input v-model="password" type="password" show-password autocomplete="current-password" data-testid="login-password" />
          </el-form-item>
          <el-button type="primary" native-type="submit" size="large" class="login-btn" :loading="busy" data-testid="login-local">
            {{ t('login.submit') }}
          </el-button>
        </el-form>
      </div>
    </div>
    <div class="login-aside">
      <h2>{{ t('app.brand') }}</h2>
      <p class="muted">{{ t('dashboard.subtitle') }}</p>
      <ul>
        <li>{{ t('nav.sites') }}</li>
        <li>{{ t('nav.results') }}</li>
        <li>{{ t('nav.strategies') }}</li>
        <li>{{ t('nav.integrations') }}</li>
      </ul>
    </div>
  </div>
</template>

<script setup>
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { setStoredLocale } from '../i18n'
import { api, setAuthedFlag } from '../api'
import { ElMessage } from 'element-plus'

const { t, locale } = useI18n()
const lang = ref(locale.value)
const username = ref('')
const password = ref('')
const busy = ref(false)

function onLang(v) {
  locale.value = v
  setStoredLocale(v)
}

async function onLocalLogin() {
  if (!username.value || !password.value) return ElMessage.error(t('login.required'))
  busy.value = true
  try {
    await api('login', {
      method: 'POST',
      body: JSON.stringify({ username: username.value, password: password.value }),
    })
    setAuthedFlag(true)
    window.location.reload()
  } catch (e) {
    ElMessage.error(e.message || t('login.failed'))
  } finally {
    busy.value = false
  }
}
</script>

<style scoped>
.login-page {
  min-height: 100%;
  display: grid;
  grid-template-columns: minmax(360px, 480px) minmax(0, 1fr);
  column-gap: 0;
  background:
    radial-gradient(900px 500px at 10% -10%, rgba(37, 99, 235, 0.35), transparent 55%),
    radial-gradient(700px 400px at 90% 20%, rgba(124, 58, 237, 0.22), transparent 50%),
    var(--gv-bg);
}
.login-panel {
  margin: 0;
  align-self: center;
  justify-self: center;
  width: min(400px, calc(100% - 48px));
  padding: 28px 28px 24px;
  position: relative;
  z-index: 2;
}
.login-brand {
  text-align: left;
  margin-bottom: 8px;
}
.login-brand h1 {
  margin: 14px 0 6px;
  font-size: 22px;
  letter-spacing: -0.02em;
}
.login-brand p {
  margin: 0;
  color: var(--gv-text-muted);
  font-size: 13px;
}
.brand-mark.lg {
  width: 44px;
  height: 44px;
  font-size: 13px;
}
.login-lang {
  display: flex;
  justify-content: flex-end;
  margin: 8px 0 4px;
}
.login-form {
  margin-top: 16px;
}
.login-btn {
  width: 100%;
  margin-top: 4px;
}
.or {
  margin: 16px 0 8px;
  text-align: center;
  color: var(--gv-text-muted);
  font-size: 12px;
}
.hint {
  margin-top: 14px;
  color: var(--gv-text-muted);
  font-size: 12px;
  line-height: 1.45;
}
.login-aside {
  display: flex;
  flex-direction: column;
  justify-content: center;
  padding: 48px 8vw 48px 40px;
  color: #e2e8f0;
  min-width: 0;
}
.login-aside h2 {
  margin: 0 0 8px;
  font-size: 32px;
  letter-spacing: -0.03em;
}
.login-aside ul {
  margin: 24px 0 0;
  padding: 0;
  list-style: none;
  display: grid;
  gap: 10px;
}
.login-aside li {
  padding: 12px 14px;
  border-radius: 10px;
  border: 1px solid rgba(148, 163, 184, 0.18);
  background: rgba(15, 23, 42, 0.45);
  color: #cbd5e1;
}
@media (max-width: 900px) {
  .login-page {
    grid-template-columns: 1fr;
    place-items: center;
    padding: 24px;
  }
  .login-panel {
    margin: 0;
  }
  .login-aside {
    display: none;
  }
}
</style>
