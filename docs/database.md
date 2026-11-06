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
6. **Les éléments d'un événement sont une table générique.** Une écoute a un artiste, un album et un titre ; un message a un contact. Chacun devient une ligne de `derived.event_items` (niveau, position, clé normalisée, nom affiché). Un niveau peut avoir plusieurs éléments, rangés par `position` : une écoute crédite chacun de ses artistes, l'artiste principal en 0 ; elle reste un seul événement, et tout ce qui compte des écoutes compte des événements, jamais des éléments. Les outils Occurrences, Découverte, Dernière fois, Classement et Rechercher travaillent tous sur cette table, indexée une fois pour toutes : un nouveau domaine n'ajoute ni table ni index (principe 12).
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
| `sources` | Registre des sources : nom stable, authentification (token ou session de la web app), token haché, seuil de silence (vide : jamais muette), dernier contact, version du client, rejets, nom affiché (`label`), dernier état rapporté et sa date (`status`, `status_reported_at`, pour la page d'état). |
| `source_kinds` | Les types que chaque source a le droit d'envoyer. |
| `kinds` | Les types de données : version de format courante, interprète avec ou sans mémoire. |
| `metrics` | Registre des métriques : forme, unité, domaine, agrégat du jour, niveaux d'éléments, découverte, métrique éclairée, source de la durée, seuil de fraîcheur, ordre d'affichage, champs des détails partagés avec les lectures génériques (`shared_details`). |
| `source_precedence` | Quelle source fait foi, par métrique et par période. |
| `people`, `pseudonyms` | L'annuaire : une personne et son nom affiché, ses pseudonymes (numéro, e-mail). |
| `settings` | Une ligne : date de mise en ligne, fuseau des journées (`Europe/Paris`). |
| `account`, `sessions` | Le compte unique de la web app et ses sessions. |
| `service_tokens` | Les jetons de service de l'API de lecture (le serveur MCP) : nom, SHA-256 du jeton, création, révocation. |

### `derived` : recalculable

| Table | Contenu |
| --- | --- |
| `generations` | Les générations et leur état, avec la version de chaque interprète. |
| `cursors` | Par génération et par type : dernier numéro du journal traité. |
| `interpretation_errors` | Les enregistrements en erreur, avec le message et la version de l'interprète. |
| `events` | Les événements normalisés : métrique, source, date et décalage, valeur, détails. |
| `event_items` | Les éléments de chaque événement, un ou plusieurs par niveau, rangés par `position` (clé : génération, numéro, rang, niveau, position). |
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
| `garmin.daily_summary` | Connecteur Garmin | `steps`, `calories.burned` |
| `garmin.sleep` | Connecteur Garmin | `sleep` |
| `garmin.activity` | Connecteur Garmin | `workout` |
| `garmin.activity_track` | Connecteur Garmin | `workout.track` |
| `youtube.watch` | Connecteur YouTube | `video.watch` |
| `youtube.video_info` | Connecteur YouTube | `video.duration` |
| `twitch.watch` | Extension Firefox (Twitch) | `stream.watch` |

| Métrique | Forme | Agrégat du jour | Niveaux | Particularités |
| --- | --- | --- | --- | --- |
| `coffee` | compteur | nombre | aucun | Chaque café garde son heure |
| `music.listen` | événement riche | nombre | artiste, album, titre | Découverte ; durée tirée de `music.track_duration` ; à vérifier après 3 jours sans écoute |
| `music.known` | déclaration | aucun | artiste, album, titre | Éclaire `music.listen` |
| `music.track_duration` | référence | aucun | titre | Valeur : durée en secondes |
| `steps` | mesure | dernière | aucun | Pas du jour ; à vérifier après 3 jours sans donnée |
| `calories.burned` | mesure | dernière | aucun | Calories du jour (kcal) ; détails : actives, au repos |
| `sleep` | mesure | dernière | aucun | Minutes de sommeil, au jour du réveil ; détails : phases, fréquence cardiaque la plus basse et moyenne ; à vérifier après 3 jours |
| `workout` | événement riche | nombre | type, lieu | Valeur : distance en km |
| `workout.track` | référence | aucun | aucun | Éclaire `workout` : la trace, lue par la carte seulement |
| `video.watch` | événement riche | nombre | chaîne, vidéo | Découverte ; durée tirée de `video.duration` (celle de la vidéo, pas le temps regardé) ; à vérifier après 3 jours |
| `video.duration` | référence | aucun | vidéo | Valeur : durée de la vidéo en secondes |
| `stream.watch` | durée | somme (minutes de l'union) | chaîne | Découverte ; temps mesuré (`played_ms`), l'union des intervalles de chaque jour ; pas de seuil de fraîcheur |

Seule la source `webapp` est créée par la migration. Les sources à token (connecteur Last.fm, raccourci iOS, import Apple) sont créées par l'outil d'administration, pour qu'aucun secret ne soit dans le dépôt.

Les messages sont hors POC depuis le 31 oct. 2026 : leur type et leur métrique arriveront par une migration, avec l'agent Mac. L'annuaire (`people`, `pseudonyms`) est déjà en place et testé pour les accueillir, mais rien ne s'en sert pendant le POC.

**Clés et charges utiles Last.fm.** Écoute : `horodatage:SHA-256(artiste, titre)`, charge utile : le scrobble brut, date en `Z` (décalage inconnu). Fiche : SHA-256 de la clé du titre normalisée, charge utile `{"artist", "track", "info"}` : le titre tel que scrobblé et la réponse brute de `track.getInfo`. Une fiche sans durée (zéro ou absente) ne produit aucun événement. Dans les clés d'éléments, artiste et titre sont séparés par une tabulation, qu'aucun nom normalisé ne peut contenir. Pas de migration à l'étape 5 : les registres prévoyaient déjà ces types et métriques.

**Clés et charges utiles Garmin** (migration `20261104120000_garmin.sql`, après le POC). Clé : le sujet, puis les 16 premiers hexadécimaux du SHA-256 de la charge utile, sérialisée clés triées : `summary:<date>:…`, `sleep:<date>:…`, `activity:<id>:…`, `track:<id>:…` (suivie de `:2`, `:3`… si ce contenu a déjà été annulé). Une révision de Garmin est un nouvel enregistrement, suivi dans le même lot d'un `record.cancel` de la version précédente, clé `cancel:<clé annulée>` : `live_events` n'en garde qu'une, sans rien de propre à Garmin. Charges utiles, en liste blanche : résumé `{calendar_date, steps, distance_m, calories_total, calories_active, calories_resting}`, nuit `{calendar_date, start, end, sleep_seconds, deep_seconds, light_seconds, rem_seconds, awake_seconds, heart_rate}` (`heart_rate` facultatif, format 1 inchangé : `[[secondes depuis le début, bpm], …]`, les échantillons de la nuit, absent sans échantillon et dans les nuits d'avant le 4 nov. 2026), séance `{activity_id, type, start, duration_s, moving_s, distance_m, avg_speed_mps, avg_hr, max_hr, calories, start_lat, start_lon, end_lat, end_lon, location_name}`, trace `{activity_id, points}` (`[lat, lon, secondes depuis le début]`, 1 000 points au plus). `occurred_at` : midi heure de Paris pour un résumé et une nuit, le début de la séance pour une séance et sa trace. Les détails reprennent ces champs, sauf la série de fréquence cardiaque d'une nuit, qui reste au journal : ses détails en gardent la plus basse (`lowest_hr`) et la moyenne arrondie (`avg_hr`), partagées avec les lectures génériques (migration `20261104130000_sleep_heart_rate.sql`). Les lectures génériques n'en rendent que ceux de `core.metrics.shared_details` (migration `20261104120100_metric_order_and_shared_details.sql`), et, pour le jeton de service, les coordonnées à travers les zones de confidentialité (`backend/src/privacy.rs`) ; les routes de session de la carte Activité lisent tout (`backend/src/activity.rs`). Pas de table ni d'index de plus : `details->'activity_id'` relie une trace à sa séance, sur quelques dizaines de lignes par mois.

**Clés et charges utiles YouTube** (migration `20261105130000_youtube.sql`). Vue : clé `<id de la vidéo>:<instant en ms>`, charge utile en liste blanche `{video_id, title, channel_id, channel_name}` (titre et chaîne `null` pour une vidéo supprimée ou privée), `occurred_at` l'instant de la vue en UTC, à la milliseconde. Fiche : clé `<id de la vidéo>`, charge utile `{video_id, duration_s, title, channel_id, channel_name}` tirée de l'API Data v3 ; sans durée (ou `P0D`, un direct), aucun événement. Les éléments ont pour clé les identifiants de YouTube (chaîne, vidéo), et pour nom ce qu'affiche la source (l'id d'une vidéo sans titre). Les lectures génériques partagent `title` et `channel_name`. La recherche lit aussi les noms affichés, par l'index `event_items_display_trgm` sur `lower(display)` (migration `20261105130100_item_display_search.sql`) : une clé qui est un identifiant ne se cherche pas par son nom.

**Clés et charges utiles Twitch** (migration `20261106120000_twitch.sql`). Un segment de lecture mesuré par l'extension Firefox : clé UUID créée par l'extension, `occurred_at` le début du segment avec le décalage local, charge utile en liste blanche `{channel, channel_name, content, duration_ms, category}` (login en minuscules ou `null`, `live`/`vod`/`clip`, de 1 à 3 600 000 ms). L'élément `channel` a pour clé le login et pour nom le nom affiché (le login s'il manque) ; une chaîne `null` n'a pas d'élément. Les détails gardent `channel`, `channel_name`, `content`, `category` et `played_ms` ; les lectures génériques partagent tout sauf `channel`, déjà la clé de l'élément. La métrique `stream.watch` est la première de forme `duration` : son temps se calcule à la lecture, l'union des intervalles `[occurred_at, +played_ms]` de chaque jour de Paris (`range_agg` sur `tstzrange`), sans table, vue ni index de plus ; un événement dure 1 h au plus, et les lectures cherchent ceux commencés jusqu'à 1 h avant leur période, par l'index `(generation, metric, occurred_at)`.

**L'état rapporté et le libellé d'une source** (migration `20261105120000_source_status.sql`, page d'état des trackers). Trois colonnes de `core.sources`, hors du journal comme le dernier contact : `status`, un objet jsonb (la base le vérifie ; l'API, le vocabulaire fermé), le dernier rapporté par `PUT /v1/sources/me/status` ; `status_reported_at` ; `label`, le nom affiché, 80 caractères au plus et sans espace autour, fixé par l'outil d'administration. `tracker_app` n'a `UPDATE` que sur `status` et `status_reported_at` : ni le libellé, ni le seuil, ni le token. Un seul état par source : l'historique des états n'est pas un fait de ma vie.

**Un marquage « déjà connu ».** Charge utile `{"level", "artist", "album", "track"}` : le niveau et les noms tels qu'affichés, `album` ou `track` seulement au niveau qui les nomme. Il donne un événement `music.known` et ses éléments, à son seul niveau, sous la même clé qu'une écoute : un artiste « A & B » marque A et B, un album ou un titre prend l'artiste principal. Les détails gardent les noms. Pas de migration à l'étape 6 : les registres prévoyaient déjà le type, la métrique et le droit de `webapp`, et l'index des éléments suffit.

**Clés et charges utiles Apple.** Un enregistrement par ligne `PLAY_END` audio avec une lecture, écoute ou non : c'est l'interprète qui décide. Clé : `play_end:<début en ms>:<fin en ms>:<position de début>:<position de fin>`, sans `Event ID` ni nom ni artiste déduit, pour qu'un export ultérieur retombe dessus. Charge utile (liste blanche, jamais la ligne telle quelle) : `song`, `album`, `artist`, `artist_source`, `started_at`, `ended_at`, `play_duration_ms`, `media_duration_ms`, `start_position_ms`, `end_position_ms`, `end_reason`, `media_type`, `item_type`, `utc_offset_seconds`, `offline`, `archive_sha256`. `artist` est déduit par l'outil, et `artist_source` dit comment : `library_album`, `library_album_day`, `daily_hour`, `daily_day`, `daily_title`, `library_title`, ou `ambiguous` et `unknown` avec un artiste vide. Une écoute sans artiste a des détails à clés vides et aucun élément : elle compte et se liste, mais n'entre ni dans les artistes, ni dans les albums, ni dans les découvertes. Pas de migration à l'étape 7 : les registres prévoyaient le type, la métrique et `core.source_precedence`.

**L'heure d'un café** est celle de l'enveloppe (`occurred_at`), fixée par le client : « maintenant » par défaut, modifiable pour un café noté après coup. La charge utile reste vide. Une heure erronée se corrige par une annulation et une nouvelle saisie.

**Les détails d'une écoute** gardent les noms affichés (`artist`, le crédit complet, `album`, `track`), les artistes crédités (`artists`, `[{key, name}]`, l'artiste principal en premier), les clés normalisées (`artist_key` = artiste principal, `album_key` = artiste principal et album, `track_key` = artiste principal et titre) et, si l'export Apple la fournit, la durée réellement écoutée (`played_ms`). La clé d'un album inclut l'artiste principal, sinon tous les « Greatest Hits » seraient confondus ; « A & B » et « A & C » partagent ainsi l'album de A, et « A & B – T » est le même titre que « A – T (feat. B) ». Les artistes crédités viennent de `music::credited_artists` (voir l'architecture, journal du 3 nov. 2026). Les minutes d'écoute se calculent en vue : la durée écoutée si elle existe, sinon la dernière durée connue du titre.

## Rôles et droits

| Rôle | Sert à | Peut |
| --- | --- | --- |
| `tracker_owner` | Les migrations, et rien d'autre | Tout, sauf ce que les triggers refusent |
| `tracker_app` | Le backend à l'exécution | Ajouter et lire le journal ; mettre à jour le dernier contact, les rejets et l'état rapporté des sources ; gérer l'annuaire et les sessions ; lire les jetons de service ; tout dans `derived` |
| `tracker_admin` | L'outil d'administration, sur le NAS uniquement | Lire tout `core` ; gérer les registres et les jetons de service ; rédiger une purge (charge utile et référence seulement) ; tout dans `derived` |

Les rôles et la base sont créés par `db/init/01-roles.sh` à la création du volume ; les mots de passe viennent de l'environnement.

**Migrations.** En production, un service `migrate` à usage unique du docker-compose applique les migrations avec le rôle propriétaire, puis le backend démarre avec `tracker_app`. Le backend ne connaît donc jamais le mot de passe du propriétaire, qui est le seul à pouvoir désactiver un trigger : c'est la porte de secours assumée, et elle ne sert qu'aux migrations.

## Cycles de vie

**Ingestion.** Une transaction par lot : `INSERT … ON CONFLICT (source_id, key) DO NOTHING RETURNING key`. Une clé absente de la réponse est un `duplicate`. Les rejets de l'enveloppe se décident avant l'insertion et incrémentent le compteur de la source.

**Interprétation.** Une tâche de fond de `serve`, en `tracker_app`. Une transaction par lot de 100 au plus, type après type : verrou partagé sur la génération active, verrou exclusif sur le curseur du type (qui sérialise les workers), pour chaque enregistrement un point de sauvegarde avec ses événements et éléments ou son erreur, puis le curseur. Un enregistrement produit donc tout ou rien, et une coupure reprend au curseur. Un enregistrement purgé avance le curseur sans rien produire. La base refuse une sortie (classes 22 et 23, ou 54000 pour un nom trop long pour l'index des éléments) : c'est une erreur d'interprétation, avec le code seul. Toute autre erreur de la base est une panne : rien n'est écrit, nouvel essai après 1 s, 2 s, 4 s… jusqu'à une minute. Sur une base neuve, le premier worker crée la génération active ; l'index unique départage les démarrages concurrents.

**Reconstruction** (`admin rebuild`, en `tracker_admin`). Un verrou consultatif interdit deux reconstructions simultanées ; une `building` trouvée malgré tout est le reste d'un arrêt, et elle est effacée avec les `retired`. La nouvelle se remplit depuis le numéro 1, par la seule commande : le worker ne travaille que sur l'active, qui continue d'avancer et de servir les lectures. Puis, dans une transaction, les deux lignes de génération sont verrouillées (le worker attend, les lectures et l'ingestion non) : si le worker a pris de l'avance, on rattrape et on recommence ; sinon on compare et on bascule, l'active passe `retired`, la nouvelle `active`. Ce qui arrive ensuite reste au-delà des curseurs de la nouvelle active, et le worker le traite. L'ancienne est supprimée juste après, ses lignes partent en cascade. Une différence interdit la bascule sans `--allow-differences` : code de sortie non nul, numéros affichés, génération laissée à examiner jusqu'à la prochaine reconstruction.

**Comparaison.** Événements, éléments, annulations et erreurs, chaque type jusqu'au plus petit des deux curseurs. On ignore le numéro de génération, `failed_at`, `activated_at` et la version d'interprète : seul compte ce qui est produit. Les colonnes comparées sont dans `backend/src/replay.rs`.

**Nouvel essai** (`admin retry-errors`, en `tracker_admin`). Réinterprète les erreurs de l'active, une transaction par enregistrement : un succès écrit la sortie et lève l'erreur, un échec la met à jour. Le curseur ne bouge pas. Un enregistrement purgé depuis sort des erreurs.

**Annulation.** Charge utile `{"source": "<nom dans core.sources>", "key": "<clé de la cible>"}`. Le nom plutôt que le numéro de source : il est stable et lisible par le client, alors que le numéro dépend de l'ordre de création. Le moteur le résout à l'écriture ; une source inconnue est une erreur d'interprétation. Le droit d'annuler est le droit `record.cancel` sur la source, et il vaut pour toutes les sources : la web app doit pouvoir annuler un café du raccourci iOS. Une révocation ne change pas le passé, car le rejeu ne consulte pas les droits actuels.

**Compte et sessions.** `admin set-password`, en `tracker_admin`, crée ou remplace la ligne de `core.account` et vide `core.sessions` dans la même transaction. `serve`, en `tracker_app`, lit le compte et gère les sessions : 30 jours depuis la connexion, `last_seen_at` à chaque requête, lignes expirées supprimées à la connexion suivante. Un token de session vaut 32 octets aléatoires, seul son SHA-256 est stocké.

**Lecture d'un jour.** Le jour devient un intervalle UTC d'après `core.settings.timezone`, lu en sous-requête et jamais en jointure (cette table d'une ligne, jamais analysée, ferait choisir un mauvais plan), puis `derived.live_events` donne les événements. La jointure avec `core.records` et `core.sources` n'apporte que la clé et le nom de la source, pour annuler : jamais la charge utile ni le choix des événements.

**Lecture des découvertes.** Calculée à chaque lecture dans `GET /v1/days/{date}/music`, jamais stockée. Pour chaque élément écouté ce jour-là : aucune écoute vivante du même niveau et de la même clé avant minuit, heure de Paris. Le champ `first_listens` liste ces éléments avec leurs `known_marks` ; c'est une découverte si cette liste est vide (`discoveries` les compte). Toutes les lectures fixent le numéro de génération active, ce qui ouvre les index, et le pool coupe le JIT : voir le journal des décisions de l'architecture.

**Jetons de service** (étape 9, migration `20261102120000_service_tokens.sql`). Le serveur MCP lit l'API de lecture avec un jeton à lui, au niveau « pseudonymes ». Une table à part plutôt qu'une source à token : une source écrit au journal et a un dernier contact, un jeton de service ne fait que lire, et rien ne doit pouvoir lui accorder un type. `admin create-service-token` le crée (seul le SHA-256 est gardé, comme les tokens de source) ; `revoke-service-token` pose `revoked_at`. `tracker_app` ne peut que le lire : le backend vérifie un jeton, sans jamais en créer.

**Lectures génériques** (étape 9, `backend/src/read.rs`). Inventaire, recherche, occurrences, durée cumulée, un élément, classement, découvertes : calculées à chaque lecture sur `derived.live_*`, comme le reste, la métrique en paramètre et ses niveaux, sa découverte et la source de sa durée lues dans `core.metrics`. La recherche passe par l'index `gin_trgm_ops` de `event_items` (clé qui contient le texte normalisé, ou mots proches) ; les autres par l'index `(generation, metric, level, item_key, occurred_at)`. Pas de table ni d'index de plus.

**Lecture d'un mois** (`GET /v1/months/{AAAA-MM}`). Calculée à chaque lecture, jamais stockée, sans table ni index de plus. Les valeurs du mois groupent `derived.live_events` par jour de Paris (`occurred_at AT TIME ZONE`, donc des jours de 23 et 25 heures justes) ; les erreurs du jour viennent de `derived.interpretation_errors` joint au journal par son numéro, pour la date de l'enregistrement ; la couverture de chaque métrique, source par source, est le minimum et le maximum de ses événements vivants ; les sources muettes suivent la règle de `/v1/sources/health`. Le détail des règles est dans le journal des décisions de l'architecture. Pas de migration à l'étape 8.

**Priorité entre sources** (`admin set-precedence`, en `tracker_admin`). Une ligne de `core.source_precedence` par métrique, remplacée dans une transaction ; aucun rejeu, `live_events` suit à la lecture. La date n'est jamais dans le dépôt : elle vient de la première donnée de la source dans la base, sauf si un écart de plus de 7 jours la sépare de la suivante, auquel cas `--from` est exigé.

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

Stockage de fichiers et références par empreinte, jalons et notifications (avec leur mémoire de ce qui a déjà été signalé), catalogue de titres, snapshots, export complet. Les jetons OAuth du serveur MCP ne sont pas dans la base : seul le backend la touche, et ils vivent hachés dans le volume du conteneur MCP.
