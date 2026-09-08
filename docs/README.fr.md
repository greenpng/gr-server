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
   - *Nginx first-party (recommandé)* : proxifier `/gr.js` + `/gr/dist/v/` vers le
     domaine pv et `/gr/v1/` vers le domaine gv (Cookie passthrough), injecter
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
