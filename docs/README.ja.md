# greenpng — プロジェクトガイド（日本語）

## 1. 本プロジェクトについて

greenpng (GR) は、セッションが本物の人間であるかどうかを、トラフィックの
検査だけではなく、**実際の訪問者のブラウザ**を探査（プローブ）することで
判定します。

エンドツーエンドのパイプライン：

1. **プローブ（ブラウザ内）** — 署名済み FE ローダーがあなたのページ上で
   実行され、セッションごとに**複数のデータソース**（入力ダイナミクス、
   デバイススタック、環境の真正性、自動化の痕跡）から**複数の段階的
   バッチ**で証拠を収集します。
2. **アップロード** — ブラウザは各バッチを封印されたインジェスト
   パイプライン経由でサーバーに送信します。リプレイや改竄された
   送信は、ストレージに到達する前に拒否されます。
3. **分析（サーバー側）** — 分析プレーンは各セッションを評価し、
   軸ごとの信頼度つきのセッション単位の判定
   （`human | watch | bot`）と、安定した衝突耐性のあるデバイス識別子を
   出力します。
4. **返却** — あなたのバックエンドは結果 API 経由で判定を取得します
   （6 言語の SDK。`public | sdk | diagnostic` のプロジェクションにより
   各呼び出し元に見える範囲を制御）。

サーバー側では、greenpng はデフォルトで署名済みプローブプレーンを内蔵した
単一のホストバイナリとして動作し、**マルチノードのロードバランス
デプロイ**もサポートします。LB モジュールが共有データ層
（PostgreSQL + Redis）を前置きした複数ノードにプローブトラフィックを
分散するため、収集と分析は水平にスケールします。

## 2. リポジトリ構成とアーキテクチャ

| ディレクトリ | 内容 |
|---|---|
| `crates/` | Rust ワークスペース — `gr-service`（コントロールプレーン + 管理コンソール + 内蔵プローブプレーン）、`gr-probe-core`、`gr-probe-plane`、`gr-probe-store`、`gr-ota`、`gr-admin`、`gr-runtime` など |
| `modules/` | 署名済みホットアップデートモジュールのソース（identity / brain / analyze / ingest / edge / probe_assets） |
| `probe/` | ブラウザプローブ FE（ローダー、パックチェーン、封印インジェストクライアント） |
| `panel/` | 管理パネル — Vue ソース（`admin-ui/`）+ ビルド済み SPA（`admin-spa/`）、英語 + 中国語 |
| `sdk/` | 6 言語のバックエンド統合 SDK（結果取得のみ） |
| `spec/` | 実行時にロードされるワイヤ/スコアリング仕様 |
| `fixtures/` | コントラクトテストデータ |
| `scripts/` | ビルドスクリプトと FE ツール（`scripts/fe/checks/`） |
| `vendor/` | vendored 依存ソース（pingora） |
| `install/` | インストーラー、データ層 compose、アップグレードスクリプト |
| `release/` | パッケージングスクリプト（マルチアーキビルド、SBOM、モジュール署名） |
| `docs/` | 本ガイド、言語ごとに 1 ファイル |

```
        visitor browser (訪問者ブラウザ)
              │  <script src="/gr.js">  (pinned, no-store)
              ▼
   FE loader ──► /v1/sdk/bootstrap ──► バージョン付きパックマニフェスト
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   パックコレクター（入力 · デバイス · 環境、複数バッチ）
              │  sealed submit（封印送信: バインド済み gv ドメインへ直通, TLS）
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service (コントロールプレーン) │
 │  gateway · ingest · ops   │◄──┤  管理コンソール · サイト設定       │
 └────────────┬─────────────┘   │  モジュールレジストリ（OTA, 署名） │
              ▼                 └──────────────┬───────────────┘
   PostgreSQL（マルチノード時は + Redis）      │
              ▼                                │
   GET /v1/session/{id}/result ──► 加盟店 SDK（6 言語）

   マルチノード: LB モジュールが共有データ層を前置きした
   複数の gr-service ノードにプローブトラフィックを分散
```

主要コンポーネント: `gr-service` はコントロールプレーン（ランダムパスの
管理コンソール、サイト/設定管理、署名済み OTA モジュールレジストリ）であり、
プローブプレーンを内蔵します。`gr-probe-plane` は封印インジェストと
セッション/結果 API を持つ Pingora ゲートウェイです。ブラウザ FE は
すべてのアセットをバージョン不変の URL に解決するため、キャッシュが
リリースをまたいで古いプローブを供給することは決してありません。
PostgreSQL（マルチノード時は Redis も）がセッション、バッチ、
分析結果を保存します。

## 3. インストールと使用方法

### 3.1 インストール

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# またはバージョン / アーキテクチャを明示指定:
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# データ層を docker 化（PostgreSQL/Redis のみ。サーバー本体はホストバイナリのまま）:
bash install/install.sh --version <VERSION> --with-docker --yes
```

インストーラーは sha256 + ELF + ed25519 モジュール署名を検証し、
`/opt/greenpng` 配下にインストールし、`.env` と systemd ユニットを書き
出し、6 つの署名済みモジュールをステージングして有効化し、
コントロール/プローブプレーンの `/v1/health` をゲート条件とします。
初回ログイン資格情報とランダムなコンソールパスは
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` に書き込まれます。

### 3.2 アップデート

| 優先度 | チャネル | 適用 |
|---|---|---|
| P0 | パネル OTA（set-release-url → install / install-fe / install-runtime） | デフォルト |
| P1 | `install/release/update_runtime_from_github.sh`、`update_module_from_github.sh` | パネルなし / 無料ノード |
| P2 | SSH 手動 | プロセス停止 / 初回インストール |

すべてのアップデートは、このリポジトリの同じタグの署名済み Release
アセットを取得します。
Docker はランタイムコンテナであり、アップデートチャネルではありません。

### 3.3 使用方法

1. 管理パネルで**サイトを作成**: サイト ID、ルートドメイン、および各判定に
   添付したい業務フィールドの Cookie 許可リスト（`password`/`token` などの
   機密名はサーバー側で強制ブロックされます）。
2. **プローブをデプロイ** — 3 つのモード:
   - *Nginx ファーストパーティ（推奨）*: pv ドメイン上で `/gr.js` + `/gr/dist/v/` +
     `/gr/v1/` をプローブプレーンへプロキシ（Cookie パススルー）し、
     HTML に `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>` を注入。
   - *Cloudflare Worker*: 同じタグを注入し、`/gr` をオリジン内でプロキシ。
   - *アプリ埋め込み*: pv/CDN からローダーを直接ロードし、
     `data-endpoint` を gv/pv に向ける。
   ブラウザからのアップロードは**常にバインド済み gv ドメインへ TLS で
   直送**されます。
3. **結果を受信** — パネルでサイト SDK キーを作成し、ポーリングします:

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **スモークテスト**（curl）:

```bash
# 許可リストの cookie を携えてセッションをオープン
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# （実際の FE バッチまたはシミュレーション後の）結果を確認:
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

---

## 4. 管理パネルのパラメータ

パネルの **Config** ページで編集します（保存 → 公開）。公開したノードは
即時に適用、クラスタの各ノードは ≤30 秒、再起動不要です。

**レート制限**（v1.0.14 のポリシー：サイト合計はデフォルトで無効。代わりに
テレメトリ経路は単一 IP ごとに制限。429 応答は発動したレイヤーを明示します）：

| パラメータ | デフォルト | 意味 |
|---|:---:|---|
| `rate_limit_open_per_min` | 0 = 無制限 | セッション open / サイト / 分 |
| `rate_limit_ingest_per_min` | 0 = 無制限 | バッチ Upload / サイト / 分 |
| `rate_limit_analyze_per_min` | 0 = 無制限 | 直接 analyze / サイト / 分 |
| `rate_limit_complete_per_min` | 0 = 無制限 | complete 受領 / サイト / 分 |
| `rate_limit_result_per_min` | 0 = 無制限 | 結果 Read / サイト / 分 |
| `rate_limit_client_event_per_min` | 0 = 無制限 | FE テレメトリ / サイト / 分（合計） |
| `rate_limit_client_event_per_ip_per_min` | 100 | FE テレメトリ **単一 IP あたり** / 分 — 超過時はその IP のみ制限；0 = 無効 |

**CDN 背後**では、per-IP レイヤーはサーバーが見る IP を鍵とします。プロキシ
の CIDR を `/opt/greenpng/.env` の `GR_TRUSTED_PROXIES` に追加し、前段
プロキシで実際の訪問者 IP を復元してください（nginx 例）：

```nginx
set_real_ip_from 173.245.48.0/20;  # Cloudflare IPv4
set_real_ip_from 2400:cb00::/32;   # Cloudflare IPv6
real_ip_header CF-Connecting-IP;
```

**ホット/コールド階層**（実際のノブ名）：`cold_ttl_ms`（604800000 = 7 日）、
`cold_promote_window_ms`（864000000 = 24 時間）、`cold_purge_interval_ms`
（300000 = 5 分）。サイトごとの保持期間はパネルの **Data Retention** ページ
で設定し、限定バッチで削除されます。

## 5. ログとメモリ

- サービスログ：`journalctl -u greenpng.service`。運用テレメトリ
  （`ops_client_events`）は `ops_retention_days`（14）日保持されます。
- **メモリに関する注記（v1.0.14 以降）**：長時間稼働するマルチコアホスト
  では glibc がコアあたり最大 8 アリーナ（各 ~64 MB）を保持でき、並行
  負荷下で RSS が階段状に増加することがあります。インストーラは既定で
  `/opt/greenpng/.env` に `MALLOC_ARENA_MAX=4` を設定します。負荷下でも
  RSS は平坦に保たれます。

## 6. オープンソース・プロジェクトと参考文献

- [Cloudflare Pingora](https://github.com/cloudflare/pingora) (Apache-2.0) — エッジゲートウェイ、TLS 終端、シールド済み ingest（openssl feature）。
- [Tokio](https://github.com/tokio-rs/tokio) & [Axum](https://github.com/tokio-rs/axum) (MIT) — コントロールプレーンの非同期ランタイムと REST フレームワーク。
- [OpenSSL](https://www.openssl.org/) (Apache-2.0) — エッジと管理コンソールの TLS バックエンド。
- [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) (BSD-3) — リリースマニフェスト、プローブアセット、OTA の署名。
- [PostgreSQL](https://www.postgresql.org/) & [Redis](https://redis.io/) — L3 ストレージとマルチノード状態。
- [flate2 / zlib](https://github.com/rust-compress/flate2) (MIT) — ペイロード圧縮（zstd は vendored pingora 内のみ）。
- [Element Plus](https://element-plus.org/) & [Vue 3](https://vuejs.org/) (MIT) — 管理コンソール UI。
- 研究参考文献：[CreepJS](https://github.com/abrahamjuliot/creepjs)（B1/B12 の着想）、[FingerprintJS](https://github.com/fingerprintjs/fingerprintjs)、[BotD](https://github.com/fingerprintjs/botd)。
