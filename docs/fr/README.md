# greenpng — Product Guide (Français)

Vérification humaine côté navigateur et intelligence anti-bot : une
sonde signée qui s'exécute dans les navigateurs de vrais visiteurs, un
pipeline d'ingestion scellé, et un plan d'analyse qui renvoie des
verdicts par session via des SDK en six langues.

> Ce guide est disponible en 12 langues — consultez
> [l'index des langues](../../README.md#documentation) à la racine du
> dépôt.

## 1. Ce que fait greenpng

Chaque session de visiteur est évaluée sur l'appareil et côté serveur :

- **Verdict humain ou robot** — `human | watch | bot` avec un niveau de
  confiance par axe (dynamique des saisies, cohérence de la pile
  matérielle, authenticité de l'environnement, traces d'automatisation,
  réutilisation d'historique).
- **Identité d'appareil stable** — un identifiant d'appareil résistant aux
  collisions qui reste stable d'une session à l'autre sans s'appuyer sur
  des cookies tiers.
- **Capture des champs métier** — les champs de cookies que vous avez
  placés en liste d'autorisation (`user_id`, `plan_tier`, …) sont
  capturés à l'ouverture de la session et rattachés au verdict. Les noms
  sensibles (password/token/…) sont bloqués côté serveur.
- **Renseignement IP** — les adresses IP des visiteurs sont masquées pour
  la vie privée en /24 (IPv4) ou /48 (IPv6) dès l'ingestion ; un
  enrichissement optionnel associe le sous-réseau à ASN/pays/ville via
  DB-IP (MMDB fourni), IPinfo, MaxMind ou un enrichisseur HTTP
  personnalisé. Les secrets ne quittent jamais le serveur.
- **API de récupération des résultats** — les marchands interrogent le
  verdict avec une clé SDK propre à chaque site ; les projections
  (`public | sdk | diagnostic`) contrôlent l'exposition des données.

## 2. Fonctionnalités clés

| Fonctionnalité | Ce qu'elle vous apporte |
|---|---|
| Sonde FE signée | Paquets navigateur infalsifiables (ed25519), URL d'actifs versionnées et immuables `/dist/v/<ver>/g/<gen>/…` — compatibles CDN, sans empoisonnement de cache |
| Ingestion scellée | Les soumissions de paquets sont scellées ; rejeu et falsification rejetés en amont |
| Modules d'analyse (OTA) | identity / brain / analyze / ingest / edge / probe_assets se mettent à jour à chaud comme modules signés sans interruption de service |
| Panneau d'administration | Console autonome sur un chemin aléatoire, administrateur unique scrypt, journal d'audit, gestion sites/stratégies/intégrations/rétention/DSAR, EN + 中文 |
| Confidentialité par défaut | Masquage des IP au plus tôt dans l'ingestion, liste d'autorisation des cookies, export/effacement DSAR, purge de rétention |
| SDK en six langues | Clients JS / Python / Go / PHP / Shell / Rust pour `wait_for_result` |
| Prêt multi-nœuds | Module LB intégré, battement de cœur du cluster, miroir OTA |

## 3. Architecture

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

**Topologies de déploiement**

- **gr-service** — un seul processus : console d'administration + API de
  contrôle + plan de sonde intégré. Port 28680 (console, chemin aléatoire)
  + 28765 (plan en loopback).
- **Nginx du site métier** — sert `/gr.js` et (optionnellement) proxifie
  le préfixe d'API same-origin ; les envois depuis le navigateur doivent
  passer par le domaine GV lié via Pingora TLS.
- **Bases de données** — PostgreSQL pour les magasins de contrôle et de
  sondes (un squelette SQLite existe pour le laboratoire).

## 4. Installation rapide

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh
bash install.sh --version <VERSION>            # see the VERSION file / Releases
# or with a dockerized data layer:
bash install.sh --version <VERSION> --with-docker --yes
```

L'installateur vérifie les signatures sha256 + ELF + ed25519 des modules,
installe sous `/opt/greenpng`, écrit `.env`, précharge et active les six
modules signés, active l'unité systemd et s'appuie sur `/v1/health`
comme validation finale.

Première connexion : des identifiants à usage unique sont écrits dans
`/opt/greenpng/data/admin/admin_bootstrap_once.txt` — le chemin
aléatoire de la console (`/c-<hex>/`) y est consigné. Il n'existe pas de
préfixe `/admin` ou `/console/`, et la connexion par mot de passe est la
seule entrée du panneau.

## 5. Tutoriel d'utilisation

### 5.1 Créer un site

Panneau **Sites → Create site** :

- `site_id` — votre identifiant de tenant, utilisé dans l'intégration
- `root_domains` — les noms d'hôte www (CORS + liaison de hostname)
- `cookie_fields` — la liste d'autorisation des cookies, p. ex.
  `["user_id", "plan_tier", "cart_id", "utm_source"]`

La ligne du site est propagée de `control.sites` vers `public.sites`
(plan de sondes) à l'enregistrement, et gr-service complète les sites
préexistants au démarrage.

### 5.2 Déployer la sonde (trois modes)

**A. Nginx first-party (recommandé)** — le vhost de votre site proxifie
le chargeur d'amorçage et le préfixe d'API same-origin :

```nginx
location = /gr.js      { proxy_pass https://pv.example.com/gr.js; proxy_set_header Host pv.example.com; }
location /gr/dist/v/   { proxy_pass https://pv.example.com/gr/dist/v/; proxy_set_header Host pv.example.com; }
location /gr/v1/       { proxy_pass https://gv.example.com/; proxy_set_header Cookie $http_cookie;
                         proxy_set_header X-Forwarded-For $remote_addr; }
```

```html
<script src="/gr.js" data-site-id="mysite" data-endpoint="/gr" data-inject-path="nginx" defer></script>
```

**B. Worker Cloudflare** — le worker injecte le script d'amorçage et
proxifie `/gr` dans l'origine. Évitez le mode « Under Attack » sur `/gr`
(les pages de défi cassent la sonde).

**C. Script de site / intégration CDN** — chargez le JS d'amorçage
directement depuis PV et pointez `data-endpoint` vers GV.

Dans tous les modes, le chargeur résout les paquets via l'amorçage SDK
et ne récupère que des URL versionnées et immuables.

### 5.3 Recevoir les résultats (SDK en six langues)

Créez une **clé backend** pour votre site dans le panneau (page SDK). Le
SDK ne relaie jamais les sondes ; il interroge uniquement les
résultats :

```
GET {gv_base}/v1/session/{session_id}/result?projection=sdk
X-Gr-Sdk-Key: <site backend key>
```

| Langage | Point d'entrée |
|---|---|
| JS | `sdk/src/index.js` — `new GrResultClient({baseUrl, apiKey}).waitForResult(...)` |
| Python | `sdk/python/gr_results.py` — `GrResultClient(base_url, api_key).wait_for_result(...)` |
| Go | `sdk/go/gr_results.go` — `gr.New(baseURL, key).WaitForResult(...)` |
| PHP | `sdk/php/GrResultsClient.php` — `(new GrResultClient(...))->waitForResult(...)` |
| Shell | `sdk/shell/gr_sdk.sh` — `gr_wait_for_result <session> sdk 8000` |
| Rust | `sdk/rust/` — `Client::new(base_url, key).wait_for_result(...)` |

Les clés de site sont à périmètre restreint : une clé émise pour le site
A ne peut pas lire les sessions du site B (`403 sdk key site
mismatch`), et les clés révoquées cessent immédiatement de fonctionner
(`401`).

### 5.4 Test de bout en bout (curl)

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

### 5.5 Rester à jour

| Canal | Commande |
|---|---|
| OTA via panneau (défaut) | Panneau d'administration → Modules / Runtime install |
| Script de mise à jour | `VERSION=<v> INSTALL_ROOT=/opt/greenpng bash install/release/update_runtime_from_github.sh` |
| Manuel | SSH + retour arrière vers le runtime précédent (`bin/releases/<v>` conservé) |

Chaque mise à jour récupère les mêmes actifs signés de la Release depuis
ce dépôt.

## 6. Organisation du dépôt

| Répertoire | Contenu |
|---|---|
| `crates/` | Espace de travail Rust — gr-service, gr-probe-core, gr-probe-plane, gr-probe-store, gr-ota, gr-admin, gr-runtime, … |
| `modules/` | Sources des modules de mise à jour à chaud signés (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | Sonde navigateur FE (chargeur, paquets, ingestion scellée) |
| `panel/` | Panneau d'administration — sources Vue (`admin-ui/`) + SPA compilée (`admin-spa/`) |
| `sdk/` | SDK de récupération des résultats (JS / Python / Go / PHP / Shell / Rust) |
| `spec/` | Spécifications chargées à l'exécution (poids des bots, catalogues) |
| `install/` | Installateur + script de mise à jour + éléments systemd |
| `release/` | Scripts d'empaquetage (bundle multi-architecture, attestation SLSA) |
| `scripts/` | Contrôles de contrat FE et utilitaires |
| `docs/` | Ce guide en 12 langues |
| `VERSION` | Source unique de vérité pour la version de la release |

## 7. Liens

- Releases et point d'entrée d'installation : ce dépôt
- Site officiel : https://www.greenpng.cc (présentation du produit, EN + 中文)
- Langues du panneau : anglais + 中文 (maintenues synchronisées dans `panel/admin-ui/src/i18n/`)
