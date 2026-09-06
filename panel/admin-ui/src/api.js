const base = location.pathname.replace(/\/?$/, '/')

/** Session is HttpOnly cookie; this flag only drives client router UX. */
const AUTH_FLAG = 'gr_auth'

export function isAuthedFlag() {
  return localStorage.getItem(AUTH_FLAG) === '1'
}
export function setAuthedFlag(on) {
  if (on) localStorage.setItem(AUTH_FLAG, '1')
  else localStorage.removeItem(AUTH_FLAG)
}

export async function api(path, opts = {}) {
  const headers = Object.assign({ 'content-type': 'application/json' }, opts.headers || {})
  // Prefer cookie session; Bearer optional for tools (not set by SPA)
  const r = await fetch(base + 'api/' + path.replace(/^\//, ''), {
    ...opts,
    headers,
    credentials: 'same-origin',
  })
  const j = await r.json().catch(() => ({}))
  if (r.status === 401) {
    setAuthedFlag(false)
    throw new Error(j.error || 'unauthorized')
  }
  if (!r.ok) throw new Error(j.error || r.statusText || 'request_failed')
  return j
}

/** Back-compat name for router */
export function token() {
  return isAuthedFlag() ? 'cookie' : ''
}

export function setToken(_t) {
  // no-op: session is HttpOnly cookie set by server
}

export { base }
