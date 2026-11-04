# Life tracker — Découpage du POC

Oct 31, 2026 · @C

Le POC couvre le compteur de cafés et les écoutes de musique (Last.fm en direct, export Apple pour l'historique), consultables dans une web app et depuis Claude par le serveur MCP. Les messages en sont sortis le 31 oct. 2026 : ils demandent un agent à part sur le Mac, et leur design reste dans [l'architecture](architecture.md) pour plus tard.

**Principe.** Le découpage suit des tranches verticales : chaque étape traverse toute la chaîne utile et s'arrête sur quelque chose d'utilisable, vérifié par des tests automatisés. Une étape n'est finie que lorsque tous ses critères « Terminé quand » sont remplis.

**POC validé le 4 nov. 2026**, par ma décision : l'étape 10 sort du POC et la semaine sans intervention n'est pas attendue (voir « Fin du POC »). La suite est dans les sections « Après le POC ».

**Accès.** Pendant tout le POC, la web app et l'API d'ingestion ne sont joignables que par Tailscale ; seul le serveur MCP est sur Internet. L'ouverture de la web app vient après (voir la dernière section).

| Étape | Résultat | État |
| --- | --- | --- |
| 0 | Schéma de la base et `CLAUDE.md` | Fait |
| 1 | Socle backend et ingestion | Fait |
| 2 | Interprètes et rejeu | Fait |
| 3 | Lecture et web app du café | Fait |
| 4 | Déploiement sur le NAS | Fait, raccourci iOS reporté |
| 5 | Musique en direct (Last.fm) | Fait |
| 6 | Découvertes et « déjà connu » | Fait |
| 7 | Historique Apple | Fait |
| 8 | Calendrier et complétude | Fait |
| 9 | Serveur MCP | Fait |
| 10 | Peaufiner l'interface | Sortie du POC |

## Étape 0 : schéma de la base

Fait le 17 oct. 2026 : voir [la base de données](database.md).

Le `CLAUDE.md` limité au POC est écrit depuis le 31 oct. 2026.

## Étape 1 : socle backend et ingestion

Fait le 31 oct. 2026. Les tests d'intégration tournent sur un PostgreSQL 18.6 jetable (`backend/test.sh`) plutôt que sur le Postgres de dev, en `tracker_app`.

**Contenu**

- Le crate Rust `backend/` (axum, tokio, sqlx), avec une commande `migrate`.
- Le schéma JSON du contrat, rangé dans le dépôt.
- `POST /v1/records` et `POST /v1/heartbeat`.
- L'outil d'administration : créer ou révoquer une source et son token, déclarer un type pour une source.

**Terminé quand**

- `migrate` applique les migrations existantes avec sqlx, ce qui lève le point laissé en suspens à l'étape 0.
- Chaque réponse est couverte par un test d'intégration sur le Postgres de dev : `accepted`, `duplicate`, chaque motif de `rejected`, ainsi que 401, 400 et 413.
- Un lot renvoyé en entier ne donne que des `duplicate`.
- Le dernier contact et le compteur de rejets sont mis à jour.
- Les fixtures du contrat sont validées contre le schéma JSON.
- Les logs ne contiennent aucune charge utile.

## Étape 2 : interprètes et rejeu

Fait le 31 oct. 2026. Chaque test d'interprétation a sa propre base sur le PostgreSQL 18.6 jetable : la génération active et les curseurs sont globaux.

**Contenu**

- Le moteur d'interprétation : curseurs, tout ou rien par enregistrement, table des erreurs.
- Les interprètes de `coffee.cup` et de `record.cancel`.
- Dans l'outil d'administration : `rebuild` (construction d'une génération à côté, comparaison, bascule) et `retry-errors`.

**Terminé quand**

- Un café donne un événement.
- Une annulation fonctionne, qu'elle arrive avant ou après sa cible.
- Un enregistrement en erreur n'arrête pas les suivants.
- Un rebuild produit une génération identique (règle 16 : la différence est vide).
- Une reprise après un arrêt repart au bon curseur.

## Étape 3 : lecture et web app du café

Fait le 31 oct. 2026. Tout se vérifie par `web/test.sh` et `backend/test.sh`, sur un PostgreSQL 18.6 jetable. L'affichage est contrôlé à 390 px sous Chromium ; essayé ensuite sur un vrai iPhone, en HTTPS, à l'étape 4, annulation et thèmes clair et sombre compris.

**Prérequis :** l'esquisse de la page du jour, validée le 31 oct. 2026 comme base provisoire : `docs/maquettes/page-du-jour.dc.html` ([maquette en ligne](https://claude.ai/artifact/NcMWypZr5N9VXw9dk4J6w6)). Elle sera reprise à l'étape 10.

**Contenu**

- Le compte web : Argon2id, cookie de session, mot de passe défini par l'outil d'administration.
- La source `webapp`, authentifiée par la session.
- L'API de lecture d'un jour : nombre de cafés et heures, en heure de Paris.
- La web app React (Vite, PWA) : connexion, page du jour, bouton +1 avec heure modifiable, annulation. Elle est faite avec les skills `apple-design`, `emil-design-eng` et `mobile-native`.
- Une sécurité qui ne compte pas sur Tailscale, pour que l'ouverture future ne demande pas de la refaire : toute route exige la session, le cookie est `Secure`, `HttpOnly` et `SameSite`, et les écritures sont protégées contre le CSRF.

**Terminé quand**

- Un test de bout en bout (Playwright) passe : connexion, +1, café visible, annulation.
- Un double envoi du même clic ne compte qu'un café.
- Le 25 oct. 2026 compte bien 25 heures.
- Une requête sans session est refusée sur chaque route, et le cookie porte bien ces attributs.
- L'affichage est vérifié à la largeur d'un iPhone, en clair et en sombre.

## Étape 4 : déploiement sur le NAS

Fait le 31 oct. 2026 sur le vrai NAS, en suivant le [runbook](deploiement.md), sauf le raccourci iOS, reporté à ma demande : son critère reste ouvert, la source `ios` et son token sont prêts, et les cafés se saisissent dans la web app en attendant.

**Contenu**

- Un dépôt GitHub privé, avec des images construites par GitHub Actions.
- Le compose de production, avec le service `migrate`, les contrôles de santé et le redémarrage automatique.
- Un fichier de secrets sur le NAS.
- L'accès par Tailscale.
- Le raccourci iOS « café », avec son propre token.

**Terminé quand**

- Un café saisi depuis l'iPhone, par la web app comme par le raccourci, apparaît.
- Un redéploiement garde les données, et `migrate` relancé ne change rien.
- La base n'est joignable par aucun autre conteneur ni utilisateur du NAS.

À partir de là, les vrais cafés s'accumulent.

## Étape 5 : musique en direct (Last.fm)

Fait le 1er nov. 2026 sur le vrai NAS : 399 écoutes et 219 fiches entrées, aucune erreur d'interprétation, collecteur isolé de la base, backend sans sortie Internet, et une vraie journée correspond à Last.fm. Les fixtures des tests restent reconstituées d'après la documentation de Last.fm.

**Contenu**

- Le collecteur et le connecteur Last.fm : curseur, nouveaux essais, réponse 429, signal de vie seulement après un succès.
- L'enrichissement `lastfm.track_info`.
- La fonction de normalisation.
- Les interprètes qui produisent `music.listen` et `music.track_duration`, avec leurs éléments.
- Dans l'API de lecture : nombre d'écoutes, listes des titres, artistes et albums, minutes estimées, artiste du jour.
- La santé des sources : seuil de 6 h, alerte douce après 3 jours sans écoute.
- La section musique de la page du jour.

**Terminé quand**

- Les tests passent sur des réponses Last.fm enregistrées, sans réseau.
- Une table de cas couvre la normalisation (« feat. », mentions de version, casse).
- Une synchronisation coupée puis reprise ne crée aucun doublon.
- Un connecteur muet depuis plus de 6 h est signalé.
- Sur le NAS, une vraie journée correspond à Last.fm.

## Étape 6 : découvertes et « déjà connu »

Fait le 1er nov. 2026 sur le vrai NAS : marquage, annulation et indépendance des niveaux essayés depuis le Mac, et `admin rebuild` à 0 différence (génération 2 active). Le rendu tactile sur iPhone reste à essayer, à l'étape 10.

**Contenu**

- Les vues de découverte à trois niveaux : artiste, album, titre.
- L'interprète de `music.mark_known` et son annulation.
- Les boutons « déjà connu » dans la page du jour.

**Terminé quand**

- L'exemple de Daft Punk de l'architecture passe en test.
- Un marquage n'agit qu'à son propre niveau.
- Annuler un marquage rétablit la découverte.
- Les découvertes sont identiques après un rebuild.

## Étape 7 : historique Apple

Fait le 2 nov. 2026 sur le vrai NAS, en suivant [le runbook](deploiement.md) : 60 720 lignes lues, 28 295 acceptées, 0 rejetée, et un réimport ne donne que des `duplicate` ; 16 champs au journal, 20 682 écoutes dont 99 sans artiste, aucune erreur. Last.fm fait foi depuis le 12 oct. 2026 à 11:11 UTC : 2,1 jours de chevauchement, 121 lignes Apple gardées au journal et ignorées par les vues. Sur 219 titres Last.fm, 113 sont connus d'Apple sous la même clé et 7 seulement sous un autre artiste. Découvertes reculées vérifiées dans la web app, `admin rebuild` à 0 différence (génération 4), archive supprimée du NAS et droit révoqué. Empreintes des trois fichiers dans `imports/apple-import-1.txt`, sur le NAS. Préparé la veille, sur des fixtures fictives et un essai de la vraie archive sur un PostgreSQL jetable du Mac (voir le journal des décisions de [l'architecture](architecture.md)).

**Prérequis :** avoir l'export réel, pour connaître ses colonnes.

**Contenu**

- L'outil d'import, qui lit l'archive et envoie les lignes par l'API d'ingestion.
- L'interprète de `apple.play_history_row`.
- La priorité de Last.fm à partir de sa date de démarrage.

**Simplification pour le POC.** L'archive est copiée à la main sur le NAS et son empreinte est notée ; le stockage de fichiers vient après le POC.

**Terminé quand**

- Réimporter la même archive ne donne que des `duplicate`.
- Le chevauchement reste dans le journal, mais les vues l'ignorent.
- Les découvertes reculent aux dates de l'historique.

## Étape 8 : calendrier et complétude

Fait le 2 nov. 2026 sur le vrai NAS, en suivant [le runbook](deploiement.md) : route du mois refusée sans session (401), vue du mois vérifiée depuis le Mac sur les vraies données (cafés depuis le 31 oct., écoutes depuis janv. 2022, jonction du 12 oct. sans trou ni double compte, losange « à vérifier » depuis le 1er nov.), et un café tardif sur un jour passé compté aussitôt puis annulé. La source muette et l'erreur qui marque sa journée ne sont vérifiées que par les tests : pas de coupure de 6 h du collecteur, et pas d'erreur fabriquée dans le journal de production. Préparé le même jour : sans migration, sans `rebuild` et sans changement de `compose.yaml`. Chaque critère a son test : côté backend (`backend/tests/calendar.rs`) l'erreur qui marque son jour de Paris, la source muette qui met en attente les jours non couverts jusqu'à son battement, les jours sans données avant une métrique ou entre deux sources, la métrique à vérifier, les jours de 23 et 25 heures, et un jour passé qui suit les données tardives sous `no-store` ; côté web (`web/e2e/calendar.spec.ts`) les mêmes signaux à l'écran et un mois relu après un café tardif. Affichage vérifié à 390 px sous Chromium, en clair et en sombre. Les choix (couverture, attente par source, compteur sans les rejets) sont au journal des décisions de [l'architecture](architecture.md).

**Contenu**

- La vue du mois.
- Les journées incomplètes ou à vérifier.
- Le compteur global d'erreurs.
- Les sources muettes, avec des journées « en attente ».

**Terminé quand**

- Un enregistrement en erreur marque sa journée.
- Une source muette marque les journées non couvertes.
- Les journées passées ne sont jamais mises en cache définitif.

## Étape 9 : serveur MCP

Fait le 3 nov. 2026 sur le vrai NAS, en suivant [le runbook](deploiement.md) (commit `2425449`, corrections `ba64cf3` et `8e5cf06`, tag déployé `8e5cf06`). Claude.ai n'atteint pas une URL sur le port 8443 : Funnel sert le MCP sur 443, et la web app passe en privé sur 8443. Le paquet Tailscale de Synology ne pose pas l'en-tête de Funnel : la page de connexion exige donc aussi `Tailscale-User-Login`, vérifié depuis le Mac (page de consentement) et l'iPhone en 4G sans Tailscale (403). Le formulaire échouait sur `Origin: null` sous `no-referrer` : politique passée en `same-origin`. Démo dans Claude.ai sur les vraies données : un titre écouté 9 fois ces 30 derniers jours (du 22 oct. au 2 nov.) ; cette semaine, depuis le lundi 2 nov., 35 min et 14 écoutes (31 min et 13 écoutes lundi, 3 min et 1 écoute aujourd'hui) ; un artiste découvert le 15 avr. 2022 (historique Apple), 1 502 écoutes depuis ; un album écouté pour la dernière fois le 14 août 2026 (81 jours), 115 écoutes depuis juin 2022. Les journaux du MCP ne contiennent ni jeton, ni code, ni adresse : méthode, chemin, statut et durée, plus « login succeeded » et « tokens issued ». **Réserve** : la question du premier café moyen n'a pas pu être posée, faute de vrais cafés saisis ; elle est couverte par le test de bout en bout, et l'étape est validée quand même. Les écarts de l'usage réel seront remontés. Préparé le 2 nov. 2026 : une nouvelle migration (`core.service_tokens`), sans `rebuild`. Chaque critère a ses tests. **Aucun appel MCP ne peut écrire** : côté backend (`backend/tests/read_api.rs`), le jeton de service est refusé sur toute route d'écriture (café, annulation, « déjà connu », signal de vie, déconnexion, et toute méthode autre que GET sur les routes de lecture), sans repli sur la session, et le journal ne bouge pas ; côté MCP, les dix outils sont tous annotés en lecture seule et `mcp/test.sh` vérifie que le journal ne contient que le jeu de données après tous les appels. **Le serveur ne répond à rien sans OAuth** (`mcp/test/oauth.test.ts`) : 401 et `WWW-Authenticate` sans jeton, jeton expiré, autre audience, client inconnu, retour hors liste, PKCE faux ou absent, code et rafraîchissement rejoués, connexion refusée par Funnel, CSRF, limitation des essais. **Les questions** (`mcp/test/e2e.test.ts`) : le flux OAuth complet, puis le client officiel du SDK, dans l'ère 2025 comme dans l'ère 2026-07-28, répond aux quatre questions sur la musique et au premier café moyen, contre le vrai backend et un PostgreSQL jetable. Les choix (exposition, OAuth, jeton de service, lectures génériques, puis les corrections du 3 nov.) sont au journal des décisions de [l'architecture](architecture.md).

**Prérequis :** trancher l'authentification du MCP (OAuth et jeton de service). Tranché le 2 nov. 2026 : voir le journal des décisions de [l'architecture](architecture.md).

**Contenu**

- Le serveur en TypeScript avec le SDK officiel, en lecture seule.
- L'accès à l'API de lecture par jeton de service, au niveau « pseudonymes ».
- Les outils listés dans l'architecture.
- L'exposition sur Internet.

**Terminé quand**

- Claude.ai répond aux quatre questions sur la musique et à « à quelle heure je prends mon premier café en moyenne ».
- Aucun appel MCP ne peut écrire.
- Le serveur ne répond à rien sans OAuth.

## Étape 10 : peaufiner l'interface

Sortie du POC le 4 nov. 2026. La liste des fonctionnalités de [l'architecture](architecture.md) (idées de trackers et fonctionnalités futures) suffit comme prérequis, sans nouvelle maquette d'ensemble. Ce qui reste à reprendre, à commencer par la carte Musique de la page du jour, est dans [`TODO.md`](../TODO.md), hors POC.

Le 3 nov. 2026, sur la branche `feat/interface-etape-10`, trois demandes faites et vérifiées par Playwright à 390 et 1280 px, en clair et en sombre : une barre latérale (fixe sur Mac, tiroir sur iPhone) qui accueillera les fonctionnalités suivantes et n'a pour l'instant que « Calendrier », des adresses pour chaque page (Retour et rechargement fonctionnent), une vue de l'année (`/v1/years/{AAAA}`, douze mini-mois dont l'intensité suit la métrique choisie) et une carte Musique triée par un seul sélecteur, découvertes comprises, avec « Tout afficher » au-delà de dix lignes. Plusieurs métriques s'affichent en points de couleur par jour, comme dans Calendrier d'Apple, dans le mois comme dans l'année. Déployé en v0.2.0 le 4 nov. 2026 (PR #7), et la vue de l'année essayée au toucher sur iPhone le même jour. La carte Musique de la page du jour ne me plaît pas encore : notée dans `TODO.md`. Les choix sont au journal des décisions de [l'architecture](architecture.md).

**Prérequis :** la liste de toutes les fonctionnalités que j'ai en tête, y compris après le POC, que je donnerai en début d'étape.

**Contenu**

- Repenser la maquette en tenant compte de ces fonctionnalités, pour que l'interface puisse les accueillir sans refonte.
- Peaufiner toute l'interface : page du jour, calendrier, animations et détails, avec les skills de la section « Interface » de l'architecture.

**Terminé quand**

- La nouvelle maquette est validée par moi.
- La web app la suit, à la largeur d'un iPhone et sur Mac, en clair et en sombre.
- Les tests de bout en bout passent toujours.

## Fin du POC

Les étapes 1 à 10 sont faites, et le café comme la musique ont tourné une semaine sur le NAS sans intervention.

Validé le 4 nov. 2026 par ma décision : les étapes 1 à 9 sont faites, l'étape 10 sort du POC, et je n'attends pas la semaine sans intervention (v0.2.0 en production depuis le même jour).

## Après le POC : Garmin Connect

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

**Terminé quand**

- Les pas, les calories, le sommeil et les courses d'une journée réelle correspondent à Garmin Connect, dans le calendrier comme par le MCP.
- Une révision de Garmin (synchro tardive) corrige la journée sans double compte, et `admin rebuild` donne 0 différence.
- La clé n'apparaît ni dans le dépôt ni dans les logs ; le connecteur n'a pas accès à la base.
- Tout l'historique (depuis juin 2026) est collecté, sans doublon à la relecture.
- L'affichage est vérifié à la largeur d'un iPhone et sur Mac, en clair et en sombre.

## Après le POC : ouvrir la web app sur Internet

À terme, je veux accéder à la web app sans Tailscale, avec une authentification. Il faudra alors :

- un domaine et HTTPS ;
- limiter les tentatives de connexion ;
- décider d'un second facteur ou d'une passkey ;
- revoir tout ce qui devient exposé.

L'API d'ingestion et la base restent hors de ce périmètre tant qu'aucun besoin ne les concerne.
