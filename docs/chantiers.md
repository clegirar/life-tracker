# Life tracker — Chantiers après le POC

Nov 4, 2026 · @C

Le [POC](poc.md) est validé depuis le 4 nov. 2026. Chaque chantier a sa section, avec ses choix, son avancement et ses critères « Terminé quand » ; un chantier n'est fini que lorsque tous ses critères sont remplis. Les idées qui attendent sont dans [l'architecture](architecture.md) (idées de trackers, fonctionnalités futures) et les reprises dans [`TODO.md`](../TODO.md) : rien ne devient un chantier sans ma demande.

| Chantier | État |
| --- | --- |
| Garmin Connect | Fait le 5 nov. 2026 (v0.4.0) |
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

## Ouvrir la web app sur Internet

À terme, je veux accéder à la web app sans Tailscale, avec une authentification. Il faudra alors :

- un domaine et HTTPS ;
- limiter les tentatives de connexion ;
- décider d'un second facteur ou d'une passkey ;
- revoir tout ce qui devient exposé.

L'API d'ingestion et la base restent hors de ce périmètre tant qu'aucun besoin ne les concerne.
