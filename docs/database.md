# Life tracker — Base de données

Oct 17, 2026 · @C

Le schéma PostgreSQL du POC, tiré du contrat de l'API d'ingestion v1 et des règles d'event sourcing de [l'architecture](architecture.md). Le SQL fait foi : `backend/migrations/`. Ce document explique les choix.

## Vue d'ensemble

Deux schémas, qui reprennent la ligne de partage du principe 3 :

- **`core`** : la vérité (le journal brut) et les référentiels. Rien ne s'y recalcule ; on le migre avec soin.
- **`derived`** : tout ce qui se reconstruit depuis le journal (événements normalisés, annulations, positions et erreurs des interprètes). On le jette et on le reconstruit, jamais on ne le migre.

PostgreSQL 18, version majeure figée (`postgres:18.6`). Deux extensions : `pg_trgm` (recherche approximative) et `btree_gist` (contrainte de non-chevauchement).

## Décisions

1. **Ordre global garanti par la base.** Une séquence peut rendre le numéro 11 visible avant le 10, si deux transactions se croisent : un interprète qui avance son curseur sauterait alors le 10 pour toujours. Un trigger prend un verrou exclusif au début de chaque ajout au journal, et la séquence n'a pas de cache : les numéros deviennent visibles dans l'ordre. Les volumes rendent ce verrou indolore ; l'ingestion garde une transaction courte par lot.
2. **Immuabilité garantie par la base, pas par la discipline.** Le rôle d'exécution n'a que le droit d'ajouter et de lire le journal. Un trigger refuse en plus toute modification, suppression ou remise à zéro, quel que soit le rôle, sauf la rédaction de purge : vider la charge utile et poser la référence de purge, une seule fois, sans toucher au reste. Le registre des purges est protégé de la même façon.
3. **Reconstruction par générations.** Chaque interprétation complète du journal est une génération (`building`, `active`, `retired`) ; toutes les tables dérivées portent son numéro. On construit la suivante à côté, puis on bascule en changeant une ligne : le calendrier n'est jamais vide. Comparer deux générations (règle 16, le rejeu complet comme test) est une jointure. Une base ne peut avoir qu'une génération active et une seule en construction.
4. **Les lectures passent par `derived.live_events`.** Cette vue ne garde que la génération active, retire les enregistrements annulés et les sources qui ne font pas foi sur la période. L'API de lecture n'interroge jamais les tables dérivées directement.
5. **Une clé déterministe pour les événements** : génération, numéro du journal, rang de l'événement dans l'enregistrement (un enregistrement donne 0, 1 ou n événements). Deux générations se comparent ligne à ligne.
6. **Les éléments d'un événement sont une table générique.** Une écoute a un artiste, un album et un titre ; un message a un contact. Chacun devient une ligne de `derived.event_items` (niveau, clé normalisée, nom affiché). Les outils Occurrences, Découverte, Dernière fois, Classement et Rechercher travaillent tous sur cette table, indexée une fois pour toutes : un nouveau domaine n'ajoute ni table ni index (principe 12).
7. **Déclarations et références sont des métriques non affichées.** « Déjà connu » (`music.known`, forme `declaration`) et la durée d'un titre (`music.track_duration`, forme `reference`) sont des événements comme les autres, rattachés à la métrique qu'ils éclairent. Seules les annulations ont leur table, parce qu'elles valent pour toutes les métriques.
8. **Une annulation désigne sa cible par source et clé**, jamais par numéro du journal : elle peut arriver avant sa cible (raccourci iOS resté hors ligne). La vue fait le lien à la lecture.
9. **On ne stocke que les échecs d'interprétation.** La position de chaque interprète dit déjà ce qui a été traité ; une ligne « ok » par enregistrement doublerait le volume pour rien.
10. **Pas d'index sur la journée.** Le jour demandé est converti en intervalle UTC, de minuit à minuit heure de Paris, puis cherché sur l'heure de l'événement. Les journées de 23 et 25 heures des changements d'heure sont ainsi justes, et le fuseau n'est figé nulle part (règle 10).
11. **La réconciliation est déclarée, pas codée.** `core.source_precedence` dit quelle source fait foi pour une métrique sur une période ; la base refuse deux périodes qui se chevauchent (`WITHOUT OVERLAPS`, nouveauté de PostgreSQL 18). C'est là que se déclare « Last.fm fait foi à partir de son démarrage ».
12. **La base vérifie qu'un type est déclaré pour sa source.** Le journal référence le couple source et type dans `core.source_kinds` : un enregistrement d'un type non déclaré est refusé par la base, même si l'API l'oubliait. Un droit se révoque par une date, jamais en supprimant la ligne.
13. **L'enveloppe est figée à vie, donc complète dès maintenant.** Elle porte en plus la version du client (purger « tout ce qu'a envoyé l'agent en version X ») et l'identifiant de requête (suivre un lot dans les logs). Le décalage horaire est vide quand la source envoie `Z` : il est inconnu, pas nul.
14. **Tokens et sessions hachés en SHA-256.** Ce sont des valeurs aléatoires de 32 octets : un hachage rapide et indexé suffit et retrouve la ligne directement. Seul le mot de passe du compte, choisi par un humain, est haché en Argon2id.
15. **Supprimer une personne anonymise son historique.** Ses pseudonymes disparaissent avec elle ; le journal et les événements gardent les pseudonymes, qui s'affichent « Contact » suivi d'un court identifiant.

## Tables

### `core` : la vérité et les référentiels

| Table | Contenu |
| --- | --- |
| `records` | Le journal brut. Numéro d'ordre, source, type, version de format, clé de dédoublonnage, date de l'événement et décalage d'origine, date de réception, version du client, identifiant de requête, charge utile, référence de purge. Unique par source et clé. |
| `purges` | Registre des purges, en ajout seul : date, auteur, sélecteur, motif, nombre d'enregistrements. Jamais le contenu purgé. |
| `sources` | Registre des sources : nom stable, authentification (token ou session de la web app), token haché, seuil de silence (vide : jamais muette), dernier contact, version du client, rejets. |
| `source_kinds` | Les types que chaque source a le droit d'envoyer. |
| `kinds` | Les types de données : version de format courante, interprète avec ou sans mémoire. |
| `metrics` | Registre des métriques : forme, unité, domaine, agrégat du jour, niveaux d'éléments, découverte, métrique éclairée, source de la durée, seuil de fraîcheur. |
| `source_precedence` | Quelle source fait foi, par métrique et par période. |
| `people`, `pseudonyms` | L'annuaire : une personne et son nom affiché, ses pseudonymes (numéro, e-mail). |
| `settings` | Une ligne : date de mise en ligne, fuseau des journées (`Europe/Paris`). |
| `account`, `sessions` | Le compte unique de la web app et ses sessions. |

### `derived` : recalculable

| Table | Contenu |
| --- | --- |
| `generations` | Les générations et leur état, avec la version de chaque interprète. |
| `cursors` | Par génération et par type : dernier numéro du journal traité. |
| `interpretation_errors` | Les enregistrements en erreur, avec le message et la version de l'interprète. |
| `events` | Les événements normalisés : métrique, source, date et décalage, valeur, détails. |
| `event_items` | Les éléments de chaque événement, un par niveau. |
| `cancellations` | Les annulations : source et clé de la cible. |
| `live_events`, `live_event_items` | Les vues que lit l'API de lecture. |

### Registres du POC

| Type de donnée | Envoyé par | Produit |
| --- | --- | --- |
| `coffee.cup` | Web app, raccourci iOS | `coffee` |
| `music.mark_known` | Web app | `music.known` |
| `record.cancel` | Web app, raccourci iOS | une annulation |
| `lastfm.scrobble` | Connecteur Last.fm | `music.listen` |
| `lastfm.track_info` | Connecteur Last.fm | `music.track_duration` |
| `apple.play_history_row` | Import Apple | `music.listen` |

| Métrique | Forme | Agrégat du jour | Niveaux | Particularités |
| --- | --- | --- | --- | --- |
| `coffee` | compteur | nombre | aucun | Chaque café garde son heure |
| `music.listen` | événement riche | nombre | artiste, album, titre | Découverte ; durée tirée de `music.track_duration` ; à vérifier après 3 jours sans écoute |
| `music.known` | déclaration | aucun | artiste, album, titre | Éclaire `music.listen` |
| `music.track_duration` | référence | aucun | titre | Valeur : durée en secondes |

Seule la source `webapp` est créée par la migration. Les sources à token (connecteur Last.fm, raccourci iOS, import Apple) sont créées par l'outil d'administration, pour qu'aucun secret ne soit dans le dépôt.

Les messages sont hors POC depuis le 31 oct. 2026 : leur type et leur métrique arriveront par une migration, avec l'agent Mac. L'annuaire (`people`, `pseudonyms`) est déjà en place et testé pour les accueillir, mais rien ne s'en sert pendant le POC.

**L'heure d'un café** est celle de l'enveloppe (`occurred_at`), fixée par le client : « maintenant » par défaut, modifiable pour un café noté après coup. La charge utile reste vide. Une heure erronée se corrige par une annulation et une nouvelle saisie.

**Les détails d'une écoute** gardent les noms affichés (`artist`, `album`, `track`), les clés normalisées (`artist_key`, `album_key` = artiste et album, `track_key` = artiste et titre) et, si l'export Apple la fournit, la durée réellement écoutée (`played_ms`). La clé d'un album inclut l'artiste, sinon tous les « Greatest Hits » seraient confondus. Les minutes d'écoute se calculent en vue : la durée écoutée si elle existe, sinon la dernière durée connue du titre.

## Rôles et droits

| Rôle | Sert à | Peut |
| --- | --- | --- |
| `tracker_owner` | Les migrations, et rien d'autre | Tout, sauf ce que les triggers refusent |
| `tracker_app` | Le backend à l'exécution | Ajouter et lire le journal ; mettre à jour le dernier contact et les rejets des sources ; gérer l'annuaire et les sessions ; tout dans `derived` |
| `tracker_admin` | L'outil d'administration, sur le NAS uniquement | Lire tout `core` ; gérer les registres ; rédiger une purge (charge utile et référence seulement) ; tout dans `derived` |

Les rôles et la base sont créés par `db/init/01-roles.sh` à la création du volume ; les mots de passe viennent de l'environnement.

**Migrations.** En production, un service `migrate` à usage unique du docker-compose applique les migrations avec le rôle propriétaire, puis le backend démarre avec `tracker_app`. Le backend ne connaît donc jamais le mot de passe du propriétaire, qui est le seul à pouvoir désactiver un trigger : c'est la porte de secours assumée, et elle ne sert qu'aux migrations.

## Cycles de vie

**Ingestion.** Une transaction par lot : `INSERT … ON CONFLICT (source_id, key) DO NOTHING RETURNING key`. Une clé absente de la réponse est un `duplicate`. Les rejets de l'enveloppe se décident avant l'insertion et incrémentent le compteur de la source.

**Interprétation.** Une tâche de fond de `serve`, en `tracker_app`. Une transaction par enregistrement : verrou partagé sur la génération active, verrou exclusif sur le curseur du type (qui sérialise les workers), événements et éléments ou erreur, curseur. Un enregistrement purgé avance le curseur sans rien produire. La base refuse une sortie (classes 22 et 23) : c'est une erreur d'interprétation, avec le code seul. Toute autre erreur de la base est une panne : rien n'est écrit, nouvel essai après 1 s, 2 s, 4 s… jusqu'à une minute. Sur une base neuve, le premier worker crée la génération active ; l'index unique départage les démarrages concurrents.

**Reconstruction** (`admin rebuild`, en `tracker_admin`). Un verrou consultatif interdit deux reconstructions simultanées ; une `building` trouvée malgré tout est le reste d'un arrêt, et elle est effacée avec les `retired`. La nouvelle se remplit depuis le numéro 1, par la seule commande : le worker ne travaille que sur l'active, qui continue d'avancer et de servir les lectures. Puis, dans une transaction, les deux lignes de génération sont verrouillées (le worker attend, les lectures et l'ingestion non) : si le worker a pris de l'avance, on rattrape et on recommence ; sinon on compare et on bascule, l'active passe `retired`, la nouvelle `active`. Ce qui arrive ensuite reste au-delà des curseurs de la nouvelle active, et le worker le traite. L'ancienne est supprimée juste après, ses lignes partent en cascade. Une différence interdit la bascule sans `--allow-differences` : code de sortie non nul, numéros affichés, génération laissée à examiner jusqu'à la prochaine reconstruction.

**Comparaison.** Événements, éléments, annulations et erreurs, chaque type jusqu'au plus petit des deux curseurs. On ignore le numéro de génération, `failed_at`, `activated_at` et la version d'interprète : seul compte ce qui est produit. Les colonnes comparées sont dans `backend/src/replay.rs`.

**Nouvel essai** (`admin retry-errors`, en `tracker_admin`). Réinterprète les erreurs de l'active, une transaction par enregistrement : un succès écrit la sortie et lève l'erreur, un échec la met à jour. Le curseur ne bouge pas. Un enregistrement purgé depuis sort des erreurs.

**Annulation.** Charge utile `{"source": "<nom dans core.sources>", "key": "<clé de la cible>"}`. Le nom plutôt que le numéro de source : il est stable et lisible par le client, alors que le numéro dépend de l'ordre de création. Le moteur le résout à l'écriture ; une source inconnue est une erreur d'interprétation. Le droit d'annuler est le droit `record.cancel` sur la source, et il vaut pour toutes les sources : la web app doit pouvoir annuler un café du raccourci iOS. Une révocation ne change pas le passé, car le rejeu ne consulte pas les droits actuels.

**Compte et sessions.** `admin set-password`, en `tracker_admin`, crée ou remplace la ligne de `core.account` et vide `core.sessions` dans la même transaction. `serve`, en `tracker_app`, lit le compte et gère les sessions : 30 jours depuis la connexion, `last_seen_at` à chaque requête, lignes expirées supprimées à la connexion suivante. Un token de session vaut 32 octets aléatoires, seul son SHA-256 est stocké.

**Lecture d'un jour.** Le jour devient un intervalle UTC d'après `core.settings.timezone`, puis `derived.live_events` donne les événements. La jointure avec `core.records` et `core.sources` n'apporte que la clé et le nom de la source, pour annuler : jamais la charge utile ni le choix des événements.

**Purge** (outil d'administration, une transaction) : ajouter l'entrée au registre, rédiger les enregistrements visés, supprimer leurs lignes dérivées dans toutes les générations. La clé conservée empêche la donnée de revenir. Après une restauration, le registre des purges est réappliqué avant la remise en service.

## Tests

`db/test/run.sh` démarre un PostgreSQL 18 jetable dans Docker, crée les rôles, applique les migrations avec le rôle propriétaire, puis vérifie :

- les droits de chaque rôle et l'immuabilité du journal et du registre des purges, rôle par rôle ;
- le dédoublonnage, y compris dans un même lot et après une purge ;
- la purge : autres enregistrements intacts à l'octet près, enveloppe conservée, dérivés supprimés ;
- l'ordre des numéros sous écritures concurrentes, avec un contre-test qui montre le trou quand le verrou est désactivé ;
- l'anonymisation par suppression dans l'annuaire ;
- les générations, les annulations reçues avant leur cible et la priorité entre sources ;
- les journées au passage à l'heure d'hiver (le 25 oct. 2026 dure 25 heures) et l'heure locale d'un café ;
- une sauvegarde et sa restauration dans un conteneur neuf, où tous les tests sont rejoués.

Les migrations y sont appliquées par `psql`, une transaction par fichier. `backend/test.sh` les applique par `sqlx` (commande `migrate`) sur une base vide, et vérifie qu'un second passage ne change rien.

`compose.dev.yaml` lance le même PostgreSQL pour le développement sur le Mac, avec ses propres données (port 54329).

## Hors POC

Stockage de fichiers et références par empreinte, jalons et notifications (avec leur mémoire de ce qui a déjà été signalé), catalogue de titres, jeton de service et OAuth du serveur MCP, snapshots, export complet.
