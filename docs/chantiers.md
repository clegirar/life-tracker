# Life tracker — Chantiers après le POC

Nov 4, 2026 · @C

Le [POC](poc.md) est validé depuis le 4 nov. 2026. Chaque chantier a sa section, avec ses choix, son avancement et ses critères « Terminé quand » ; un chantier n'est fini que lorsque tous ses critères sont remplis. Les idées qui attendent sont dans [l'architecture](architecture.md) (idées de trackers, fonctionnalités futures) et les reprises dans [`TODO.md`](../TODO.md) : rien ne devient un chantier sans ma demande.

| Chantier | État |
| --- | --- |
| Garmin Connect | Fait le 5 nov. 2026 (v0.4.0) |
| Page d'état des trackers | Construite le 5 nov. 2026 (PR #13), mise en service à vérifier |
| Page Sources | Construite le 6 nov. 2026 (PR #20), sans action sur le NAS |
| YouTube | En cours depuis le 5 nov. 2026 : backend construit, web app en cours |
| Twitch | En cours depuis le 6 nov. 2026 : backend en production (#17), extension Firefox et web app construites, essai sur le vrai Twitch concluant ; restent la fusion, la signature et l'installation |
| Steam | En cours depuis le 6 nov. 2026 : backend, web app et MCP construits (branche `feat/steam`) ; essai de l'étape 0 en partie fait, mise en service à suivre |
| Twitch : import de l'historique (export RGPD) | À faire, quand je le demanderai (l'export est demandé à Twitch) |
| Ouvrir la web app sur Internet | À faire, quand je le demanderai |

## Garmin Connect

Fait le 5 nov. 2026, par ma décision, tous les critères vérifiés sur le vrai NAS (voir ci-dessous).

Premier tracker après le POC : mes pas, mon sommeil, mes séances de course et mes calories brûlées, par jour. Les choix du 4 nov. 2026 sont au journal des décisions de [l'architecture](architecture.md).

**Accès.** L'API personnelle officielle de Garmin (ouverte le 21 oct. 2026) : une clé que je crée moi-même dans Garmin Connect (Paramètres, Personal API Key, un an au plus), échangée contre un jeton de 24 h. Lecture seule, en tirage, sans mot de passe, sans MFA et sans rien d'exposé sur Internet. Un connecteur du collecteur, sur le modèle de Last.fm, avec sa propre source et son propre token.

**Historique.** Par l'API seule : elle sert tout mon historique (voir l'essai ci-dessous).

**Courses.** Un résumé par séance : distance, durée, allure, fréquence cardiaque moyenne et maximale, calories. Pas de fichier FIT tant qu'il n'y a pas de stockage de fichiers.

**Essai sur le vrai compte, le 4 nov. 2026** (structure seulement, jamais les valeurs ; clé retirée du Mac ensuite, à révoquer et recréer pour le NAS) :

- **Historique :** mes données commencent en juin 2026 et l'API les sert toutes. L'import de l'export Garmin est donc inutile.
- **Pas et calories :** `/usersummary-service/v2/dailysummaries` donne en un appel, par jour (`calendarDate`), `movement.steps.value` et `calories.burnedActive`, `burnedResting` et `burnedTotal`. Il accepte 90 jours par appel mais refuse 400 (lu à tort comme un succès lors de l'essai, corrigé le même soir : tranches de 31 jours).
- **Sommeil :** `/sleep-service/sleep/dailySleepSummariesByDate` (32 jours au plus) donne la durée (`sleepTimeSeconds`), les phases (profond, léger, paradoxal, éveil), le début et la fin en UTC. Pas de score de sommeil : ma montre n'en calcule pas. Des nuits manquent (montre pas portée) : ce sont des jours sans données, pas des nuits sans sommeil.
- **Courses :** `/activitylist-service/activities/search/activities` donne `activityType.typeKey` (`running` surtout, aussi `walking` et `cycling`), `startTimeGMT`, `distance`, `duration`, `movingDuration`, `averageSpeed`, `averageHR`, `maxHR` et `calories`. La réponse contient aussi des coordonnées GPS et mon nom : la liste blanche écarte le nom ; le GPS et le lieu des séances sont gardés depuis (voir ci-dessous).
- **Plages :** `stats/steps/daily` plafonne à 28 jours, le sommeil entre 32 et 90. Aucun en-tête de limite de débit.

**Tranché le 4 nov. 2026** (voir le journal des décisions de [l'architecture](architecture.md)) : chaque version des totaux de Garmin est un fait, sous une clé qui dépend de son contenu, et une nouvelle version part avec l'annulation de la précédente ; le sommeil est une mesure en minutes, pas une durée ; le collecteur devient « un collecteur, plusieurs connecteurs ». Ajoutés le même jour à ma demande : le GPS et le lieu des séances, leur trace tirée du fichier FIT et une carte OpenStreetMap sur la page du jour, et le lieu comme élément (« dans combien de villes ai-je couru ») ; le MCP voit les lieux, jamais les coordonnées.

**Construit le 4 nov. 2026**, sur la branche `feat/garmin-connect` : migration, interprètes, connecteur (contre un Garmin simulé, fichier FIT fabriqué par le test), routes de la carte Activité, MCP, web (carte Activité et calendrier à six métriques, captures à 390 et 1280 px en clair et en sombre). Le même jour, sur ma décision, le MCP voit aussi les détails, les coordonnées et les parcours, à travers des zones de confidentialité que je règle sur le NAS ; et l'ordre des métriques du calendrier devient cafés, écoutes, pas, séances, sommeil, calories. Reste à déployer et à vérifier sur le NAS avec une clé neuve (voir [le runbook](deploiement.md), « Mise à jour : Garmin Connect ») : les critères ci-dessous qui demandent la vraie journée, le vrai historique et l'iPhone.

**Vérifié en production le 4 nov. 2026** (v0.3.1) : premier passage de 119 appels, 400 enregistrements acceptés, 0 rejeté ; la carte de la page du jour montre le tracé, donc la CSP d'OpenStreetMap fonctionne ; `admin rebuild` donne 0 différence. J'ai décidé de **ne pas configurer de zone de confidentialité** : le MCP ne reçoit donc aucune coordonnée ni aucun parcours (le comportement sûr par défaut), et la web app n'en est pas touchée.

**Fréquence cardiaque de la nuit, ajoutée le 4 nov. 2026** à ma demande, sur la branche `feat/garmin-fc-sommeil` : pour chaque nuit, la fréquence cardiaque la plus basse et la moyenne pendant le sommeil, que le résumé de sommeil de Garmin ne donne pas (`avgHeartRate` y est vide). Essai sur le vrai compte le même jour (structure seulement) : `/wellness-service/wellness/dailyHeartRate?date=AAAA-MM-JJ` rend la série d'un jour, un échantillon toutes les deux minutes (720 par jour) du minuit au minuit de Paris, avec des vides ; un jour par appel. Une nuit chevauche deux séries : celle de la veille du réveil et celle du jour. Le connecteur garde dans la nuit les échantillons compris entre son début et sa fin ; l'interprète en tire la plus basse et la moyenne, que voient la carte Activité (« FC la plus basse 48 bpm · moyenne 55 bpm ») et le MCP. Le premier passage après le déploiement relit une fois tout le sommeil de l'historique avec sa fréquence cardiaque et annule l'ancienne version de chaque nuit (voir le journal des décisions de [l'architecture](architecture.md)).


**Fréquence cardiaque de la nuit vérifiée en production le 5 nov. 2026** (v0.4.0) : relecture unique de l'historique du sommeil passée, minimum et moyenne affichés dans la web app, `admin rebuild` à 0 différence.
**Terminé quand**

- Les pas, les calories, le sommeil et les courses d'une journée réelle correspondent à Garmin Connect, dans le calendrier comme par le MCP.
- Une révision de Garmin (synchro tardive) corrige la journée sans double compte, et `admin rebuild` donne 0 différence.
- La clé n'apparaît ni dans le dépôt ni dans les logs ; le connecteur n'a pas accès à la base.
- Tout l'historique (depuis juin 2026) est collecté, sans doublon à la relecture.
- L'affichage est vérifié à la largeur d'un iPhone et sur Mac, en clair et en sombre.

## Page d'état des trackers

Ouvert le 5 nov. 2026, à ma demande, avant YouTube : une page de la web app qui dit, tracker par tracker, s'il fonctionne et ce qu'il attend de moi. YouTube viendra s'y brancher avec son bouton « Renouveler ».

**Base existante.** `/v1/sources/health` donne déjà, par source, le dernier contact et le seuil de silence, et par métrique la fraîcheur et la couverture de chaque source (le bandeau de silence de la musique s'en sert).

**Ajout.** Chaque connecteur rapporte son état avec son heartbeat : bon, action requise ou erreur, une raison codée (jamais de donnée personnelle), une échéance éventuelle et une action éventuelle. Le backend garde le dernier état rapporté par source, hors du journal. La page montre une carte par tracker :

- **Garmin Connect :** dernière passe réussie, échéance de la clé personnelle (un an au plus ; donnée par Garmin si l'échange de la clé la renvoie, à vérifier, sinon la date que je donne à la création), et un refus `401` signalé « clé expirée ou révoquée », avec la marche à suivre. La page affiche et rappelle, sans recevoir de clé : la nouvelle se met sur le NAS comme aujourd'hui.
- **Musique (Apple Music par le scrobbler, Last.fm) :** le scrobbler est sur mon téléphone, la page ne le pilote pas ; elle montre le dernier scrobble reçu, la dernière passe du connecteur Last.fm, et alerte quand rien n'arrive plus (scrobbler arrêté).
- **Raccourci café :** le dernier contact.

**Conception, le 5 nov. 2026** (à reporter au journal des décisions de [l'architecture](architecture.md) à la construction) :

- **Deux signaux distincts.** Le dernier contact (`last_seen_at`) garde son sens : le service a bien été interrogé. Un connecteur en panne n'envoie pas de heartbeat, et le seuil de silence continue d'alerter. L'état, lui, passe par une route à part, `PUT /v1/sources/me/status`, authentifiée par le token de la source mais **sans toucher au dernier contact**. Sinon, un connecteur qui dirait « clé refusée » paraîtrait vivant.
- **Un vocabulaire fermé, aucun texte libre.** Le corps contient seulement :
  - `state` : `ok`, `action_required` ou `error` ;
  - `reason` : un code connu, par exemple `key_rejected`, `consent_expired`, `rate_limited`, `service_unavailable`, `ingest_failed` ;
  - `expires_at` (facultatif) : l'échéance de la clé ou du jeton ;
  - `next_run_at` (facultatif) ;
  - `last_collected_at` (facultatif, ajouté avec YouTube) : la dernière collecte réussie.

  Le backend refuse tout autre champ ou code. Aucun message d'erreur, aucune donnée personnelle ni aucun secret ne peut donc s'y glisser. C'est la web app qui traduit chaque code en phrase et en marche à suivre.
- **Le stockage.** Deux colonnes sur `core.sources`, par une nouvelle migration : `status jsonb` et `status_reported_at timestamptz`, avec le droit `UPDATE` sur elles seules pour `tracker_app`. Un seul état par source, le dernier, hors du journal comme le dernier contact. Une colonne `label` donne en plus le nom affiché (« Garmin Connect »), fixé par `admin create-source --label` ou `admin set-label <source> <libellé>`. Sans libellé, la page montre le nom.
- **Les connecteurs.** Last.fm et Garmin rapportent leur état à la fin de chaque passe, qu'elle réussisse ou non. Un rapport qui échoue ne fait pas échouer la passe.
  - Garmin ne connaît pas l'échéance de sa clé : le jeton échangé dure 24 h, la clé jusqu'à un an. On lui donne cette échéance par `GARMIN_KEY_EXPIRES_ON` (la date affichée par Garmin à la création de la clé), et il la rapporte. La carte prévient 14 jours avant.
- **La lecture.** `GET /v1/sources/status`, sous session, réunit par source :
  - le libellé, le dernier contact et le seuil de silence ;
  - l'état rapporté et sa date ;
  - pour chaque métrique que la source alimente, sa dernière donnée (déjà calculée pour `/v1/sources/health`).

  La page montre les sources qui ont un seuil de silence ou un état. L'import Apple, ponctuel, et la source `webapp` n'apparaissent pas.
- **La page.** Une rubrique « État » dans le menu, à côté du calendrier, avec un point quand une carte demande une action. Chaque carte montre :
  - le nom et un état en un mot (Fonctionne, À faire, En panne, Silencieux) ;
  - les dernières données, par exemple « dernier scrobble il y a 2 h » ou « dernière passe Garmin il y a 10 min » ;
  - l'échéance éventuelle ;
  - quand il le faut, la marche à suivre.

  Les cartes du jour : Garmin Connect, Musique (le scrobbler par Last.fm) et Raccourci café.

**Terminé quand**

- Chaque tracker en service a sa carte, avec un état juste : je le vérifie en coupant un connecteur sur le NAS et en le voyant passer en alerte, puis revenir.
- Un état rapporté ne contient ni secret ni donnée personnelle, et n'entre pas au journal.
- La page exige la session, comme toute route.
- La règle est écrite : tout nouveau tracker a sa carte dans cette page (principe 12 de l'architecture, `CLAUDE.md`).
- L'affichage est vérifié à la largeur d'un iPhone et sur Mac, en clair et en sombre.

**Construit le 5 nov. 2026, backend** : migration `20261105120000_source_status.sql`, `PUT /v1/sources/me/status`, `GET /v1/sources/status`, `admin create-source --label` et `admin set-label`, rapports d'état de Last.fm et de Garmin, `GARMIN_KEY_EXPIRES_ON` (voir le journal des décisions de [l'architecture](architecture.md), 5 nov. 2026). Écart : un libellé suffit aussi à faire paraître une source, pour le raccourci café, sans seuil de silence.

**Construit le 5 nov. 2026, web app** : rubrique « État » (`/etat`) dans le menu, une carte par source avec son état en un mot (Fonctionne, À faire, En panne, Silencieux), ses dernières données, l'échéance de la clé et, au besoin, « Que faire ». Un point dans le menu pour À faire et En panne, pas pour Silencieux. Une erreur rapportée avant un contact réussi plus récent est ignorée. Écart : un connecteur Last.fm silencieux veut dire que le collecteur n'atteint plus Last.fm, pas que le scrobbler s'est arrêté ; ce dernier cas est une simple note sur la carte Musique quand la dernière écoute dépasse 3 jours. Captures Playwright à 390 et 1280 px, en clair et en sombre. Reste la mise en service (`docs/deploiement.md`, « Mise à jour : page d'état des trackers »).

## Page Sources

Construite le 6 nov. 2026, à ma demande (PR #20 ; choix au journal des décisions de [l'architecture](architecture.md)). Rubrique « Sources » (`/sources`) : une fiche par tracker (comment ses données arrivent, ses clés, l'état de sa source) et « Autres clés à gérer » (MCP, web app, base, ghcr.io, déploiement, AMO). Pour chaque clé : son nom et son échéance (vivante quand le serveur la connaît, pour la clé Garmin et l'accès Google, en orange 14 jours avant, 2 jours pour Google, en rouge une fois passée) ; ni où elle vit ni comment la remplacer, à ma demande. Aucune valeur secrète. Testée par `web/src/sources.test.ts` (variables vérifiées contre le dépôt) et `web/e2e/sources.spec.ts` (aucun token de l'environnement de test dans la page). Rien à faire sur le NAS : aucun changement du backend. Désormais, tout nouveau tracker et toute nouvelle clé y ont leur fiche.

## YouTube

Ouvert le 5 nov. 2026, à ma demande. Ce que je regarde sur YouTube : les vidéos classiques vues, sans les Shorts, par chaîne et par vidéo (tops et découvertes, comme la musique), et le temps de visionnage.

**Accès.** L'API YouTube Data v3 ne donne plus l'historique de visionnage. Le scraper (Maxun ou autre) est écarté pour les raisons qui ont écarté l'accès non officiel à Garmin : il faudrait les cookies du compte Google sur le NAS, ce serait fragile et contraire aux conditions de Google. On passe par l'API Google Data Portability (`dataportability.myactivity.youtube`), dans un connecteur du collecteur avec sa source `youtube` : au plus un export toutes les 24 h. Mon client OAuth reste en mode Testing (passer en production demanderait la vérification de Google et un audit de sécurité) : le jeton expire au bout de 7 jours et je redonne mon accord chaque semaine. Un oubli ne perd rien, l'export suivant reprend depuis la dernière donnée.

**Renouveler l'accord depuis la web app** (ajouté le 5 nov. 2026, à ma demande). Quand le jeton expire, la web app affiche un bouton « Renouveler » qui ouvre la page de consentement de Google. Google renvoie sur la web app, et le collecteur reprend seul.
- Le collecteur prépare la demande (état aléatoire et vérificateur PKCE, qu'il garde) et confie seulement l'URL au backend, avec le token de sa source.
- Google renvoie un code d'usage unique sur une route de la web app, sous session. Le backend le garde quelques minutes, en mémoire, et le collecteur vient le chercher puis l'échange lui-même.
- Le jeton et le secret client ne vivent que dans le collecteur ; le code seul ne sert à rien sans le vérificateur PKCE. Rien n'entre au journal, et le collecteur ne touche pas la base.
- Le client OAuth est de type « Application Web », avec l'adresse de retour `https://<nas>.<tailnet>.ts.net:8443/autorisation` : Google l'accepte (essai du 5 nov.).
- Le cookie de session est `SameSite=Strict` : la navigation qui revient de Google arrive sans lui. Le retour atterrit donc sur une page de la web app, `/autorisation`, qui transmet ensuite le code au backend, cette fois avec la session.
- Data Portability refuse un nouvel accord tant que le précédent est actif (« Incremental auth is not allowed for the requested scopes »). Avant de redemander, le connecteur annule l'ancien par `authorization:reset` si son jeton fonctionne encore.

**Historique.** Par l'API seule : son premier export rend tout mon historique (voir l'essai ci-dessous). L'import Google Takeout, d'abord prévu, est donc retiré.

**Temps de visionnage.** Google ne donne pas le temps réellement regardé. On compte la durée de chaque vidéo, tirée de l'API Data v3 (clé publique) et entrée au journal comme enrichissement, sur le modèle de la durée des morceaux. L'interface dit que c'est la durée des vidéos. Une vidéo sans durée connue est comptée à part, jamais à zéro.

**Écartés et comptés par motif :** les publicités, les recherches, les posts consultés, les abonnements, les réactions (« Liked », « Disliked »…), les écoutes YouTube Music (déjà couvertes par la musique ; aucune dans mon historique), les Shorts et les vidéos supprimées ou privées (leur nature ne se sait plus).

**Essai sur le vrai compte, le 5 nov. 2026** (structure seulement, jamais les valeurs ; jeton et archives restés hors du dépôt) :

- **Consentement :** le scope restreint est accepté pour mon client « Desktop » non vérifié en mode Testing, une fois mon adresse ajoutée aux utilisateurs de test (sans elle : `403 access_denied`). Accès choisi : 180 jours ; Google rend quand même `refresh_token_expires_in = 604799`, soit 7 jours.
- **Export :** `portabilityArchive:initiate` sur `myactivity.youtube`, puis `archiveJobs/<id>/portabilityArchiveState` : terminé en 11 minutes environ. Deux zips sur `storage.googleapis.com`, sans redirection : `Portability/My Activity/YouTube/MyActivity.json` et un `archive_browser.json` sans intérêt.
- **Format :** une liste d'entrées au format de Takeout (`header`, `title`, `titleUrl`, `subtitles[{name, url}]`, `time`, `products`, `details`, `activityControls`, `description`). `time` en UTC ISO, à la milliseconde le plus souvent. Libellés en anglais (« Watched », « From Google Ads ») quand Takeout est en français : on reconnaît une vidéo à `titleUrl` en `/watch?v=` (id de 11 caractères, seul paramètre) et une publicité à `details`, jamais au texte. Le fichier mêle vidéos vues, recherches, posts et abonnements.
- **Historique :** 24 558 vidéos vues depuis sept. 2023, autant que Takeout (24 594). Environ 19 % des entrées sont des publicités. 2 039 vidéos vues n'ont plus de chaîne (supprimées ou privées ; leur titre est l'URL) : on garde l'id seul.
- **Stabilité :** deux exports du même jour (Takeout et Data Portability) ont 24 318 visionnages communs et diffèrent d'environ 1 % de chaque côté, sans décalage d'heure. Aucun doublon sur la paire (vidéo, instant).
- **Fréquence :** un second export dans les 24 h est refusé en `429 RESOURCE_EXHAUSTED_TIME_BASED`, avec l'heure de la prochaine autorisation (`timestamp_after_24hrs`).
- **Data v3 :** `videos.list?part=contentDetails,snippet` rend 42 vidéos sur 42 encore en ligne, aucune des 5 supprimées ; durées ISO 8601 (`PT9M9S`, `PT9S` pour les Shorts, `PT1H2M3S`). 1 unité de quota par lot de 50, sur 10 000 par jour.
- **Client Web :** un premier client Web supprimé donne `deleted_client`. Avec le client Desktop encore autorisé, le client Web reçoit « Incremental auth is not allowed » ; après `authorization:reset`, Google accepte l'adresse de retour en `ts.net` et passe à l'avertissement d'app non validée.
- **Restent à voir, par le connecteur lui-même :** le filtre `startTime` (le jeton d'essai est annulé ; les clés évitent de toute façon les doublons) et l'expiration effective du jeton au bout de 7 jours.

**Terminé quand**

- Une journée réelle correspond à mon historique YouTube, sans ses Shorts, dans le calendrier comme par le MCP.
- Tout l'historique est collecté, sans doublon à la relecture, et `admin rebuild` donne 0 différence.
- Après l'expiration du jeton, je renouvelle l'accord depuis la web app (iPhone comme Mac), et l'export suivant reprend sans trou.
- Ni clé, ni jeton, ni donnée personnelle dans le dépôt ou les logs ; le connecteur n'a pas accès à la base.
- YouTube a sa carte dans la page d'état : échéance du jeton, dernier export, prochain export permis, bouton « Renouveler ».
- L'affichage est vérifié à la largeur d'un iPhone et sur Mac, en clair et en sombre, et la durée y est présentée comme celle des vidéos.

**Construit le 5 nov. 2026, backend** : migrations `20261105130000_youtube.sql` (types `youtube.watch` et `youtube.video_info`, métriques `video.watch` et `video.duration`) et `20261105130100_item_display_search.sql` (recherche par le nom affiché), lecture en flux de l'export (`backend/src/youtube.rs`), connecteur `collector::youtube` (export de Data Portability toutes les 24 h au plus, fiches par Data v3, renouvellement anticipé par `authorization:reset`, date du dernier export reçu rapportée en `last_collected_at` et `error`/`service_unavailable` sans export depuis 48 h), `GET /v1/days/{date}/video`, le relais de l'accord en mémoire (`PUT /v1/sources/me/authorization`, `GET /v1/sources/me/authorization/code`, `POST /v1/authorizations/callback`, `authorization_url` dans `GET /v1/sources/status`) et le motif `consent_required` (voir le journal des décisions de [l'architecture](architecture.md), 5 nov. 2026). Écarts : la recherche lit aussi les noms affichés, car les clés de YouTube sont des identifiants ; le collecteur passe à 128 Mo. Reste la mise en service (`docs/deploiement.md`, « Mise à jour : YouTube »).

**Construit le 5 nov. 2026, web app** : carte YouTube du jour (vidéos, durée des vidéos dite « pas le temps regardé », vidéos sans durée à part, « Vidéo indisponible » sans lien ni chaîne, chaîne du jour, nouvelles chaînes, liens simples vers YouTube sans intégration, chacun précédé de la miniature de la vidéo, que le navigateur demande à `i.ytimg.com` sans Referer (seul ajout à la CSP) ; une journée après la dernière vue reçue dit « pas encore de données », l'export étant quotidien), carte de la page d'état (« Accès Google » à l'heure près, en couleur deux jours avant sans rendre la carte « À faire », « Prochain export », « Dernier export » (et la marche à suivre quand aucun export n'aboutit depuis 2 jours), bouton « Donner l'accord » ou « Renouveler l'accord » tiré de `authorization_url` quelle que soit la source, ouvert dans l'onglet même), page `/autorisation` (transmet `{state, code}` ou `{state, error}` une seule fois, retire le code de l'adresse, garde l'issue dans l'historique ; sans session, la connexion y ramène avec la requête), couleur `video.watch` du calendrier, Playwright `web/e2e/youtube.spec.ts`. Écart : le libellé d'une métrique garde sa majuscule de nom propre dans les phrases du calendrier (« 3 vidéos YouTube »).

**Retiré le 6 nov. 2026**, à ma demande : la section « Nouvelles chaînes » de la carte YouTube (comme pour Twitch ; la route les renvoie toujours, pour le MCP).

**Corrigé le 5 nov. 2026** : l'export de production contient aussi les réactions (« Disliked https://… », environ 90 entrées), en `/watch?v=` comme une vue ; elles étaient entrées comme vidéos vues. Une vue exige désormais un préfixe de vue connu, une réaction est écartée et la fausse vue d'avant annulée par `record.cancel` ; un export sans aucune vue reconnue n'est pas lu (voir le journal des décisions de [l'architecture](architecture.md), 5 nov. 2026, et `docs/deploiement.md`, « Correctif : les réactions YouTube »).

**Corrigé le 5 nov. 2026** : seules les vidéos classiques entrent. Le premier export a fait entrer 24 555 vues, dont ≈85 % de Shorts d'après un échantillon. Data v3 classe chaque vidéo : un Short dure 3 min au plus et son lecteur est au moins aussi haut que large ; une vidéo absente de la réponse est supprimée ou privée, écartée aussi. Les vues en attente de classement restent dans l'état du connecteur ; celles d'avant sont annulées une fois par `record.cancel` (voir le journal des décisions de [l'architecture](architecture.md), 5 nov. 2026, « vidéos classiques seulement », et `docs/deploiement.md`, même section).

## Twitch

Ouvert le 6 nov. 2026, à ma demande. Voir dans le calendrier, dès le lendemain matin, ma journée Twitch : le temps total et le temps par chaîne (les nouvelles chaînes, d'abord prévues, retirées de la carte : sans intérêt pour Twitch) ; et pouvoir demander à Claude « combien d'heures sur X ce mois-ci ». Les choix sont au journal des décisions de [l'architecture](architecture.md), 6 nov. 2026.

**Accès.** L'API Twitch (Helix) ne donne pas l'historique de visionnage d'un spectateur. Je regarde uniquement dans Firefox sur le Mac : une extension Firefox personnelle mesure la lecture en direct et l'envoie à l'API d'ingestion, avec le token de sa source `twitch`. Le téléphone et la TV sont hors champ.

**Ce qui compte.** Un live, une rediffusion ou un clip, pendant qu'il est en lecture, dans un onglet visible ou en arrière-plan, avec ou sans le son ; les publicités aussi. Une vidéo en pause ou en chargement ne compte pas. Deux streams en même temps : chaque chaîne reçoit son temps, et le total du jour est l'union des intervalles, pas leur somme.

**Mes choix** (à corriger si besoin) : seuil de silence de 3 jours (un week-end sans Firefox affichera « en attente ») ; jusqu'à l'import RGPD, presque toutes les chaînes apparaîtront comme « nouvelles » les premières semaines, sans bouton dédié ; pas de popup dans la barre d'outils en v1 ni de mise à jour automatique de l'extension (chaque version est signée puis réinstallée) ; la catégorie reste dans les détails.

**Étapes** (le backend dans sa propre PR, fusionné d'abord pour essayer l'extension contre le NAS ; l'extension et la web app ensemble, un commit chacune) :

1. **Backend** (fusionné le 6 nov. 2026, #17) : migration `20261106120000_twitch.sql` (type `twitch.watch`, métrique `stream.watch`, première de forme `duration`), interprète `twitch_watch`, sens générique de la forme `duration` (union des intervalles par jour de Paris) dans `/v1/durations`, `/v1/top` (classé par minutes) et le calendrier, `GET /v1/days/{date}/stream`, textes des outils MCP.
2. **Extension Firefox** (construite le 6 nov. 2026, `extension/`, sans release) : segments de 60 s au plus, file dans `storage.local`, heartbeat et état, page d'options ; signée par `web-ext sign` sur le Mac seulement (`extension/sign.sh`). Testée par `extension/test.sh` (page, segmenteur, file avec un faux `fetch`, lot validé contre le contrat, `web-ext lint`), puis essayée sur le vrai Twitch (ci-dessous) sans correction des sélecteurs du DOM (`SELECTORS` de `extension/src/lib/page.ts`).
3. **Web app** (construite le 6 nov. 2026) : carte Twitch du jour (temps total et temps à deux streams, chaînes par temps avec leurs contenus et leurs heures, chaîne inconnue, texte d'une journée vide), couleur fuchsia du calendrier, carte de la page d'état (dernier signal de Firefox, dernier envoi, dernier visionnage, consignes de silence et d'échec d'envoi propres à l'extension). Testée par `web/e2e/twitch.spec.ts` sur des chaînes fictives.

**Essai sur le vrai Twitch** (étape 0, fait le 6 nov. 2026 avec moi, extension temporaire chargée par about:debugging contre le NAS, source `twitch` créée après la v0.8.0). Sans aucune correction de `SELECTORS` :

- le battement passe : écriture depuis l'arrière-plan sans CORS, `Cross-Origin-Resource-Policy` de `web` sans effet, permission d'hôte sans port acceptée ;
- live : chaîne depuis l'URL, catégorie lue, segments de 60 s, la pause ferme le segment ;
- rediffusion (`/videos/<id>`) : chaîne lue dans la page, `vod` ;
- clip : chaîne et catégorie, `clip` ;
- mini-lecteur et onglet en arrière-plan : le live d'un autre onglet continue de compter sur sa chaîne pendant qu'un autre stream joue ; le total du jour en fait l'union ;
- changement de chaîne : un nouveau segment ;
- délai d'arrivée au NAS d'une à deux minutes (segment fermé au plus tard à 60 s, puis la file), l'heure enregistrée restant celle du début réel.

Non essayés : popout, clips.twitch.tv, page d'une chaîne hors ligne qui lit une rediffusion. À reprendre seulement si un écart apparaît.

**Terminé quand**

- La source est créée sur le NAS, et l'extension est signée et installée dans Firefox.
- Une vraie soirée correspond à ce que j'ai regardé, dans le calendrier comme par le MCP (`cumulative_duration` sur une chaîne).
- Deux streams en même temps donnent l'union dans le total, et chacun son temps entier.
- Une session à cheval sur minuit, et un Mac fermé en cours de stream, sont bien comptés.
- Des envois faits hors ligne rattrapent sans doublon.
- `admin rebuild` donne 0 différence.
- La carte de la page d'état passe en « silence » quand l'extension est désactivée, et revient.
- Ni token ni donnée personnelle dans le dépôt ou les logs.
- L'affichage est vérifié à la largeur d'un iPhone et sur Mac, en clair et en sombre.

## Steam

Ouvert le 6 nov. 2026, à ma demande. Suivre Steam : les heures jouées par jeu, le temps joué chaque jour sur chaque jeu, et les succès ; et pouvoir demander à Claude « combien d'heures sur X ce mois-ci ». Les choix sont au journal des décisions de [l'architecture](architecture.md), 6 nov. 2026, et dans la fiche Steam.

**Accès.** L'API Web de Steam, avec une clé personnelle (steamcommunity.com/dev/apikey) et mon identifiant Steam 64 bits, dans un connecteur du collecteur avec sa source `steam`, toutes les 15 min. `IPlayerService/GetOwnedGames` donne pour chaque jeu son `appid`, son nom, `playtime_forever` (minutes depuis toujours) et `rtime_last_played` ; `ISteamUserStats/GetPlayerAchievements` donne, jeu par jeu, chaque succès avec sa date de déblocage. Une clé Steam n'expire pas.

**Historique.** Il n'y en a pas, jour par jour : Steam ne donne que des compteurs cumulés. Le temps d'un jour se déduit de l'écart entre deux relevés successifs ; avant la mise en service, on n'aura que le total par jeu. Les succès, eux, arrivent avec leurs vraies dates, tout l'historique compris.

**Ce qui compte.** Le temps que Steam compte pour chaque jeu, hors ligne compris une fois synchronisé. L'écart entre deux relevés est placé juste avant la fin de la partie (`rtime_last_played` s'il tombe entre les deux relevés, sinon l'heure du relevé), en tranches d'une heure au plus, donc à quelques minutes près ; une partie à cheval sur minuit se répartit entre les deux jours. Les succès débloqués seulement, sans leur description.

**Mes choix** (à corriger si besoin) : seuil de silence de 6 h, comme Last.fm (le collecteur passe toutes les 15 min, qu'il y ait une partie ou non) ; pas de seuil de fraîcheur sur les métriques (une semaine sans jouer est normale) ; pas de découverte de jeu (un jeu acheté n'est pas découvert le jour où il est joué), revu le 7 nov. 2026 : un jeu joué pour la première fois après la mise en service est une découverte, un jeu déjà joué avant ne l'est jamais ; profil et « Détails des jeux » publics, nécessaires aux succès seulement ; un jeu jamais joué n'apparaît pas.

**Étapes** :

0. **Essai, avant tout code** (en partie fait le 6 nov. 2026, quelques `curl` avec ma clé, jamais dans le dépôt ; structure seulement) :
   - `GetOwnedGames` rend la liste (`appid`, `name`, `playtime_forever` en minutes, `rtime_last_played` en secondes Unix, 0 pour jamais, `playtime_disconnected` pour le hors-ligne, et des champs écartés : icône, compteurs par système, `content_descriptorids`) **même avec un profil privé**, pour ma propre clé ;
   - `GetPlayerAchievements` (`l=french`) : profil privé, `{"playerstats": {"error": "Profile is not public", "success": false}}` (d'où la raison `profile_private`) ; jeu sans succès, `success` sans liste `achievements` ; jeu avec succès, chaque succès avec `apiname`, `achieved` (1 ou 0), `unlocktime`, `name` (traduit si le jeu l'est, en anglais sinon) et `description` (écartée) ;
   - **reste à faire** : une partie d'une demi-heure avec un relevé toutes les 5 min, pour savoir si le compteur avance pendant la partie ou seulement à la fin, et si `rtime_last_played` en marque le début ou la fin. Le placement ci-dessus en dépend ; le journal garde les valeurs brutes, un rejeu le corrigera s'il le faut.
1. **Backend**, sur la branche `feat/steam` : la migration, les interprètes, le connecteur, la route `/games`, les tests et les docs.
2. **Web et MCP** : la carte du jour, la page d'état (consigne pour un profil privé), la fiche Sources, les tests de bout en bout, les captures iPhone et Mac, en clair et en sombre.
3. **Une seule PR**, `feat: tracker Steam, temps de jeu et succès`, qui emporte aussi la piste Beeper et la carte YouTube sans « Nouvelles chaînes ». Elle demande des actions sur le NAS : `compose.yaml` à recopier avant la fusion, la source, la clé (`docs/deploiement.md`, « Mise à jour : Steam »).

**Construit le 6 nov. 2026, backend** : migration `20261107120000_steam.sql` (types `steam.playtime` et `steam.achievement` ; métriques `game.play`, de forme `duration`, sa référence `game.total_playtime` et `game.achievement`), interprètes `steam_playtime` et `steam_achievement`, connecteur `collector::steam` (`STEAM_API_KEY`, `STEAM_ID`, `STEAM_TOKEN`, état `steam-state.json`), la raison `profile_private` dans le vocabulaire de l'état (contrat et `source_status.rs`), `GET /v1/days/{date}/games` sur la partie générique de la carte Twitch (`stream::measured_day`), le contrôle de santé du collecteur et le runbook. Testé contre un Steam simulé par `backend/tests/steam.rs` (points de départ, partie de 40 min, minuit, écart de 3 h en tranches, jeu acheté entre deux passages, état perdu, succès, profil privé, 401, 429, logs du vrai binaire, contrat, rejeu). Écarts : pas de `STEAM_SOURCE`, puisque le connecteur n'annule rien ; un écart de plus de deux semaines entre deux relevés ne compte aucun temps (point de départ) ; une liste de jeux sans `games` donne aussi `profile_private`. Restent la web app et le MCP (étape 2), puis l'essai de l'étape 0 et la mise en service.

**Construit le 6 nov. 2026, web app** : carte Steam du jour (`SteamCard.tsx`, `steam.ts`, `useSteam.ts`, `fetchDayGames`), montrée si « Jeux » ou « Succès » est choisi (depuis le 7 nov. 2026, si « Jeux » l'est, les succès n'ayant plus de filtre) : le temps de jeu du jour (jamais un zéro pour un jour à succès seul), le nombre de succès, puis chaque jeu avec son temps du jour, ses heures, son total Steam à la fin du jour (« 151 h au total ») et ses succès du jour avec leur heure ; un jeu qui n'a qu'un succès est listé sans temps ; chaque ligne ouvre la page du jeu sur le magasin Steam. Journée vide dite comme pour Twitch (« Pas de données Steam à cette date », « Pas encore de données pour ce jour… », « Pas de jeu ce jour-là »), alerte si le collecteur se tait. Couleur `--steam`, un vert olive (`#5f7a00`, `#bfe04d` en sombre). Page d'état : « Dernière partie », « Dernier succès », la consigne du profil privé et celle de la clé refusée (`STEAM_API_KEY`). Fiche Sources : l'arrivée par le collecteur, la clé d'API Web (sans échéance) et le token de la source. MCP : aucun code, un cas de bout en bout (`cumulative_duration`, `top`, `occurrences`). Testé par `web/e2e/steam.spec.ts` et `mcp/test/e2e.test.ts`, captures iPhone et Mac, clair et sombre. Restent l'essai de l'étape 0 et la mise en service.

**Revu le 7 nov. 2026, à ma demande** : les succès sortent des filtres et du calendrier (`game.achievement` passe à l'agrégat `none` : ni case, ni pastille, ni place dans `day_summary` ; la carte Steam, montrée si « Jeux » est choisi, garde leur nombre et leur liste par jeu ; le MCP les lit toujours par `occurrences` et `top`). Les jeux joués pour la première fois deviennent des découvertes (`game.play` avec `discovery = true`) : une section « Découvertes » dans la carte Steam, absente un jour sans nouveau jeu (ni titre ni « aucune »), et `discoveries` dans `GET /v1/days/{date}/games`. Pour qu'un jeu déjà joué avant le suivi n'en soit pas une, l'interprète déclare « déjà joué » (`game.known`, forme `declaration`, comme `music.known`) tout point de départ qui a déjà du temps : premier relevé à plus de zéro, état perdu, écart de plus de deux semaines. Un jeu à zéro au premier passage, ou acheté ensuite, devient une découverte à sa première partie. `read::day_discoveries` exclut désormais toute déclaration qui s'applique à la métrique, comme `read::discoveries` le faisait déjà. Testé par `backend/tests/steam.rs` (jeu déjà joué, jeu à zéro puis joué, jeu acheté, état perdu), `web/e2e/steam.spec.ts` (un jour à découverte, un jour sans : pas de section) et `mcp/test/e2e.test.ts`.

**Terminé quand**

- Après une vraie partie, le calendrier, la carte du jour et le MCP (`cumulative_duration`, `top`) montrent le bon temps sur le bon jeu, à quelques minutes près.
- Une partie à cheval sur minuit est répartie entre les deux jours.
- Le total d'heures de chaque jeu correspond à celui affiché par Steam.
- L'historique des succès est arrivé avec ses vraies dates, et un nouveau succès apparaît au passage suivant.
- Le NAS arrêté puis relancé ne crée aucun doublon.
- `admin rebuild` ne donne aucune différence.
- La carte d'état passe en alerte quand la clé est révoquée, puis revient.
- Aucune clé ni donnée personnelle dans le dépôt ou dans les logs.
- La carte est vérifiée aux largeurs iPhone et Mac, en clair et en sombre.

## Twitch : import de l'historique (export RGPD)

À faire, seulement quand je le demanderai. L'extension ne voit que ce que je regarde à partir de son installation ; l'export de mes données que j'ai demandé à Twitch (RGPD) devrait contenir mon historique de visionnage. Quand il arrivera : relever sa structure (jamais les valeurs), décider de ce qu'il donne vraiment (durées ou seulement des débuts de lecture), et l'importer par l'API d'ingestion comme l'import Apple, avec la réconciliation qu'il faudra (`admin set-precedence`) là où il recouvre l'extension. Les découvertes se recalculeront d'elles-mêmes.

## Ouvrir la web app sur Internet

À terme, je veux accéder à la web app sans Tailscale, avec une authentification. Il faudra alors :

- un domaine et HTTPS ;
- limiter les tentatives de connexion ;
- décider d'un second facteur ou d'une passkey ;
- revoir tout ce qui devient exposé.

L'API d'ingestion et la base restent hors de ce périmètre tant qu'aucun besoin ne les concerne.
