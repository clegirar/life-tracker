# Life tracker

App auto-hébergée sur un NAS Synology qui retrace ma vie jour par jour, avec un calendrier web et un serveur MCP pour Claude. Le POC (compteur de cafés et musique) est validé depuis le 4 nov. 2026 ; on construit le premier tracker après le POC : Garmin Connect.

## Où trouver quoi

- `docs/poc.md` : les étapes du POC et leurs critères. On travaille une étape à la fois, dans l'ordre ; une étape n'est finie que si tous ses critères « Terminé quand » sont remplis.
- `docs/architecture.md` : les principes, le contrat de l'API d'ingestion et le journal des décisions. À lire avant toute décision de conception.
- `docs/database.md` : le schéma et ses raisons.
- `TODO.md` : les reprises notées pour plus tard, à ne traiter que sur demande.

## Périmètre

En cours : Garmin Connect (section « Après le POC » de `docs/poc.md`). Hors champ tant que je ne les demande pas : les messages et l'agent Mac, l'annuaire côté interface, les autres trackers, les « fonctionnalités futures » de l'architecture, les sauvegardes, l'ouverture de la web app sur Internet, le contenu de `TODO.md`. Ne rien construire pour eux par anticipation ; le design des messages reste dans l'architecture pour plus tard.

## Commandes

Seul ce qui existe aujourd'hui est listé ; ajouter ici les commandes de chaque nouveau composant.

- Tout le dépôt : `./test.sh` (script de déploiement, base, qualité et tests du backend, web app, serveur MCP ; conteneurs jetables, Docker requis).
- Scripts de déploiement : `deploy/test.sh` (`deploy.sh` avec un faux `docker`, sans Docker ; `next-version.sh` sur un dépôt git jetable ; shellcheck s'il est installé).
- CI : `.github/workflows/ci.yml`, sur chaque PR et chaque push sur `main` hors documentation, lance en parallèle `db/test/run.sh`, le format, clippy et `backend/test.sh`, `web/test.sh`, `mcp/test.sh` et `deploy/test.sh`. `pr-title.yml` contrôle le titre de la PR. Workflows vérifiés par `actionlint`.
- Tests de la base : `db/test/run.sh` (conteneurs jetables, Docker requis).
- Base de développement : `cp .env.example .env && docker compose -f compose.dev.yaml up -d` (port 54329).
- Tests du backend : `backend/test.sh [arguments]` (conteneur jetable, Docker requis ; Rust figé par `backend/rust-toolchain.toml`). Par `cargo nextest run` si cargo-nextest est installé (comme en CI, 0.9.150), sinon `cargo test` : les arguments suivent l'outil. Les tests les plus longs partent en premier (`backend/.config/nextest.toml`).
- Après un changement de requête SQL : `backend/test.sh --prepare` régénère le cache `backend/.sqlx/` (sqlx-cli 0.9 requis), à versionner.
- Qualité : dans `backend/`, `cargo fmt --check` et `cargo clippy --all-targets -- -D warnings`.
- Web app : `web/test.sh [arguments de playwright test]` lance lint, types, tests unitaires, build, puis Playwright contre le vrai backend et un PostgreSQL 18.6 jetable (Docker requis ; télécharge Chromium au premier passage). Captures et logs dans `web/e2e-artifacts/`, ignoré par git.
- Dans `web/` : `npm ci`, puis `npm run dev` (proxy de `/v1` vers `API_TARGET`, par défaut `http://127.0.0.1:8080`), `npm run lint`, `npm run typecheck`, `npm test`, `npm run build`.

- Serveur MCP : `mcp/test.sh [arguments de node --test]` lance types, build, tests OAuth, puis de bout en bout contre le vrai backend et un PostgreSQL 18.6 jetable (Docker requis). Dans `mcp/` : `npm ci`, `npm run typecheck`, `npm run build`, `npm test` (sans backend, les tests de bout en bout sont sautés).
- Lancer le MCP en développement, depuis `mcp/` après `npm run build` : `MCP_PUBLIC_URL=https://localhost:8443 MCP_BACKEND_URL=http://127.0.0.1:8080 MCP_SERVICE_TOKEN=<jeton de développement> MCP_LOGIN_HASH=<sortie de hash-password> MCP_LISTEN=127.0.0.1:8787 MCP_STATE_DIR=<dossier> node dist/main.js serve`. Le haché : `printf '%s\n' "$SECRET" | node dist/main.js hash-password` (16 caractères au moins, lu sur l'entrée standard, sortie déjà encodée pour un fichier d'environnement). Jamais de jeton ni de secret de production en développement.
- Images de production, linux/amd64 : `docker buildx build --platform linux/amd64 --load -t <nom> backend` (de même pour `web` et `mcp`). Sur GitHub, seulement à une release : `.github/workflows/release.yml`.
- Releases : déploiement continu. Quand la CI de `main` est verte et que le contenu d'une image a changé depuis le dernier tag, `.github/workflows/release.yml` calcule la version (`deploy/next-version.sh <sha> <fichier des notes>`), crée le tag `vX.Y.Z` et la GitHub Release avec ses notes, construit les images `vX.Y.Z` et déploie sur le NAS (`deploy/deploy.sh` par Tailscale et SSH). Retour arrière : `gh workflow run Release -f tag=vX.Y.Z`. Mise en place : `docs/deploiement.md`, « Déploiement automatique ».
- Production : `compose.yaml`, à lancer sur le NAS et jamais depuis le dépôt ; secrets et commandes dans `docs/deploiement.md`.

Depuis `backend/`, après `set -a && . ../.env && set +a` (chaque commande lit sa propre URL) :

- Migrations : `cargo run -- migrate` (`MIGRATE_DATABASE_URL`, `tracker_owner`).
- API et interprètes : `cargo run -- serve` (`DATABASE_URL`, `tracker_app` ; `LISTEN_ADDR`, `RUST_LOG`).
- Collecteur : `cargo run -- collect` (`LASTFM_API_KEY`, `LASTFM_USER`, `COLLECTOR_TOKEN` : le token d'une source de développement, `INGEST_URL`, `COLLECTOR_STATE_DIR` ; facultatifs `LASTFM_API_URL`, `COLLECT_INTERVAL_SECONDS`). Ne connaît pas la base. Jamais de token ni de clé de production en développement.
- Administration (`ADMIN_DATABASE_URL`, `tracker_admin`) : `cargo run -- admin create-source <nom> [--silence-threshold "6 hours"]` (affiche le token une seule fois), `admin revoke-source <nom>`, `admin create-service-token <nom>` (jeton de service de l'API de lecture, pour le MCP : lecture seule, pseudonymes ; affiché une seule fois), `admin revoke-service-token <nom>`, `admin grant-kind <source> <type>`, `admin revoke-kind <source> <type>`, `admin rebuild [--allow-differences]` (génération à côté, comparaison, bascule), `admin retry-errors`, `admin set-password <identifiant>` (crée le compte web ou change son mot de passe, lu au clavier ou sur l'entrée standard, et ferme les sessions), `admin coverage <métrique>` (par source : nombre et dates, trou ou chevauchement), `admin set-precedence <métrique> <source> [--from <RFC 3339>]` (la source fait foi à partir de cette date ; sans date, sa première donnée, refusée si elle est isolée).

Import Apple, depuis `backend/` : `INGEST_URL=http://127.0.0.1:8080 cargo run -- import-apple --archive <dossier> < <fichier du token>` (dossier des trois fichiers lus, voir `docs/deploiement.md` ; aucun accès à la base, relancer reprend). Jamais de vraie archive dans le dépôt : les fixtures de `backend/tests/fixtures/apple/` sont fictives.

Le cookie de session est toujours `Secure`. Seule exception, pour le développement : `DEV_INSECURE_COOKIE=true` sur `serve`, utile à Safari et à un téléphone en http. Chrome et Firefox l'acceptent déjà sur `localhost`.

## Règles à ne jamais casser

- Toute donnée entre par l'API d'ingestion, toute lecture passe par l'API de lecture. Rien d'autre ne touche la base.
- Le journal `core.records` est en ajout seul. On corrige par une annulation, jamais par une modification. Seule exception : la purge, par l'outil d'administration.
- Les interprètes sont déterministes : ni heure courante, ni appel externe. Un enrichissement externe entre lui-même dans le journal.
- Ce qui croise plusieurs enregistrements ou dépend de tout l'historique (découvertes, minutes d'écoute) se calcule en vue, pas dans un interprète.
- `derived` se reconstruit et ne se migre jamais. Un rejeu ne déclenche aucun effet de bord.
- Tout est stocké en UTC ; le jour de Paris se calcule à la lecture.
- Un nouveau tracker se déclare dans les registres. Pas de table, d'index ni d'outil MCP propre à un domaine sans vraie nécessité.
- Une journée sans données ne se confond jamais avec une journée sans activité.

## Sécurité

- Aucun secret dans le dépôt, aucun token de production en développement, jamais de test contre le journal de production.
- Aucune donnée personnelle dans les logs : seulement des identifiants d'enregistrements.
- Le backend tourne avec `tracker_app` et ne connaît jamais le mot de passe de `tracker_owner`. Ne pas élargir les droits pour contourner un refus de la base : un refus signale en général un défaut de conception.
- Toute route de la web app exige la session, même derrière Tailscale.
- Le MCP est en lecture seule et ne voit que des pseudonymes.

## Conventions

- Documentation et commentaires en français ; code, noms de champs et API en anglais.
- Migrations dans `backend/migrations/`, sans version descendante. Elles restent modifiables jusqu'au premier déploiement sur le NAS (étape 4) ; ensuite, on en ajoute une nouvelle.
- Versions d'images explicites, jamais `latest`. PostgreSQL 18 reste figé.
- Git : une branche `type/sujet` par travail (par exemple `feat/compteur-the`), une PR par `gh pr create`. Messages et titres de PR en Conventional Commits : `type(scope): description en français`. Types : `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `chore`, `ci`, `build` ; scopes facultatifs : `backend`, `web`, `mcp`, `db`, `deploy`, `ci` ; `!` ou `BREAKING CHANGE:` pour une rupture. Fusion en squash : le titre de la PR devient le commit sur `main`. Toute PR qui change le code des images (`backend/`, `web/`, `mcp/`, hors tests et Markdown) part en production à la fusion, une fois la CI verte ; son type ne fixe que la version : `feat` donne une mineure, le reste un correctif, une rupture une mineure avant 1.0.0. Une PR qui demande une action sur le NAS (`compose.yaml` à recopier avant la fusion, secret, `admin rebuild`) le dit dans sa description. Une PR qui change l'interface de `web/` montre ses captures dans sa description : celles de Playwright (`web/e2e-artifacts/`, données fictives seulement), à la largeur d'un iPhone et d'un Mac, en clair et en sombre, pour chaque écran touché. `gh` ne téléverse pas d'image : elles vont dans la branche orpheline `captures`, sous `pr-<numéro>/`, jamais dans `main`, et la description les affiche par `https://github.com/clegirar/life-tracker/blob/captures/pr-<numéro>/<fichier>.png?raw=true`. Chaque commit est signé (`git commit -s`, pour le contrôle DCO). C'est l'utilisateur qui fusionne. Commit, push et PR seulement à sa demande.
- Le plus simple qui respecte les principes. Toute décision de conception s'ajoute au journal des décisions de l'architecture, et chaque étape terminée se note dans `docs/poc.md`.

## Interface

Pour tout travail sur la web app, suivre la section « Interface » de l'architecture et utiliser les skills `apple-design`, `emil-design-eng`, `mobile-native`, `animate` et `review-animations`. Le skill `design-taste-frontend` ne s'applique pas aux écrans d'app.
