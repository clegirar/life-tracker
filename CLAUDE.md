# Life tracker

App auto-hébergée sur un NAS Synology qui retrace ma vie jour par jour, avec un calendrier web et un serveur MCP pour Claude. On construit le POC : le compteur de cafés et la musique.

## Où trouver quoi

- `docs/poc.md` : les étapes du POC et leurs critères. On travaille une étape à la fois, dans l'ordre ; une étape n'est finie que si tous ses critères « Terminé quand » sont remplis.
- `docs/architecture.md` : les principes, le contrat de l'API d'ingestion et le journal des décisions. À lire avant toute décision de conception.
- `docs/database.md` : le schéma et ses raisons.

## Périmètre

Hors POC : les messages et l'agent Mac, l'annuaire côté interface, les autres trackers, les « fonctionnalités futures » de l'architecture, les sauvegardes, l'ouverture de la web app sur Internet. Ne rien construire pour eux par anticipation ; le design des messages reste dans l'architecture pour plus tard.

## Commandes

Seul ce qui existe aujourd'hui est listé ; ajouter ici les commandes de chaque nouveau composant.

- Tout le dépôt : `./test.sh` (base, qualité et tests du backend, web app ; conteneurs jetables, Docker requis).
- Tests de la base : `db/test/run.sh` (conteneurs jetables, Docker requis).
- Base de développement : `cp .env.example .env && docker compose -f compose.dev.yaml up -d` (port 54329).
- Tests du backend : `backend/test.sh [arguments de cargo test]` (conteneur jetable, Docker requis ; Rust figé par `backend/rust-toolchain.toml`).
- Après un changement de requête SQL : `backend/test.sh --prepare` régénère le cache `backend/.sqlx/` (sqlx-cli 0.9 requis), à versionner.
- Qualité : dans `backend/`, `cargo fmt --check` et `cargo clippy --all-targets -- -D warnings`.
- Web app : `web/test.sh [arguments de playwright test]` lance lint, types, tests unitaires, build, puis Playwright contre le vrai backend et un PostgreSQL 18.6 jetable (Docker requis ; télécharge Chromium au premier passage). Captures et logs dans `web/e2e-artifacts/`, ignoré par git.
- Dans `web/` : `npm ci`, puis `npm run dev` (proxy de `/v1` vers `API_TARGET`, par défaut `http://127.0.0.1:8080`), `npm run lint`, `npm run typecheck`, `npm test`, `npm run build`.

- Images de production, linux/amd64 : `docker buildx build --platform linux/amd64 --load -t <nom> backend` (de même pour `web`). Sur GitHub : `.github/workflows/images.yml`.
- Production : `compose.yaml`, à lancer sur le NAS et jamais depuis le dépôt ; secrets et commandes dans `docs/deploiement.md`.

Depuis `backend/`, après `set -a && . ../.env && set +a` (chaque commande lit sa propre URL) :

- Migrations : `cargo run -- migrate` (`MIGRATE_DATABASE_URL`, `tracker_owner`).
- API et interprètes : `cargo run -- serve` (`DATABASE_URL`, `tracker_app` ; `LISTEN_ADDR`, `RUST_LOG`).
- Administration (`ADMIN_DATABASE_URL`, `tracker_admin`) : `cargo run -- admin create-source <nom> [--silence-threshold "6 hours"]` (affiche le token une seule fois), `admin revoke-source <nom>`, `admin grant-kind <source> <type>`, `admin revoke-kind <source> <type>`, `admin rebuild [--allow-differences]` (génération à côté, comparaison, bascule), `admin retry-errors`, `admin set-password <identifiant>` (crée le compte web ou change son mot de passe, lu au clavier ou sur l'entrée standard, et ferme les sessions).

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
- Le plus simple qui respecte les principes. Toute décision de conception s'ajoute au journal des décisions de l'architecture, et chaque étape terminée se note dans `docs/poc.md`.

## Interface

Pour tout travail sur la web app, suivre la section « Interface » de l'architecture et utiliser les skills `apple-design`, `emil-design-eng`, `mobile-native`, `animate` et `review-animations`. Le skill `design-taste-frontend` ne s'applique pas aux écrans d'app.
