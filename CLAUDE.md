# Life tracker

App auto-hébergée sur un NAS Synology qui retrace ma vie jour par jour, avec un calendrier web et un serveur MCP pour Claude. Le POC (compteur de cafés et musique) est validé depuis le 4 nov. 2026 ; Garmin Connect (pas, sommeil, séances, calories) s'y est ajouté le 5 nov. 2026. Les chantiers suivants se décident un par un.

## Où trouver quoi

- `docs/chantiers.md` : les chantiers en cours depuis la fin du POC, leurs choix et leurs critères. Un chantier n'est fini que si tous ses critères « Terminé quand » sont remplis.
- `docs/poc.md` : les étapes du POC, terminé le 4 nov. 2026 ; figé, pour l'historique.
- `docs/architecture.md` : les principes, le contrat de l'API d'ingestion et le journal des décisions. À lire avant toute décision de conception.
- `docs/database.md` : le schéma et ses raisons.
- `TODO.md` : les reprises notées pour plus tard, à ne traiter que sur demande.

## Périmètre

Chantiers en cours (voir `docs/chantiers.md`) : Steam, par l'API Web de Steam (temps de jeu par jeu et par jour, total d'heures, succès ; en production, restent des vérifications après une vraie partie). Twitch, par une extension Firefox (en production, extension signée et installée ; restent des vérifications). YouTube, par l'API Google Data Portability, est construit. La page d'état des trackers est construite (PR #13), sa mise en service sur le NAS reste à vérifier. Garmin Connect est fait. Déplacements, par l'app Overland sur l'iPhone (trace du jour et lieux) : backend construit, restent la web app et la mise en service. GitHub, par l'API GraphQL (commits, PR, reviews, issues, commentaires, lignes ; tout l'historique) : construit, restent la mise en service et une vraie journée. Hors champ tant que je ne les demande pas : l'import de l'historique Twitch (export RGPD), les messages et l'agent Mac, l'annuaire côté interface, les autres trackers, les « fonctionnalités futures » de l'architecture, les sauvegardes, l'ouverture de la web app sur Internet, le contenu de `TODO.md`. Ne rien construire pour eux par anticipation ; le design des messages reste dans l'architecture pour plus tard.

## Commandes

Seul ce qui existe aujourd'hui est listé ; ajouter ici les commandes de chaque nouveau composant.

- Tout le dépôt : `./test.sh` (script de déploiement, base, qualité et tests du backend, web app, serveur MCP, extension Firefox ; conteneurs jetables, Docker requis).
- Scripts de déploiement : `deploy/test.sh` (`deploy.sh` avec un faux `docker`, sans Docker ; `next-version.sh` sur un dépôt git jetable ; shellcheck s'il est installé).
- CI : `.github/workflows/ci.yml`, sur chaque PR et chaque push sur `main` hors documentation, lance en parallèle `db/test/run.sh`, le format, clippy et `backend/test.sh`, `web/test.sh`, `mcp/test.sh`, `extension/test.sh` et `deploy/test.sh`. `pr-title.yml` contrôle le titre de la PR. Workflows vérifiés par `actionlint`.
- Tests de la base : `db/test/run.sh` (conteneurs jetables, Docker requis).
- Base de développement : `cp .env.example .env && docker compose -f compose.dev.yaml up -d` (port 54329).
- Tests du backend : `backend/test.sh [arguments]` (conteneur jetable, Docker requis ; Rust figé par `backend/rust-toolchain.toml`). Par `cargo nextest run` si cargo-nextest est installé (comme en CI, 0.9.150), sinon `cargo test` : les arguments suivent l'outil. Les tests les plus longs partent en premier (`backend/.config/nextest.toml`).
- Après un changement de requête SQL : `backend/test.sh --prepare` régénère le cache `backend/.sqlx/` (sqlx-cli 0.9 requis), à versionner.
- Qualité : dans `backend/`, `cargo fmt --check` et `cargo clippy --all-targets -- -D warnings`.
- Web app : `web/test.sh [arguments de playwright test]` lance lint, types, tests unitaires, build, puis Playwright contre le vrai backend et un PostgreSQL 18.6 jetable (Docker requis ; télécharge Chromium au premier passage). Captures et logs dans `web/e2e-artifacts/`, ignoré par git.
- Dans `web/` : `npm ci`, puis `npm run dev` (proxy de `/v1` vers `API_TARGET`, par défaut `http://127.0.0.1:8080`), `npm run lint`, `npm run typecheck`, `npm test`, `npm run build`.

- Serveur MCP : `mcp/test.sh [arguments de node --test]` lance types, build, tests OAuth, puis de bout en bout contre le vrai backend et un PostgreSQL 18.6 jetable (Docker requis). Dans `mcp/` : `npm ci`, `npm run typecheck`, `npm run build`, `npm test` (sans backend, les tests de bout en bout sont sautés).
- Extension Firefox (Twitch) : `extension/test.sh [arguments de node --test]` lance types, tests unitaires (page, segmenteur, file avec un faux `fetch`, heure, lot validé contre `contract/ingestion-v1.schema.json` par ajv), build, puis `web-ext lint` (ni Docker ni réseau). Dans `extension/` : `npm ci`, `npm run typecheck`, `npm test`, `npm run build` (dans `dist/`, à charger par about:debugging pour un essai), `npm run lint`. Signature, sur le Mac seulement : `extension/sign.sh` (`web-ext sign --channel=unlisted`, clés AMO `WEB_EXT_API_KEY`/`WEB_EXT_API_SECRET` lues dans l'environnement, jamais dans le dépôt ni en CI ; version à augmenter dans `extension/package.json` ; `.xpi` dans `extension/web-ext-artifacts/`). Pas de release : `deploy/next-version.sh` ne regarde que `backend web mcp`.
- Lancer le MCP en développement, depuis `mcp/` après `npm run build` : `MCP_PUBLIC_URL=https://localhost:8443 MCP_BACKEND_URL=http://127.0.0.1:8080 MCP_SERVICE_TOKEN=<jeton de développement> MCP_LOGIN_HASH=<sortie de hash-password> MCP_LISTEN=127.0.0.1:8787 MCP_STATE_DIR=<dossier> node dist/main.js serve`. Le haché : `printf '%s\n' "$SECRET" | node dist/main.js hash-password` (16 caractères au moins, lu sur l'entrée standard, sortie déjà encodée pour un fichier d'environnement). Jamais de jeton ni de secret de production en développement.
- Images de production, linux/amd64 : `docker buildx build --platform linux/amd64 --load -t <nom> backend` (de même pour `web` et `mcp`). Sur GitHub, seulement à une release : `.github/workflows/release.yml`.
- Releases : déploiement continu. Quand la CI de `main` est verte et que le contenu d'une image a changé depuis le dernier tag, `.github/workflows/release.yml` calcule la version (`deploy/next-version.sh <sha> <fichier des notes>`), crée le tag `vX.Y.Z` et la GitHub Release avec ses notes, construit les images `vX.Y.Z` et déploie sur le NAS (`deploy/deploy.sh` par Tailscale et SSH). Retour arrière : `gh workflow run Release -f tag=vX.Y.Z`. Mise en place : `docs/deploiement.md`, « Déploiement automatique ».
- Production : `compose.yaml`, à lancer sur le NAS et jamais depuis le dépôt ; secrets et commandes dans `docs/deploiement.md`.

Depuis `backend/`, après `set -a && . ../.env && set +a` (chaque commande lit sa propre URL) :

- Migrations : `cargo run -- migrate` (`MIGRATE_DATABASE_URL`, `tracker_owner`).
- API et interprètes : `cargo run -- serve` (`DATABASE_URL`, `tracker_app` ; `LISTEN_ADDR`, `RUST_LOG` ; `PRIVACY_ZONES`, `lat,lon,rayon_m` séparées par `;`, une zone fictive en développement).
- Collecteur : `cargo run -- collect` (`INGEST_URL`, `COLLECTOR_STATE_DIR`, et au moins un connecteur). Last.fm si `LASTFM_API_KEY` est fourni, avec `LASTFM_USER` et `COLLECTOR_TOKEN` (le token d'une source de développement) ; facultatifs `LASTFM_API_URL`, `COLLECT_INTERVAL_SECONDS`. Garmin si `GARMIN_API_KEY` est fourni, avec `GARMIN_TOKEN` (le token de la source `garmin`) ; facultatifs `GARMIN_API_URL` (tests), `GARMIN_SOURCE` (nom de la source, `garmin` par défaut), `GARMIN_INTERVAL_SECONDS` (900), `GARMIN_KEY_EXPIRES_ON` (`AAAA-MM-JJ`, l'échéance affichée par Garmin, rapportée à la page d'état ; illisible, le collecteur ne démarre pas). Chaque connecteur rapporte son état après chaque passage (`PUT /v1/sources/me/status`). YouTube si `YOUTUBE_CLIENT_ID` est fourni, avec `YOUTUBE_CLIENT_SECRET`, `YOUTUBE_REDIRECT_URI` (la page `/autorisation` de la web app), `YOUTUBE_API_KEY` (Data v3) et `YOUTUBE_TOKEN` (le token de la source `youtube`) ; facultatifs `YOUTUBE_SOURCE` (nom de la source, `youtube` par défaut), `YOUTUBE_INTERVAL_SECONDS` (300 ; un export au plus par 24 h) et, pour les tests, `YOUTUBE_OAUTH_URL`, `YOUTUBE_PORTABILITY_URL`, `YOUTUBE_DATA_URL`, `YOUTUBE_STORAGE_URL`. L'accord Google se donne depuis la page État de la web app. Steam si `STEAM_API_KEY` est fourni, avec `STEAM_ID` (l'identifiant Steam 64 bits) et `STEAM_TOKEN` (le token de la source `steam`) ; facultatifs `STEAM_INTERVAL_SECONDS` (900) et, pour les tests, `STEAM_API_URL`. GitHub si `GITHUB_API_TOKEN` est fourni (token classique, `repo` et `read:org`), avec `GITHUB_INGEST_TOKEN` (le token de la source `github` ; pas `GITHUB_TOKEN`, pris par `gh`) et `GITHUB_TOKEN_EXPIRES_ON` (`AAAA-MM-JJ`, illisible, le collecteur ne démarre pas) ; facultatifs `GITHUB_INTERVAL_SECONDS` (900) et, pour les tests, `GITHUB_API_URL`. Chaque connecteur a son état (`state.json`, `garmin-state.json`, `youtube-state.json` en 0600, `steam-state.json`, `github-state.json`). Ne connaît pas la base. Jamais de token ni de clé de production en développement : il n'existe qu'une clé Garmin et une clé Steam par compte, et elles vivent sur le NAS, comme le token GitHub (jamais celui de `gh` du Mac).
- Position de l'iPhone : pas de connecteur ; l'app Overland pousse ses lots sur la route d'adaptation `POST /v1/locations/overland` (token de la source `iphone` : `admin create-source iphone --silence-threshold "2 days" --label "Position (iPhone)"`, puis `admin grant-kind iphone overland.location` et `overland.visit`), traduits en `overland.location` et `overland.visit`. La carte du jour lit `GET /v1/days/{date}/locations` (session seulement).
- Twitch : pas de connecteur ; l'extension Firefox (`extension/`, voir plus haut et `docs/deploiement.md`) écrit `twitch.watch` avec le token de la source `twitch` (`admin create-source twitch --silence-threshold "3 days" --label "Twitch"`, puis `admin grant-kind twitch twitch.watch`).
- Administration (`ADMIN_DATABASE_URL`, `tracker_admin`) : `cargo run -- admin create-source <nom> [--silence-threshold "6 hours"] [--label "Garmin Connect"]` (affiche le token une seule fois), `admin set-label <source> <libellé>` (le nom affiché par la page d'état), `admin revoke-source <nom>`, `admin create-service-token <nom>` (jeton de service de l'API de lecture, pour le MCP : lecture seule, pseudonymes ; affiché une seule fois), `admin revoke-service-token <nom>`, `admin grant-kind <source> <type>`, `admin revoke-kind <source> <type>`, `admin rebuild [--allow-differences]` (génération à côté, comparaison, bascule), `admin retry-errors`, `admin set-password <identifiant>` (crée le compte web ou change son mot de passe, lu au clavier ou sur l'entrée standard, et ferme les sessions), `admin coverage <métrique>` (par source : nombre et dates, trou ou chevauchement), `admin set-precedence <métrique> <source> [--from <RFC 3339>]` (la source fait foi à partir de cette date ; sans date, sa première donnée, refusée si elle est isolée).

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
- Tout nouveau tracker a sa carte dans la page d'état des trackers (s'il fonctionne, ce qu'il attend de moi) ; un chantier de tracker n'est pas fini sans elle.
- Tout nouveau tracker et toute nouvelle clé ont leur fiche dans la page Sources (`web/src/sources.ts` : comment les données arrivent, le nom de chaque clé et son échéance), jamais avec une valeur secrète ni un nom d'hôte du NAS.
- Une journée sans données ne se confond jamais avec une journée sans activité.

## Sécurité

- Aucun secret dans le dépôt, aucun token de production en développement, jamais de test contre le journal de production.
- Aucune donnée personnelle dans les logs : seulement des identifiants d'enregistrements.
- Le backend tourne avec `tracker_app` et ne connaît jamais le mot de passe de `tracker_owner`. Ne pas élargir les droits pour contourner un refus de la base : un refus signale en général un défaut de conception.
- Toute route de la web app exige la session, même derrière Tailscale.
- Le MCP est en lecture seule et ne voit que des pseudonymes. Il ne reçoit des détails d'événements que les champs listés par `core.metrics.shared_details`, et toute coordonnée ou trace passe par les zones de confidentialité (`PRIVACY_ZONES` de `serve`, sur le NAS seulement, jamais dans le dépôt, le journal ni les logs) ; sans zone valide, aucune coordonnée ni aucun parcours pour le jeton de service.

## Conventions

- Documentation et commentaires en français ; code, noms de champs et API en anglais.
- Migrations dans `backend/migrations/`, sans version descendante. Elles restent modifiables jusqu'au premier déploiement sur le NAS (étape 4) ; ensuite, on en ajoute une nouvelle.
- Versions d'images explicites, jamais `latest`. PostgreSQL 18 reste figé.
- Git : une branche `type/sujet` par travail (par exemple `feat/compteur-the`), une PR par `gh pr create`. Messages et titres de PR en Conventional Commits : `type(scope): description en français`. Types : `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `chore`, `ci`, `build` ; scopes facultatifs : `backend`, `web`, `mcp`, `extension`, `db`, `deploy`, `ci` ; `!` ou `BREAKING CHANGE:` pour une rupture. Fusion en squash : le titre de la PR devient le commit sur `main`. Toute PR qui change le code des images (`backend/`, `web/`, `mcp/`, hors tests et Markdown) part en production à la fusion (`extension/` n'en fait pas partie : elle se signe et s'installe à la main), une fois la CI verte ; son type ne fixe que la version : `feat` donne une mineure, le reste un correctif, une rupture une mineure avant 1.0.0. Une PR qui demande une action sur le NAS (`compose.yaml` à recopier avant la fusion, secret, `admin rebuild`) le dit dans sa description. Une PR qui change l'interface de `web/` montre ses captures dans sa description : celles de Playwright (`web/e2e-artifacts/`, données fictives seulement), à la largeur d'un iPhone et d'un Mac, en clair et en sombre, pour chaque écran touché. `gh` ne téléverse pas d'image : elles vont dans la branche orpheline `captures`, sous `pr-<numéro>/`, jamais dans `main`, et la description les affiche par `https://github.com/clegirar/life-tracker/blob/captures/pr-<numéro>/<fichier>.png?raw=true`. Chaque commit est signé (`git commit -s`, pour le contrôle DCO). C'est l'utilisateur qui fusionne. Commit, push et PR seulement à sa demande.
- Le plus simple qui respecte les principes. Toute décision de conception s'ajoute au journal des décisions de l'architecture, et l'avancement de chaque chantier se note dans `docs/chantiers.md`.

## Interface

Pour tout travail sur la web app, suivre la section « Interface » de l'architecture et utiliser les skills `apple-design`, `emil-design-eng`, `mobile-native`, `animate` et `review-animations`. Le skill `design-taste-frontend` ne s'applique pas aux écrans d'app.
