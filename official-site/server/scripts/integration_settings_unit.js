import {
  encryptSecret,
  decryptSecret,
  mergeIntegrationPatch,
  publicIntegrationSettings,
  saveIntegrationSettings,
  loadIntegrationSettings,
} from "../src/integration_settings.js";
import { StripeBillingAdapter, MockBillingAdapter } from "../src/billing.js";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

class MemoryDb {
  constructor() { this.row = null; }
  async get(sql) {
    if (sql.includes("FROM admin_settings")) return this.row;
    return null;
  }
  async run(_sql, ...params) {
    this.row = { setting_key: params[0], value_json: params[1], updated_at: params[2], updated_by: params[3], revision: 1 };
    return { changes: 1 };
  }
}

async function main() {
  const key = Buffer.alloc(32, 7);
  const ciphertext = encryptSecret("sk_test_secret", key);
  assert(ciphertext.startsWith("enc:v1:") && !ciphertext.includes("sk_test_secret"), "secret encrypted");
  assert(decryptSecret(ciphertext, key) === "sk_test_secret", "secret decrypts");

  const current = {
    stripe: { secret_key: "sk_old", webhook_secret: "wh_old", price_id: "price_old" },
    gmail: { user: "ops@example.com", app_password: "app_old" },
  };
  const patched = mergeIntegrationPatch(current, {
    stripe: { secret_key: "", price_id: "price_new" },
    gmail: { app_password: "" },
  });
  assert(patched.stripe.secret_key === "sk_old" && patched.gmail.app_password === "app_old", "blank secrets preserve existing");
  assert(patched.stripe.price_id === "price_new", "non-secret patch applies");

  const db = new MemoryDb();
  await saveIntegrationSettings(db, key, {
    stripe: { secret_key: "sk_test", webhook_secret: "wh_test", price_id: "price_test", annual_usd: 99 },
    gmail: { user: "ops@example.com", app_password: "app_test", host: "smtp.gmail.com", port: 465 },
  }, "admin@example.com", 100);
  assert(!db.row.value_json.includes("sk_test") && !db.row.value_json.includes("app_test"), "stored JSON has no plaintext secrets");
  const loaded = await loadIntegrationSettings(db, key, { GV6_DEPLOY_ENV: "lab" });
  assert(loaded.config.stripe.secret_key === "sk_test", "stored Stripe secret loads");
  assert(loaded.config.gmail.app_password === "app_test", "stored Gmail secret loads");
  assert(loaded.adapter instanceof StripeBillingAdapter, "stored Stripe config activates adapter");

  const publicView = publicIntegrationSettings(loaded.config);
  assert(publicView.stripe.configured && publicView.gmail.configured, "public status reports configured");
  assert(!JSON.stringify(publicView).includes("sk_test") && !JSON.stringify(publicView).includes("app_test"), "public view redacts secrets");

  const lab = await loadIntegrationSettings(new MemoryDb(), key, { GV6_DEPLOY_ENV: "lab" });
  assert(lab.adapter instanceof MockBillingAdapter && !lab.gmail, "empty lab uses mock adapters");
  console.log("INTEGRATION_SETTINGS_UNIT_PASS");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
