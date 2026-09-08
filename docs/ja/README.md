# greenpng — Product Guide (日本語)

ブラウザ側での人間性検証とアンチボットインテリジェンス:実際の訪問者のブラウザ内で動作する署名済みプローブ、封印されたインジェストパイプライン、そして 6 言語の SDK を通じてセッション単位の判定を返す分析プレーンを提供します。

> 本ガイドは 12 言語で提供されています。リポジトリルートの
> [言語インデックス](../../README.md#documentation)をご覧ください。

## 1. greenpng が提供するもの

すべての訪問者セッションは、デバイス上およびサーバー側でスコアリングされます:

- **人間 vs ボット判定** — `human | watch | bot` と軸別の信頼度(入力ダイナミクス、デバイススタックの一貫性、環境の真正性、自動化トレース、履歴の再利用)。
- **安定したデバイス識別** — サードパーティ Cookie に依存せず、セッションをまたいで安定する衝突検知型デバイス ID。
- **ビジネスフィールドの取得** — 許可リスト化された Cookie フィールド(`user_id`、`plan_tier` など)をセッション開始時に取得し、判定に付与します。機密性の高い名前(password/token など)はサーバー側でブロックされます。
- **IP インテリジェンス** — 訪問者 IP はインジェスト時点で /24(IPv4)または /48(IPv6)にプライバシーマスクされます。オプションのエンリッチメントで、サブネットを ASN/国/都市にマッピングできます(DB-IP 同梱 MMDB、IPinfo、MaxMind、またはカスタム HTTP エンリッチャー)。シークレットがサーバーの外に出ることはありません。
- **結果取得 API** — 加盟店はサイトごとの SDK キーで判定結果をポーリングします。プロジェクション(`public | sdk | diagnostic`)で公開範囲を制御します。

## 2. 主な機能

| 機能 | 提供される価値 |
|---|---|
| 署名済み FE プローブ | 改ざん検知可能なブラウザパック(ed25519)、バージョン付き不変アセット URL `/dist/v/<ver>/g/<gen>/…` — CDN 安全、キャッシュポイズニングなし |
| 封印済みインジェスト | パック提出は封印され、リプレイ/改ざんは上流で拒否 |
| 分析モジュール(OTA) | identity / brain / analyze / ingest / edge / probe_assets は署名済みモジュールとしてダウンタイムなしでホットアップデート |
| 管理パネル | ランダムパスのスタンドアロンコンソール、単一 scrypt 管理者、監査ログ、サイト/ストラテジ/統合/リテンション/DSAR 管理、EN + 中文 |
| プライバシー・バイ・デフォルト | 最前端のインジェスト地点での IP マスキング、Cookie 許可リスト、DSAR エクスポート/消去、リテンションパージ |
| 6 言語 SDK | `wait_for_result` 向けの JS / Python / Go / PHP / Shell / Rust クライアント |
| マルチノード対応 | 組み込み LB モジュール、クラスタハートビート、OTA ミラー |

## 3. アーキテクチャ

```
            visitor browser
                  │  <script src="/gr.js"> (pinned, no-store)
                  ▼
        FE loader ──► /v1/sdk/bootstrap ──► versioned pack manifest
                  │        (asset_base /dist/v/<fe>/g/<gen>/)
                  ▼
        pack collectors (input, device, environment)
                  │  sealed submit
                  ▼
   ┌──────────────┴───────────────┐
   │ gr-probe-plane (Pingora)      │  gateway / ingest / session APIs
   │  ├─ B8 gateway (TLS SNI)      │  bound-domain direct upload
   │  ├─ sealed ingest + batching  │
   │  └─ /v1/ops/* (token-gated)   │
   └──────────────┬───────────────┘
                  ▼
        gr-service (control plane)
          ├─ admin console + admin API (axum)
          ├─ site / strategy / integration config
          ├─ startup repair: panel policy + site backfill
          └─ module registry (OTA, signed)
                  ▼
        PostgreSQL (sessions, probe_batches, analysis_latest, admin)
                  ▼
        GET /v1/session/{id}/result  ──► merchant SDK (6 languages)
```

**デプロイ形態**

- **gr-service** — 単一プロセス:管理コンソール + 制御 API + ツリー内プローブプレーン。ポート 28680(コンソール、ランダムパス)+ 28765(プレーンのループバック)。
- **ビジネスサイトの nginx** — `/gr.js` を配信し、(オプションで)同一オリジンの API プレフィックスをプロキシします。ブラウザからのアップロードは、バインドされた GV ドメインを Pingora TLS 経由で直接利用しなければなりません。
- **DB** — 制御 + プローブストアに PostgreSQL(ラボ用の SQLite スケルトンも存在)。

## 4. クイックインストール

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

インストーラーは sha256 + ELF + ed25519 モジュール署名を検証し、`/opt/greenpng` 配下にインストールし、`.env` を書き出し、6 つの署名済みモジュールをステージングして有効化し、systemd ユニットを有効にし、`/v1/health` をもって完了ゲートとします。

初回ログイン:ワンタイム認証情報は
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` に書き込まれます — ランダムなコンソールパス(`/c-<hex>/`)はそこに記録されます。`/admin` や `/console/` プレフィックスは存在せず、パスワードログインがパネルへの唯一の入口です。

## 5. 利用チュートリアル

### 5.1 サイトの作成

パネルの **Sites → Create site**:

- `site_id` — テナント ID。埋め込みコードで使用されます
- `root_domains` — www ホスト名(CORS + ホスト名バインディング)
- `cookie_fields` — Cookie の許可リスト。例:
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

サイト行は保存時に `control.sites → public.sites`(プローブプレーン)へ流れ、gr-service は起動時に既存サイトをバックフィルします。

### 5.2 プローブのデプロイ(3 つのモード)

**A. Nginx ファーストパーティ(推奨)** — サイトの vhost がブートローダーと同一オリジン API プレフィックスをプロキシします:

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Cloudflare Worker** — ワーカーがブートスクリプトを注入し、`/gr` をオリジン内でプロキシします。`/gr` では "Under Attack" モードを避けてください(チャレンジページがプローブを壊します)。

**C. サイトスクリプティング / CDN 埋め込み** — ブート JS を PV から直接読み込み、`data-endpoint` を GV に向けます。

どのモードでも、ローダーは SDK ブートストラップを通じてパックを解決し、バージョン付きの不変 URL のみを取得します。

### 5.3 結果の受信(6 言語 SDK)

パネルの SDK ページで、サイトの**バックエンドキー**を作成します。SDK はプローブを中継しません。結果のみを照会します:

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| 言語 | エントリポイント |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

サイトキーはスコープされます:サイト A 用に発行されたキーはサイト B のセッションを読めず(`403 sdk key site mismatch`)、失効したキーは即座に動作しなくなります(`401`)。

### 5.4 エンドツーエンドのスモークテスト(curl)

```bash
OPEN=$(curl -fsS -X POST https://gv.example.com/v1/session/open \
  -H 'Content-Type: application/json' \
  -H 'Cookie: user_id=u9; plan_tier=pro' \
  -d '{"site_id":"mysite","visitor_terminal_id":"vt_demo1"}')
SID=$(printf '%s' "$OPEN" | jq -r .session_id)

curl -fsS -X POST "https://gv.example.com/v1/session/$SID/analyze" \
  -H "X-Gr-Sdk-Key: $GR_SITE_RESULT_KEY" -H 'Content-Type: application/json' -d '{}'

curl -fsS "https://gv.example.com/v1/session/$SID/result?projection=sdk" \
  -H "X-Gr-Sdk-Key: $GR_SITE_RESULT_KEY" | jq '.sdk_projection'
```

### 5.5 アップデートの維持

| チャネル | コマンド |
|---|---|
| パネル OTA(デフォルト) | Admin panel → Modules / Runtime install |
| アップデータスクリプト | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| 手動 | SSH + 旧ランタイムへのロールバック(`bin/releases/<v>` は保持) |

どのアップデートも、本リポジトリの同じ署名済み Release アセットを取得します。

## 6. リポジトリ構成

| ディレクトリ | 内容 |
|---|---|
| `crates/` | Rust ワークスペース — gr-service, gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | 署名済みホットアップデートモジュールのソース(identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | ブラウザプローブ FE(ローダー、パック、封印済みインジェスト) |
| `panel/` | 管理パネル — Vue ソース(`admin-ui/`)+ ビルド済み SPA(`admin-spa/`) |
| `sdk/` | 結果取得 SDK(JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | ランタイム読み込みスペック(ボット重み、カタログ) |
| `install/` | インストーラー + アップデータ + systemd 素材 |
| `release/` | パッケージングスクリプト(マルチアーキバンドル、SLSA アテステーション) |
| `scripts/` | FE 契約チェックとヘルパー |
| `docs/` | 本ガイドの 12 言語版 |
| `VERSION` | リリースバージョンの唯一の信頼できる情報源 |

## 7. リンク

- Releases とインストールの入口:本リポジトリ
- 公式サイト:https://www.greenpng.cc(製品紹介、EN + 中文)
- パネルのロケール:English + 中文(`panel/admin-ui/src/i18n/` で同期維持)
