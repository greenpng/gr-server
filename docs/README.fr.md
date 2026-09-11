# greenpng — Guide du projet (Français)

## 1. Ce qu'est ce projet

greenpng (GR) vérifie si une session correspond à un vrai humain en sondant le
**navigateur du visiteur réel**, et non en inspectant uniquement le trafic.

Le pipeline, de bout en bout :

1. **Sondage (dans le navigateur)** — un loader FE signé s'exécute sur vos pages et
   collecte des preuves depuis **plusieurs sources** (dynamique des saisies, pile matérielle,
   authenticité de l'environnement, traces d'automatisation) en **lots successifs multiples**
   par session.
2. **Upload** — le navigateur soumet chaque lot au serveur via un
   pipeline d'ingest scellé ; les soumissions rejouées ou falsifiées sont rejetées
   avant d'atteindre le stockage.
3. **Analyse (côté serveur)** — le plan d'analyse note chaque session en un
   verdict par session (`human | watch | bot`) avec une confiance par axe et
   une identité de périphérique stable et consciente des collisions.
4. **Retour** — votre backend récupère le résultat via l'API de résultats
   (SDK dans six langues ; les projections `public | sdk | diagnostic` contrôlent ce que
   chaque appelant voit).

Côté serveur, greenpng s'exécute comme un unique binaire hôte avec un plan de sondage
intégré (in-tree) par défaut, et prend en charge un **déploiement multi-nœuds avec répartition de charge** :
un module LB répartit le trafic de sondage entre les nœuds, devant une couche de données partagée
(PostgreSQL + Redis), de sorte que la collecte et l'analyse passent à l'échelle horizontalement.

## 2. Structure du dépôt et architecture

| Répertoire | Contenu |
|---|---|
| `crates/` | Workspace Rust — `gr-service` (plan de contrôle + console d'administration + plan de sondage intégré), `gr-probe-core`, `gr-probe-plane`, `gr-probe-store`, `gr-ota`, `gr-admin`, `gr-runtime`, … |
| `modules/` | Sources des modules de mise à jour à chaud signés (identity / brain / analyze / ingest / edge / probe_assets) |
| `probe/` | FE de sondage navigateur (loader, chaîne de packs, client d'ingest scellé) |
| `panel/` | Panneau d'administration — sources Vue (`admin-ui/`) + SPA compilé (`admin-spa/`), EN + 中文 |
| `sdk/` | SDK d'intégration backend en six langages (récupération des résultats uniquement) |
| `spec/` | Spécifications de protocole et de scoring chargées à l'exécution |
| `fixtures/` | Données de tests de contrat |
| `scripts/` | Scripts de build et outillage FE (`scripts/fe/checks/`) |
| `vendor/` | Sources de dépendances vendorisées (pingora) |
| `install/` | Installateur, compose de la couche de données, scripts de mise à niveau |
| `release/` | Scripts de packaging (build multi-arch, SBOM, signature des modules) |
| `docs/` | Ce guide, un fichier par langue |

```
        navigateur du visiteur
              │  <script src="/gr.js">  (pinned, no-store)
              ▼
   FE loader ──► /v1/sdk/bootstrap ──► manifest de packs versionné
              │        asset_base /dist/v/<fe>/g/<gen>/
              ▼
   collecteurs de packs (saisie · périphérique · environnement, multi-lots)
              │  soumission scellée (directe vers le domaine gv lié, TLS)
              ▼
 ┌────────────┴─────────────┐   ┌──────────────────────────────┐
 │ gr-probe-plane (Pingora)  │   │ gr-service (plan de contrôle) │
 │  passerelle · ingest · ops│◄──┤  console admin · config sites │
 └────────────┬─────────────┘   │  registre de modules (OTA, signé)│
              ▼                 └──────────────┬───────────────┘
   PostgreSQL (+ Redis en multi-nœuds)          │
              ▼                                │
   GET /v1/session/{id}/result ──► SDK marchand (six langages)

   multi-nœuds : le module LB répartit le trafic de sondage entre les nœuds gr-service
   devant la couche de données partagée
```

Composants clés : `gr-service` est le plan de contrôle (console d'administration à
chemin aléatoire, gestion des sites et de la configuration, registre de modules OTA
signés) et héberge le plan de sondage intégré ; `gr-probe-plane` est la passerelle
Pingora avec l'ingest scellé et les API session/résultat ; le FE navigateur résout
chaque asset vers une URL à version immuable afin que les caches ne servent jamais
un sondage périmé entre les releases ; PostgreSQL (plus Redis en multi-nœuds) stocke
les sessions, les lots et les résultats d'analyse.

## 3. Installation et utilisation

### 3.1 Installer

```bash
curl -fsSL https://raw.githubusercontent.com/greenpng/gr-server/main/install/install.sh | bash
# ou avec une version / arch explicite :
bash install/install.sh --version <VERSION> --arch x86_64 --yes
# avec une couche de données dockerisée (PostgreSQL/Redis uniquement ; le serveur reste un binaire hôte) :
bash install/install.sh --version <VERSION> --with-docker --yes
```

L'installateur vérifie les signatures sha256 + ELF + ed25519 des modules, installe
sous `/opt/greenpng`, écrit le `.env` et l'unité systemd, prépare puis
active les six modules signés, et vérifie le `/v1/health` du contrôle et du plan.
Les identifiants de première connexion et le chemin aléatoire de la console sont
écrits dans `/opt/greenpng/data/admin/admin_bootstrap_once.txt`.

### 3.2 Mettre à jour

| Priorité | Canal | Pour |
|---|---|
| P0 | OTA via le panneau (set-release-url → install / install-fe / install-runtime) | défaut |
| P1 | `install/release/update_runtime_from_github.sh`, `update_module_from_github.sh` | sans panneau / nœuds gratuits |
| P2 | SSH manuel | processus mort / première installation |

Toutes les mises à jour tirent les assets signés du même tag de la Release depuis ce
dépôt. Docker est un conteneur d'exécution, pas un canal de mise à jour.

### 3.3 Utiliser

1. **Créer un site** dans le panneau d'administration : identifiant du site, domaines racine, et
   la liste de cookies autorisés pour les champs métier que vous voulez attacher à chaque verdict
   (les noms sensibles comme `password`/`token` sont bloqués côté serveur).
2. **Déployer le sondage** — trois modes :
   - *Nginx first-party (recommandé)* : sur le domaine pv, proxifier `/gr.js` +
     `/gr/dist/v/` + `/gr/v1/` vers le plan de sonde (Cookie passthrough), injecter
     `<script src="/gr.js" data-site-id="…" data-endpoint="/gr"
     data-inject-path="nginx" defer></script>` dans le HTML.
   - *Worker Cloudflare* : injecter le même tag et proxifier `/gr` in-origin.
   - *Intégration applicative* : charger le loader directement depuis pv/CDN avec
     `data-endpoint` pointant vers gv/pv.
   Les uploads du navigateur vont toujours **directement vers le domaine gv lié en TLS**.
3. **Recevoir les résultats** — créer une clé SDK de site dans le panneau, puis interroger :

```bash
KEY="grsk_..." SID="cycle_..." BASE="https://gv.example.com"
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

4. **Test de fumée** (curl) :

```bash
# ouvrir une session portant les cookies autorisés
curl -sS -X POST "$BASE/v1/session/open" -H "X-Gr-Sdk-Key: $KEY" \
     -H 'Cookie: user_id=u9; plan_tier=pro' -d '{"site_id":"mysite"}'
# → {"session_id":"cycle_…", …}
# puis vérifier le résultat (après de vrais lots FE ou des lots simulés) :
curl -sS -H "X-Gr-Sdk-Key: $KEY" "$BASE/v1/session/$SID/result"
```

---

## 4. Paramètres du panneau d'administration

S'éditent sur la page **Config** du panneau (enregistrer → publier) : le
nœud qui publie applique immédiatement ; les nœuds du cluster en ≤30 s,
sans redémarrage.

**Limites de débit** (politique v1.0.14 : les totaux par site sont
désactivés par défaut ; la télémétrie est bornée par IP individuelle ; les
réponses 429 nomment la couche déclenchée) :

| Paramètre | Défaut | Signification |
|---|:---:|---|
| `rate_limit_open_per_min` | 0 = illimité | ouvertures de session / site / min |
| `rate_limit_ingest_per_min` | 0 = illimité | envois de lots / site / min |
| `rate_limit_analyze_per_min` | 0 = illimité | analyses directes / site / min |
| `rate_limit_complete_per_min` | 0 = illimité | accusés complete / site / min |
| `rate_limit_result_per_min` | 0 = illimité | lectures de résultat / site / min |
| `rate_limit_client_event_per_min` | 0 = illimité | télémétrie FE / site / min (total) |
| `rate_limit_client_event_per_ip_per_min` | 100 | télémétrie FE **par IP individuelle** / min — le dépassement ne borne que cette IP ; 0 = désactivé |

**Derrière un CDN**, la couche par IP s'appuie sur l'IP vue par le serveur.
Ajoutez les CIDR du proxy à `GR_TRUSTED_PROXIES` dans `/opt/greenpng/.env`
et restaurez la vraie IP du visiteur sur le proxy frontal (exemple nginx) :

```nginx
set_real_ip_from 173.245.48.0/20;  # Cloudflare IPv4
set_real_ip_from 2400:cb00::/32;   # Cloudflare IPv6
real_ip_header CF-Connecting-IP;
```

**Répartition chaud/froid** (vrais noms de réglages) : `cold_ttl_ms`
(604800000 = 7 jours), `cold_promote_window_ms` (864000000 = 24 h),
`cold_purge_interval_ms` (300000 = 5 min) ; la rétention par site se règle
sur la page **Data Retention** du panneau et se purge par lots bornés.

## 5. Journalisation et mémoire

- Journaux du service : `journalctl -u greenpng.service` ; la télémétrie
  opérationnelle (`ops_client_events`) est conservée `ops_retention_days`
  (14) jours.
- **Note mémoire (dès v1.0.14)** : sur des hôtes multicœurs à longue durée
  de vie, glibc peut garder jusqu'à 8 arènes par cœur (~64 Mo chacune), si
  bien que le RSS peut grimper par paliers sous concurrence. L'installateur
  fixe donc `MALLOC_ARENA_MAX=4` dans `/opt/greenpng/.env` ; le RSS reste
  stable en charge.

## 6. Projets open source et références

- [Cloudflare Pingora](https://github.com/cloudflare/pingora) (Apache-2.0) — passerelle de bord, terminaison TLS, ingest scellé (feature openssl).
- [Tokio](https://github.com/tokio-rs/tokio) & [Axum](https://github.com/tokio-rs/axum) (MIT) — runtime asynchrone et framework REST du plan de contrôle.
- [OpenSSL](https://www.openssl.org/) (Apache-2.0) — backend TLS du bord et de la console d'administration.
- [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) (BSD-3) — signatures des manifestes, actifs de sonde et OTA.
- [PostgreSQL](https://www.postgresql.org/) & [Redis](https://redis.io/) — stockage L3 et état multi-nœuds.
- [flate2 / zlib](https://github.com/rust-compress/flate2) (MIT) — compression des payloads (zstd seulement dans le pingora vendored).
- [Element Plus](https://element-plus.org/) & [Vue 3](https://vuejs.org/) (MIT) — UI de la console.
- Références de recherche : [CreepJS](https://github.com/abrahamjuliot/creepjs) (inspiration B1/B12), [FingerprintJS](https://github.com/fingerprintjs/fingerprintjs), [BotD](https://github.com/fingerprintjs/botd).
