/**
 * PostgreSQL — sole database for official-site (local + production).
 * SQL uses ? placeholders; converted to $1, $2, … for pg.
 */
import pg from "pg";

const { Pool } = pg;

export function toPg(sql) {
  let n = 0;
  return sql.replace(/\?/g, () => `$${++n}`);
}

export function defaultDatabaseUrl() {
  return (
    (process.env.GR_DATABASE_URL ?? process.env.GV6_DATABASE_URL) ||
    process.env.DATABASE_URL ||
    "postgresql://gv6:gv6@127.0.0.1:5432/gv6_official"
  );
}

export async function openDb(connectionString = defaultDatabaseUrl()) {
  const pool = new Pool({ connectionString, max: 20 });
  await pool.query("SELECT 1");
  return {
    pool,
    async get(sql, ...params) {
      const r = await pool.query(toPg(sql), params);
      return r.rows[0] || null;
    },
    async all(sql, ...params) {
      const r = await pool.query(toPg(sql), params);
      return r.rows;
    },
    async run(sql, ...params) {
      const r = await pool.query(toPg(sql), params);
      return { changes: r.rowCount ?? 0 };
    },
    async exec(sql) {
      await pool.query(sql);
    },
    async transaction(fn) {
      const client = await pool.connect();
      const tx = {
        async get(sql, ...params) {
          const r = await client.query(toPg(sql), params);
          return r.rows[0] || null;
        },
        async all(sql, ...params) {
          const r = await client.query(toPg(sql), params);
          return r.rows;
        },
        async run(sql, ...params) {
          const r = await client.query(toPg(sql), params);
          return { changes: r.rowCount ?? 0 };
        },
      };
      try {
        await client.query("BEGIN");
        const result = await fn(tx);
        await client.query("COMMIT");
        return result;
      } catch (e) {
        await client.query("ROLLBACK");
        throw e;
      } finally {
        client.release();
      }
    },
    async close() {
      await pool.end();
    },
  };
}

export async function migrate(db) {
  await db.exec(`
    CREATE TABLE IF NOT EXISTS users (
      id TEXT PRIMARY KEY,
      email TEXT NOT NULL UNIQUE,
      password_hash TEXT NOT NULL,
      created_at BIGINT NOT NULL,
      email_verified BOOLEAN NOT NULL DEFAULT FALSE,
      totp_secret TEXT,
      totp_enabled BOOLEAN NOT NULL DEFAULT FALSE,
      parent_user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
      account_role TEXT NOT NULL DEFAULT 'owner'
    );
    CREATE TABLE IF NOT EXISTS sessions (
      token TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      expires_at BIGINT NOT NULL,
      created_at BIGINT NOT NULL,
      recent_auth_at BIGINT
    );
    CREATE TABLE IF NOT EXISTS sites (
      site_id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      domain TEXT NOT NULL UNIQUE,
      name TEXT NOT NULL,
      plan TEXT NOT NULL DEFAULT 'free',
      paid_until BIGINT,
      strategy_version_id TEXT NOT NULL,
      created_at BIGINT NOT NULL,
      verify_token TEXT,
      verified_at BIGINT,
      verify_method TEXT
    );
    CREATE TABLE IF NOT EXISTS strategy_versions (
      version_id TEXT PRIMARY KEY,
      title TEXT NOT NULL,
      plan_min TEXT NOT NULL,
      body_json TEXT NOT NULL,
      created_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS payments (
      id TEXT PRIMARY KEY,
      site_id TEXT NOT NULL,
      user_id TEXT NOT NULL,
      amount_usd DOUBLE PRECISION NOT NULL,
      created_at BIGINT NOT NULL,
      note TEXT
    );
    CREATE TABLE IF NOT EXISTS oauth_clients (
      client_id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      redirect_uris_json TEXT NOT NULL,
      created_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS oauth_codes (
      code TEXT PRIMARY KEY,
      client_id TEXT NOT NULL,
      user_id TEXT NOT NULL,
      redirect_uri TEXT NOT NULL,
      code_challenge TEXT NOT NULL,
      expires_at BIGINT NOT NULL,
      created_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS access_tokens (
      token TEXT PRIMARY KEY,
      refresh_token TEXT,
      user_id TEXT NOT NULL,
      client_id TEXT NOT NULL,
      expires_at BIGINT NOT NULL,
      created_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS nodes (
      instance_id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      label TEXT,
      x25519_pub_b64 TEXT NOT NULL,
      created_at BIGINT NOT NULL,
      last_seen_at BIGINT
    );
    CREATE TABLE IF NOT EXISTS tickets (
      id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      site_id TEXT,
      category TEXT NOT NULL,
      subject TEXT NOT NULL,
      body TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'open',
      created_at BIGINT NOT NULL,
      updated_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS ticket_replies (
      id TEXT PRIMARY KEY,
      ticket_id TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
      author TEXT NOT NULL,
      body TEXT NOT NULL,
      created_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS email_tokens (
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      token_hash TEXT NOT NULL,
      expires_at BIGINT NOT NULL,
      PRIMARY KEY (user_id, token_hash)
    );
    CREATE TABLE IF NOT EXISTS password_reset_tokens (
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      token_hash TEXT NOT NULL,
      expires_at BIGINT NOT NULL,
      PRIMARY KEY (user_id, token_hash)
    );
    CREATE TABLE IF NOT EXISTS totp_recovery_codes (
      user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      code_hash TEXT NOT NULL,
      created_at BIGINT NOT NULL,
      used_at BIGINT,
      PRIMARY KEY (user_id, code_hash)
    );
    CREATE TABLE IF NOT EXISTS account_security_events (
      id BIGSERIAL PRIMARY KEY,
      user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
      action TEXT NOT NULL,
      ip TEXT NOT NULL,
      created_at BIGINT NOT NULL,
      detail_json TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX IF NOT EXISTS account_security_events_user_created
      ON account_security_events(user_id, created_at DESC);
    CREATE TABLE IF NOT EXISTS rate_limit_hits (
      k TEXT NOT NULL,
      ts BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS admin_settings (
      setting_key TEXT PRIMARY KEY,
      value_json TEXT NOT NULL,
      updated_at BIGINT NOT NULL,
      updated_by TEXT NOT NULL,
      revision INTEGER NOT NULL DEFAULT 1
    );
    CREATE INDEX IF NOT EXISTS rate_limit_hits_k_ts ON rate_limit_hits (k, ts);
    CREATE TABLE IF NOT EXISTS stripe_events (
      event_id TEXT PRIMARY KEY,
      type TEXT NOT NULL,
      created_at BIGINT NOT NULL,
      processed_at BIGINT,
      payload_json TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS stripe_customers (
      site_id TEXT PRIMARY KEY REFERENCES sites(site_id) ON DELETE CASCADE,
      customer_id TEXT NOT NULL UNIQUE
    );
    CREATE TABLE IF NOT EXISTS stripe_subscriptions (
      site_id TEXT PRIMARY KEY REFERENCES sites(site_id) ON DELETE CASCADE,
      subscription_id TEXT NOT NULL,
      status TEXT NOT NULL,
      current_period_end BIGINT NOT NULL DEFAULT 0
    );
  `);
  await db.exec(`
    ALTER TABLE sites ADD COLUMN IF NOT EXISTS trial_end BIGINT;
    ALTER TABLE sites ADD COLUMN IF NOT EXISTS grace_until BIGINT;
    ALTER TABLE sites ADD COLUMN IF NOT EXISTS billing_status TEXT NOT NULL DEFAULT 'free';
    ALTER TABLE stripe_events ADD COLUMN IF NOT EXISTS attempts INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE stripe_events ADD COLUMN IF NOT EXISTS last_error TEXT;
    ALTER TABLE stripe_events ADD COLUMN IF NOT EXISTS updated_at BIGINT;
    ALTER TABLE sessions ADD COLUMN IF NOT EXISTS recent_auth_at BIGINT;
  `);
}
