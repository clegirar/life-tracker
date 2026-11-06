# Life tracker — Architecture

Oct 12, 2026 · @C

## Vision et périmètre

Une app auto-hébergée sur mon NAS qui retrace ma vie jour par jour : un calendrier où je vois ce que j'ai fait par domaine, et un serveur MCP pour interroger ces données depuis Claude.

Les données arrivent par trois types de sources :

- **Manuelle** : je déclare moi-même (un bouton qui incrémente un compteur).
- **API** : un service externe interrogé régulièrement (historique d'écoute).
- **Appareil** : un agent sur une de mes machines qui envoie ce qu'il observe.

**Périmètre du POC** : compteur de cafés et écoutes Apple Music (via Last.fm et l'export Apple). Le découpage en étapes est dans [le POC](poc.md).

**Hors POC** : les messages de l'iPhone (sortis du POC le 31 oct. 2026, car ils demandent un agent à part sur le Mac ; leur design reste décrit ici), les messageries tierces (WhatsApp, Signal…) et tous les autres trackers. L'architecture doit pourtant les accueillir sans refonte.

## Vue d'ensemble

Trois sources alimentent une API d'ingestion unique sur le NAS ; tout est écrit dans le journal brut, puis traduit en événements normalisés ; l'API de lecture les interroge pour le calendrier et pour Claude. Seule la base stocke quelque chose.

&#91;embedded content: architecture du POC · 3 sources, 1 journal, 2 lecteurs\]

Les référentiels (annuaire, registres des sources et des métriques) éclairent la lecture sans jamais toucher au journal.

## Principes structurants

Ces règles s'appliquent à toutes les sources, présentes et futures.

1. **Une seule porte d'entrée.** Toute donnée, quelle que soit sa source, entre par la même API d'ingestion.
2. **Le journal brut fait foi.** Il est immuable : on ajoute, on ne modifie ni ne supprime jamais, à la seule exception de la purge exceptionnelle.
3. **Tout le reste est recalculable.** C'est une chaîne : les événements normalisés (et les statuts d'interprétation) sont stockés, mais peuvent être effacés et reconstruits depuis le journal ; les agrégats et les vues sont recalculés à chaque lecture depuis ces événements, et ne sont donc jamais stockés.
4. **Les interprètes sont déterministes.** Rejouer le journal donne toujours le même résultat : pas de dépendance à l'heure actuelle ni d'appel externe. Un enrichissement externe est lui-même enregistré dans le journal.
5. **On corrige en ajoutant.** Une erreur se corrige par un enregistrement d'annulation, jamais par un effacement.
6. **Journal immuable, référentiels modifiables, vues recalculées.** Ce qui doit pouvoir changer (noms des contacts, identité d'un titre) vit hors du journal, dans des référentiels modifiables.
7. **La politique de données se décide par source.** Les sources sensibles sont minimisées avant d'entrer ; les autres gardent un brut riche.
8. **Un fait sur un moment va dans un interprète ; un fait relatif à tout l'historique va dans une vue.** « J'ai écouté X à 14h » est figé ; « c'est la première fois » dépend de tout le passé et doit rester recalculable.
9. **Une seule porte de sortie.** Toute lecture passe par l'API de lecture, jamais directement par la base. Les règles de visibilité vivent ainsi à un seul endroit.
10. **Un manque de données se voit.** Une journée sans données n'est jamais confondue avec une journée sans activité : santé des sources, journées incomplètes ou à vérifier, outil Complétude pour Claude.
11. **Le moins exposé possible.** Pendant le POC, seul le serveur MCP est sur Internet, en lecture seule ; Claude ne voit que des pseudonymes ; rien de personnel dans les logs ni les notifications. Par défaut, chaque donnée va le moins loin possible.
12. **Ajouter un tracker, c'est déclarer, pas reconstruire.** Une nouvelle source se déclare dans les registres, avec son interprète et éventuellement un connecteur, sans rien changer au journal, aux outils MCP ni au calendrier générique. Un tracker qui exigerait une refonte est le signal qu'il faut repenser quelque chose. Tout tracker a aussi sa carte dans la page d'état des trackers : s'il fonctionne et ce qu'il attend de moi (décidé le 5 nov. 2026).
13. **Le plus simple d'abord.** À chaque choix, la solution la plus simple qui respecte les principes, en gardant la porte ouverte : vues à la volée, pas de catalogue, outils MCP ajoutés au besoin, pas de snapshots pour l'instant. Ce sont des choix pour aujourd'hui, pas des interdits : quand les volumes ou les besoins changeront, des snapshots ou des vues pré-calculées pourront devenir le bon choix.
14. **Pensé pour durer des décennies.** Technologies éprouvées, formats ouverts, export complet de mes données. L'app pourra être réécrite un jour ; mes données doivent lui survivre.

## Modèle en trois couches

Trois couches de données, de la vérité à la lecture : le journal brut (1), les événements normalisés (2) et l'API de lecture (3). Les interprètes font le passage de 1 à 2 ; les référentiels, les tables d'exploitation et le stockage de fichiers vivent à côté.

| Élément | Contenu | Modifiable | Recalculable |
| --- | --- | --- | --- |
| Journal brut (couche 1) | Un enregistrement par arrivée de données : numéro d'ordre global, source, type de donnée et version de format, clé de dédoublonnage, date de l'événement (UTC et décalage d'origine), date de réception, charge utile telle qu'envoyée | Non : ajout seul, la purge étant la seule exception | Non, c'est la vérité |
| Interprètes (logique, rien de stocké) | Un par type de donnée, versionné ; transforme du brut en 0, 1 ou n événements | Oui, par nouvelle version | Sans objet |
| Événements normalisés (couche 2) | Métrique, date (UTC), valeur, détails ; lien vers l'enregistrement brut et la version d'interprète | Non directement | Oui, entièrement |
| API de lecture (couche 3) | Agrégats par jour, découvertes, records, calculés à la volée et jamais stockés ; seul accès du calendrier et du MCP | Sans objet | Rien à recalculer |
| Référentiels | Registre des métriques, registre des sources, annuaire des contacts, paramètres (date de mise en ligne) ; catalogue des titres après le POC | Oui | Non |
| Tables d'exploitation | Statuts d'interprétation, registre des purges | Statuts : oui ; purges : ajout seul | Statuts : oui, par rejeu |
| Stockage de fichiers | Gros fichiers (exports, plus tard photos), référencés par le journal avec leur empreinte | Non | Non |

**Type de donnée et métrique, deux niveaux.** Le type de donnée décrit ce qu'une source envoie, côté journal ; la métrique décrit un fait de ma vie, côté événements. Un interprète passe de l'un à l'autre, et plusieurs types peuvent donner la même métrique : `lastfm.scrobble` et `apple.play_history_row` produisent tous deux des écoutes. Les déclarations (`music.mark_known`, `record.cancel`) ne sont pas des métriques : elles changent la façon dont les vues lisent les autres événements.

**Deux familles d'interprètes.** Certains traitent chaque enregistrement isolément (un clic café donne un événement). D'autres ont besoin des enregistrements précédents de la même source, par exemple une source qui n'enverrait que des photos successives d'une liste, à comparer pour en déduire les nouveautés. Le mécanisme de recalcul rejoue ces derniers dans l'ordre, source par source. Aucun dans le POC.

**Le format du brut évoluera.** Chaque enregistrement porte une version de format ; les anciens formats sont convertis vers le format actuel avant l'interprétation (voir les règles d'event sourcing).

**Formes de métriques** (registre des métriques) : compteur, événement riche, durée, état ou valeur ponctuelle. Chaque métrique y déclare son type, son unité, son domaine et sa façon de s'agréger sur une journée, pour que le calendrier l'affiche sans code spécifique. Sa forme décide aussi quels outils MCP s'y appliquent.

**Registre des sources** : chaque source a une identité, un token stocké haché, les types de données qu'elle a le droit d'envoyer, son seuil de silence, sa date de dernier contact, la version de son client et son compteur de rejets. C'est ce qui permet de détecter une source muette plutôt que de croire à une journée vide.

**Collecte des API.** Un seul collecteur héberge un connecteur par service (Last.fm, Garmin Connect et YouTube aujourd'hui, d'autres demain).

- Le collecteur gère le commun : planification, curseurs de reprise, nouveaux essais, signaux de vie, envoi à l'API d'ingestion.
- Un connecteur ne sait faire qu'une chose : interroger son service et rendre du brut.
- Chaque connecteur a son token et apparaît comme une source distincte dans le registre ; une panne de l'un n'arrête pas les autres.
- Le collecteur passe par l'API d'ingestion comme n'importe quelle source, jamais directement par la base. Un connecteur lourd ou instable peut donc être sorti dans son propre conteneur sans rien changer d'autre.

## Règles d'event sourcing

Ces règles, ajoutées à la revue du document, rendent les principes vérifiables dans la pratique.

1. **Deux dates et un ordre.** Chaque enregistrement porte la date de l'événement à la source et sa date de réception, plus un numéro d'ordre global attribué par le journal. Les vues se basent sur la première, le rejeu sur le numéro d'ordre.
2. **Une clé d'idempotence pour chaque donnée, même manuelle.** Un clic café n'a pas d'identifiant naturel : le bouton web ou le raccourci iOS génère un identifiant unique par clic, sinon un double envoi compte deux cafés.
3. **Une annulation désigne ce qu'elle annule.** Elle référence la source et la clé de l'enregistrement d'origine.
4. **L'immuabilité est garantie par la base, pas par la discipline.** Le compte de l'application ne peut qu'ajouter dans le journal ; seul le compte d'administration de la purge peut rédiger.
5. **L'ingestion accepte des lots.** Chaque enregistrement d'un lot est dédoublonné individuellement, donc renvoyer un lot entier après une coupure ne crée aucun doublon.
6. **Une jointure est un fait relatif.** Tout ce qui croise plusieurs enregistrements (une écoute et la durée du titre) se calcule en vue, pas dans un interprète.
7. **Le passé peut changer d'apparence.** Des données tardives (rattrapage du Mac, import de l'export Apple) modifient les vues de jours anciens : c'est normal, et le calendrier ne met jamais une journée passée en cache définitif.
8. **Pas de snapshots pour l'instant.** Avec quelques centaines d'événements par jour, rejouer tout le journal reste rapide. Ce n'est pas définitif : quand les volumes grandiront ou qu'un rejeu deviendra lent, les snapshots pourront devenir le bon choix.
9. **Des faits, pas des états.** On enregistre « +1 café », jamais « 4 cafés aujourd'hui » : un total se calcule, il ne se stocke pas. Seule exception, les mesures (un poids, une humeur), qui sont elles-mêmes des observations datées.
10. **Le temps reste en UTC, le jour se calcule à la lecture.** Chaque date est stockée en UTC, avec le décalage horaire d'origine quand la source le donne. Le découpage en journées (minuit, heure de Paris) se fait dans l'API de lecture, jamais figé dans le journal ni dans les événements.
11. **Les clés de dédoublonnage sont rangées par source.** L'unicité porte sur le couple source et clé, pour que deux sources qui utiliseraient le même identifiant ne s'écrasent jamais.
12. **Un convertisseur par ancien format.** Plutôt que des interprètes qui gèrent toutes les versions passées, chaque ancien format est d'abord converti vers le format actuel ; l'interprète ne connaît que la dernière version.
13. **Les interprètes retiennent leur position.** Ils avancent dans l'ordre des numéros du journal et mémorisent le dernier traité, pour reprendre exactement où ils s'étaient arrêtés.
14. **Un rejeu ne déclenche rien.** Notifications, jalons et rappels ne sont jamais émis par un interprète : reconstruire la base ne doit pas renvoyer « ton 1 000e café ». Ils naissent d'un mécanisme à part, qui mémorise ce qu'il a déjà signalé.
15. **Les gros fichiers restent hors du journal.** Une photo ou un export volumineux va dans un stockage de fichiers sur le NAS ; le journal garde sa référence et son empreinte, qui prouve que le fichier n'a pas changé.
16. **Le rejeu complet est un test régulier.** Reconstruire depuis zéro à côté et comparer avec l'existant : un écart révèle un interprète qui n'est pas déterministe.
17. **Les référentiels changent l'affichage, pas l'histoire.** Renommer un contact s'applique rétroactivement dans les vues, c'est voulu ; le journal, lui, reste intact.

## Contrat de l'API d'ingestion (v1)

Le contrat est décrit une fois dans un schéma JSON rangé dans le dépôt ; le backend, l'agent Mac et tous les clients sont testés contre lui. Les noms de champs sont en anglais, comme le code. Validé le 13 oct. 2026, y compris la règle : tout type de donnée doit être déclaré pour sa source dans le registre, sinon l'enregistrement est rejeté.

**Points d'entrée**

| Appel | Rôle |
| --- | --- |
| `POST /v1/records` | Envoyer un lot d'enregistrements (500 au plus, 1 Mo au plus) |
| `POST /v1/heartbeat` | Signal de vie quand il n'y a rien à envoyer ; ne va pas dans le journal |
| `PUT /v1/sources/me/status` | Le dernier état de la source pour la page d'état (vocabulaire fermé, 204) ; ne va pas dans le journal et ne compte pas comme un contact |
| `PUT /v1/sources/me/authorization` | L'URL de consentement OAuth qu'un connecteur a préparée, montrée par la page d'état (204) ; en mémoire seulement, une heure au plus ; ne compte pas comme un contact |
| `GET /v1/sources/me/authorization/code` | Le code (ou le refus) que Google a renvoyé à la web app pour cette source, une seule fois ; en mémoire seulement ; ne compte pas comme un contact |

Authentification : `Authorization: Bearer <token>`. Le token identifie la source : la source n'est jamais écrite dans le corps, une source ne peut donc pas se faire passer pour une autre. Les tokens sont stockés hachés dans le registre des sources. Le client envoie sa version dans l'en-tête `X-Client-Version`. Tout appel authentifié met à jour la date de dernier contact, sauf le rapport d'état ; un connecteur n'envoie donc son signal de vie qu'après avoir réussi à interroger son service. Exception à l'authentification par token : la web app passe par la session de connexion, qui vaut pour la source « web app », pour qu'aucun token ne soit jamais exposé dans le navigateur.

**Enveloppe d'un enregistrement**

| Champ | Contenu |
| --- | --- |
| `key` | Clé d'idempotence, unique pour la source (un identifiant naturel, ou un UUID généré par le client) |
| `kind` | Type de donnée, déclaré pour cette source dans le registre (par exemple `coffee.cup`) |
| `schema_version` | Version du format de la charge utile pour ce type |
| `occurred_at` | Date de l'événement à la source, au format RFC 3339 avec son décalage horaire (`2026-10-13T09:42:10+02:00`, ou `Z` si la source ne le connaît pas) |
| `payload` | La charge utile, telle que la source la produit |

Le serveur ajoute lui-même la source (d'après le token), la date de réception et le numéro d'ordre global.

**Réponse.** Statut HTTP 200 avec un résultat par enregistrement : `accepted` (écrit au journal), `duplicate` (déjà reçu, compte comme un succès) ou `rejected` avec un motif (type non déclaré pour la source, champ manquant, date à plus d'un jour dans le futur). Une erreur sur le lot entier renvoie 401 (token), 400 (JSON illisible) ou 413 (lot trop gros), et rien n'est écrit.

**Un rejet est définitif.** Le renvoyer échouerait pareil : la source le met de côté localement, continue avec la suite et le signale comme une erreur. `duplicate` et `accepted` font avancer le curseur de la source.

**Types de données du POC**

Messages : hors POC depuis le 31 oct. 2026, la ligne `message` est gardée pour plus tard.

| Source | Type | Clé | Charge utile |
| --- | --- | --- | --- |
| Web app, raccourci iOS | `coffee.cup` | UUID généré au clic | Vide : un enregistrement par café |
| Web app | `music.mark_known` | UUID | Niveau (artiste, album, titre), artiste, album et titre tels qu'affichés |
| Web app, raccourci iOS | `record.cancel` | UUID | La source (son nom dans le registre) et la clé de l'enregistrement annulé : `{"source": "webapp", "key": "…"}` |
| Agent Mac | `message` | GUID du message | Sens, service (iMessage, SMS, RCS), pseudonyme du contact ; groupes filtrés par l'agent |
| Connecteur Last.fm | `lastfm.scrobble` | Horodatage + artiste + titre | Le scrobble tel que renvoyé par l'API |
| Connecteur Last.fm | `lastfm.track_info` | Artiste + titre normalisés | La fiche du titre renvoyée par l'API, dont sa durée (enrichissement) |
| Import Apple | `apple.play_history_row` | Instants et positions de la lecture | Une ligne `PLAY_END`, réduite à une liste blanche de champs musicaux (voir le journal des décisions), avec l'artiste déduit et sa méthode |
| Connecteur Garmin | `garmin.daily_summary` | `summary:<date>:<empreinte du contenu>` | Pas, distance, calories totales, actives et au repos du jour (liste blanche) |
| Connecteur Garmin | `garmin.sleep` | `sleep:<date>:<empreinte>` | Durée, phases, début et fin de la nuit, au jour du réveil, et les échantillons de fréquence cardiaque entre son début et sa fin (liste blanche) |
| Connecteur Garmin | `garmin.activity` | `activity:<id>:<empreinte>` | Une séance : type, durées, distance, allure, cardio, calories, coordonnées de début et de fin, lieu (liste blanche, sans nom ni propriétaire) |
| Connecteur Garmin | `garmin.activity_track` | `track:<id>:<empreinte>` | La trace simplifiée de la séance, tirée du fichier FIT : `[lat, lon, secondes]` |
| Connecteur Garmin | `record.cancel` | `cancel:<clé annulée>` | L'annulation de la version précédente d'un résumé, d'une nuit, d'une séance ou d'une trace, envoyée avec la nouvelle |
| Connecteur YouTube | `youtube.watch` | `<id de la vidéo>:<instant en ms>` | Une vidéo vue de l'export Data Portability : id, titre, id et nom de la chaîne (liste blanche ; ni publicité, ni recherche, ni post, ni abonnement) |
| Connecteur YouTube | `youtube.video_info` | `<id de la vidéo>` | La fiche de la vidéo par l'API Data v3, dont sa durée (enrichissement) |
| Extension Firefox (Twitch) | `twitch.watch` | UUID créé par l'extension | Un segment de lecture : login et nom de la chaîne, contenu (`live`, `vod`, `clip`), durée jouée en ms (1 h au plus), catégorie (liste blanche ; ni URL, ni titre, ni chat) |

L'export Apple suit la règle des gros fichiers : pendant le POC, ses fichiers sont copiés à la main sur le NAS le temps de l'import, et son empreinte est notée dans chaque charge utile ; un outil d'import envoie les lignes par lots par l'API, comme n'importe quelle source.

## Fiches par source

Chaque source a sa fiche : ce qu'on veut en tirer décide ce que le brut contient et ce qui est filtré avant d'entrer. Messages : hors POC depuis le 31 oct. 2026.

|  | Café | Messages | Musique |
| --- | --- | --- | --- |
| Type de source | Manuelle | Appareil (agent Mac) | API + export ponctuel |
| Ce que je veux savoir | Nombre de cafés par jour | Nombre de messages envoyés et reçus, et avec qui | Temps d'écoute par jour, titres, artistes, albums, découvertes |
| Sensibilité | Faible | Forte | Faible |
| Contenu du brut | Un clic +1, ou une annulation | Métadonnées : identifiant du message, date, service (iMessage, SMS, RCS), sens (envoyé ou reçu), pseudonyme du contact | Last.fm : brut riche, tel que fourni. Export Apple : liste blanche de champs musicaux |
| Filtré avant d'entrer | Rien | Le texte et l'identité en clair du contact | Export Apple : tout le reste (IP, ville, coordonnées, Apple ID, appareils, abonnement) |
| Interprète | Sans mémoire | Sans mémoire | Sans mémoire (Last.fm fournit des écoutes horodatées) |
| Référentiel | Aucun | Annuaire des contacts | Aucun pour le POC |
| Point d'attention | Bouton web ou raccourci iOS | Captation via `chat.db` sur le Mac, en mode rattrapage | Temps d'écoute estimé ; découverte calculée en vue |

**Messages.** L'iPhone n'est pas lisible directement ; avec Messages dans iCloud et le transfert des SMS, le Mac reçoit tout l'historique. L'agent tourne quand le Mac est allumé et reprend où il s'était arrêté : les dates restent exactes, seule la journée en cours peut être en retard. L'iPhone conserve les messages indéfiniment. Le journal ne contient jamais de nom : l'annuaire relie chaque pseudonyme à un nom affiché, que je saisis moi-même, et supprimer une ligne de l'annuaire anonymise l'historique de cette personne. Claude ne voit que les pseudonymes, jamais les noms. Le pseudonyme est une empreinte à clé (HMAC) du numéro ou de l'adresse, normalisé avant calcul (+33612… et 0612… donnent le même) ; un simple hachage serait réversible en essayant tous les numéros. Une même personne peut avoir plusieurs pseudonymes (numéro, e-mail) : l'annuaire en relie plusieurs à une seule personne.

**Musique.** Une « découverte » est un titre écouté pour la première fois de ma vie ; un artiste découvert se calcule de la même façon. Pour le POC, pas de catalogue : un titre est identifié par son artiste principal et son titre normalisés (minuscules, sans « feat. » ni mention de version). Une écoute crédite chacun de ses artistes : ceux du nom d'artiste (« A, B & C », « A x B », « A feat. B »…) et les invités cités dans le titre (« T (feat. D) ») ; elle compte pour chacun, mais reste une seule écoute et un seul titre. Le premier nommé est l'artiste principal, qui donne les clés d'album et de titre. Les vrais groupes dont le nom contient un séparateur (« Simon & Garfunkel ») sont listés dans le code et ne se coupent jamais. Un catalogue pourra venir plus tard, et les découvertes se recalculeront. Le temps d'écoute demande la durée de chaque titre, obtenue par un enrichissement enregistré dans le journal ; comme il croise deux enregistrements, il se calcule en vue. Si l'export Apple fournit la durée réelle d'écoute, la métrique sera plus précise avant le démarrage de Last.fm qu'après. Source en direct : Last.fm, alimenté par un scrobbler (FastScrobbler, validé en test, la même app sur iPhone et Mac). iOS pouvant suspendre l'app en arrière-plan, quelques écoutes peuvent arriver en retard ou manquer ; l'app rattrape en partie via l'historique Apple Music, et un raccourci y lance une analyse de l'historique chaque jour à 8h et 20h. L'export de données Apple (privacy.apple.com) fournit l'historique passé, en une fois.

**Marquer comme déjà connu.** Sans catalogue, tout titre jamais vu dans le journal est une découverte. Depuis le calendrier, je peux marquer un artiste, un album ou un titre « déjà connu » : c'est une déclaration de ma part, donc un enregistrement de la source manuelle dans le journal (comme un clic café), annulable par une annulation. Les vues des découvertes excluent les éléments marqués, chacun à son niveau. Le marquage garde l'artiste et le titre tels qu'affichés, pas seulement la clé normalisée, pour rester valable si la normalisation ou un futur catalogue change.

**Trois découvertes, trois marquages.**

- **Artiste découvert** : la première fois que j'écoute un artiste que je n'avais jamais écouté.
- **Album découvert** : la première fois que j'écoute un titre d'un album dont je n'avais jamais rien écouté.
- **Titre découvert** : la première fois que j'écoute un titre.

Je peux marquer un artiste, un album ou un titre « déjà connu » ; il ne compte alors plus comme découverte à son niveau. Chaque marquage n'agit qu'à son propre niveau : marquer un artiste ne dit rien de ses albums ni de ses titres, et marquer un album ne dit rien de ses titres.

Exemple : je marque Daft Punk « déjà connu », puis j'écoute pour la première fois un de leurs titres. Daft Punk n'est pas un artiste découvert, mais ce titre est bien un titre découvert.

Limite connue sans catalogue : les noms viennent tels que la source les envoie. Les crédits se coupent par des règles et une liste d'exceptions, pas par un catalogue : un vrai groupe absent de la liste (« Mumford & Sons ») se coupe à tort jusqu'à ce qu'on l'y ajoute, suivi d'un `admin rebuild`. Une compilation se range par artiste principal, donc se coupe en plusieurs albums, et un même album peut exister sous deux noms (« Album » et « Album — Single »).

### Musique : ce que montre la web app

Par jour :

- nombre de titres écoutés ;
- liste complète des titres, des artistes et des albums ou EP ;
- minutes d'écoute (estimées à partir de la durée des titres).

Ajouts retenus :

- **découvertes du jour** (titres, albums, artistes), avec le bouton « déjà connu » à côté de chacune ; un seul sélecteur Titres, Artistes, Albums trie à la fois les découvertes et les listes ;
- **artiste du jour** : le plus écouté ;
- l'indicateur de journée incomplète ou à vérifier.

Répartition dans la journée et retrouvailles : non retenues.

### Musique : ce que je demande à Claude

- Combien de fois j'ai écouté tel titre ces X derniers jours ?
- Combien de minutes d'écoute cette semaine ?
- Quand ai-je découvert tel artiste, album ou titre ?
- Depuis quand je n'ai pas écouté tel artiste, album ou titre ?

Outils MCP proposés pour y répondre, Claude composant le reste :

| Outil | Répond à (exemples) | Portée |
| --- | --- | --- |
| Inventaire | Ce qui est tracké : métriques, unités, sources, depuis quand, et quels outils s'appliquent à chacune. Claude l'appelle d'abord pour savoir quoi demander | Tous domaines |
| Rechercher | Retrouver un élément à partir d'un nom approximatif (artiste, album, titre) | Tous domaines |
| Occurrences | Les occurrences d'une métrique ou d'un élément sur une période, avec leur heure locale. Trois formes : la liste, le nombre par jour, la première et la dernière de chaque jour (écoutes d'un titre ; heure du premier café de chaque jour) | Tous domaines |
| Durée cumulée | Temps passé sur une période, par jour, semaine ou mois (minutes d'écoute) | Métriques qui ont une durée |
| Découverte | La première fois de ma vie : date de première occurrence ; si marqué déjà connu : « connu avant le » + date de mise en ligne (quand j'ai découvert un artiste) | Domaines où la découverte a un sens |
| Dernière fois | Date de la dernière occurrence d'un élément (dernière écoute d'un album) | Tous domaines |
| Classement | Éléments les plus fréquents sur une période (top artistes) | Tous domaines |
| Découvertes | Liste des découvertes sur une période | Domaines où la découverte a un sens |
| Résumé d'une journée | Tout ce que montre le calendrier pour un jour | Tous domaines |
| Complétude | Journées incomplètes, sources muettes ou à vérifier, pour que Claude nuance ses réponses | Tous domaines |

Les outils sont génériques : ils prennent la métrique en paramètre, et le registre des métriques dit lesquels s'appliquent à chacune (une durée cumulée n'a pas de sens pour le café). Un nouveau domaine ne demande donc aucun nouvel outil, sauf calcul vraiment propre à lui.

Claude compose le reste lui-même : « à quelle heure je prends en moyenne mon premier café sur une semaine » = Occurrences sous la forme « première de chaque jour » sur sept jours, puis une moyenne.

**Faire évoluer les outils.** Le serveur MCP n'est qu'une fine couche au-dessus de l'API de lecture : ajouter un outil ne touche ni au journal ni aux interprètes. Règle : on ajoute un outil quand une vraie question échoue, pas par anticipation, car trop d'outils rendent le choix plus difficile pour Claude.

La date de mise en ligne de l'app est un paramètre fixé une fois, rangé avec les référentiels.

### Garmin Connect : la fiche

Premier tracker après le POC (voir le journal des décisions, 4 nov. 2026).

|  | Garmin Connect |
| --- | --- |
| Type de source | API (l'API personnelle de Garmin, une clé à moi échangée contre un jeton de 24 h) |
| Ce que je veux savoir | Pas, calories brûlées, sommeil et ses phases, séances (course surtout) avec leur parcours ; dans combien de villes j'ai couru |
| Sensibilité | Moyenne à forte : santé, et positions des séances, qui situent le domicile (d'où les zones de confidentialité pour le MCP) |
| Contenu du brut | Listes blanches champ par champ : totaux du jour, nuit avec les échantillons de fréquence cardiaque de son sommeil (`[secondes depuis le début, bpm]`, tirés des séries `dailyHeartRate` de deux jours), résumé de séance avec ses coordonnées de début et de fin et son lieu, trace simplifiée |
| Filtré avant d'entrer | Nom de la séance, nom et identifiants du propriétaire, `userProfilePk`, `uuid`, les échantillons de fréquence cardiaque hors de la nuit, et tout champ non listé ; le fichier FIT lui-même (pas de stockage de fichiers) |
| Interprète | Sans mémoire. Une révision arrive en nouvel enregistrement, avec l'annulation du précédent |
| Référentiel | Aucun |
| Point d'attention | Les totaux sont des états révisés par Garmin ; une journée sans montre n'est pas une journée à zéro pas |

**Métriques.** `steps` (Pas), `calories.burned` (Calories brûlées) et `sleep` (Sommeil, en minutes) sont des mesures du jour, agrégées en `last` : la valeur vivante du jour. `workout` (Séances) compte les séances, par type (`activity_type`) et par lieu (`place`, la ville de Garmin) ; sa valeur est la distance en km. `workout.track` est une référence : la trace, que seule la carte lit.

**Dans la web app.** La carte Activité de la page du jour : pas et distance, calories (actives, au repos), sommeil avec ses heures, sa fréquence cardiaque la plus basse et moyenne et ses phases, puis chaque séance avec son type en français, son heure, son lieu, sa distance, sa durée, son allure (course) ou sa vitesse, sa fréquence cardiaque moyenne et maximale, et sa carte OpenStreetMap (tracé, départ, arrivée). Un jour sans données le dit, jamais « 0 pas ».

**Pour Claude.** Les outils génériques, plus `route` : `day_summary` donne les mesures du jour avec leurs détails (phases du sommeil et sa fréquence cardiaque la plus basse et moyenne, jamais la série ; calories actives et au repos) et chaque séance (heure, type, lieu, distance, durée, allure, cardio, coordonnées de début et de fin), `top` sur `place` répond « dans combien de villes ai-je couru », `route` donne le parcours d'une séance. Toute coordonnée passe par les zones de confidentialité : dedans, masquée, et une trace y est coupée ; sans zone configurée, aucune coordonnée ni aucun parcours. La web app, elle, voit tout.

### Twitch : la fiche

En cours depuis le 6 nov. 2026 (voir [les chantiers](chantiers.md) et le journal des décisions, 6 nov. 2026).

|  | Twitch |
| --- | --- |
| Type de source | Appareil : une extension Firefox à moi, sur le Mac, qui mesure la lecture en direct (l'API Twitch ne donne pas l'historique d'un spectateur) |
| Ce que je veux savoir | Le temps passé sur Twitch par jour, le temps par chaîne, les nouvelles chaînes ; « combien d'heures sur X ce mois-ci » |
| Sensibilité | Faible à moyenne : ce que je regarde, et quand |
| Contenu du brut | Un segment de lecture : login et nom affiché de la chaîne, contenu (live, rediffusion, clip), durée jouée, catégorie |
| Filtré avant d'entrer | Tout le reste : URL, titre du stream, chat, pseudo du compte, cookies ; rien n'est lu hors des pages de Twitch |
| Interprète | Sans mémoire : un segment donne un événement de forme `duration` |
| Référentiel | Aucun |
| Point d'attention | Deux streams en même temps : chaque chaîne a son temps, le total du jour est l'union ; le téléphone et la TV ne sont pas suivis ; l'historique passé viendra de l'export RGPD (chantier à part) |

**Ce qui compte.** Un live, une rediffusion ou un clip, pendant qu'il est en lecture, onglet visible ou en arrière-plan, avec ou sans le son. Une vidéo en pause ou en chargement ne compte pas, et les publicités comptent (c'est du temps passé sur le stream).

**Métrique.** `stream.watch` (« Twitch »), de forme `duration`, en minutes, agrégat `sum`, niveau `channel` (clé : le login), découverte, sans seuil de fraîcheur, ordre 80, détails partagés `channel_name`, `content`, `category`, `played_ms`. Le temps d'un jour est l'union des segments, coupée au minuit de Paris ; celui d'une chaîne, l'union de ses seuls segments.

**Dans la web app** (à venir) : la carte Twitch du jour sur `GET /v1/days/{date}/stream` (temps total, temps à deux streams, chaînes par temps, contenus, nouvelles chaînes) et sa carte dans la page d'état (dernier signal de Firefox).

**Pour Claude.** Les outils génériques : `cumulative_duration` (en tout, ou `level=channel` et `key=<login>`), `top` classé par minutes, `discoveries`, `day_summary` (les minutes du jour).

## Réconciliation multi-sources

Quand deux sources décrivent la même réalité, chaque fait risque d'exister deux fois ; la réconciliation se fait dans les vues, jamais dans le journal.

Premier cas : l'export Apple et le tracking en direct se recouvrent sur une période. Le cas reviendra avec d'autres trackers (pas comptés par l'iPhone et par une montre), d'où un mécanisme générique. Deux approches possibles, à choisir pour chaque nouveau cas :

- **Source prioritaire par période** : sur un intervalle donné, une seule source fait foi pour une métrique.
- **Rapprochement** : deux faits identiques à quelques minutes près sont fusionnés.

**Décision pour la musique : Last.fm fait foi sur le chevauchement.** L'export Apple n'est utilisé que pour la période antérieure au démarrage de Last.fm. Les écoutes de l'export qui chevauchent restent dans le journal, mais les vues les ignorent. La date se déclare par `admin set-precedence`, sur le NAS : elle n'est jamais dans le dépôt.

## Exposition et accès

Seul le serveur MCP est exposé sur Internet, car Claude.ai l'appelle depuis ses propres serveurs ; tout le reste reste privé derrière Tailscale.

| Composant | Joignable depuis | Protection |
| --- | --- | --- |
| Web app (calendrier) | Mes appareils, via Tailscale (`tailscale serve` privé sur le port 8443 depuis le 3 nov. 2026) | Réseau privé + connexion (un seul compte) |
| API d'ingestion | Agents et raccourcis, via Tailscale | Un token par source |
| Serveur MCP | Internet, par Tailscale Funnel sur le port 443 (depuis le 3 nov. 2026) ; sa page de connexion, depuis le tailnet seulement | HTTPS + OAuth (serveur d'autorisation intégré, Claude seul client) ; pseudonymes uniquement ; en lecture seule |
| Base de données | Le NAS uniquement | Non exposée |

Cette exposition vaut pour le POC. À terme, la web app sera elle aussi ouverte sur Internet, avec une authentification : voir [les chantiers](chantiers.md). Sa sécurité ne compte donc jamais sur Tailscale, et toute route exige la session.

Le NAS est partagé avec d'autres personnes : les données du tracker doivent rester isolées de leurs comptes.

Le calendrier et le serveur MCP passent par la même API de lecture, pendant de l'API d'ingestion : l'une écrit, l'autre lit. Elle applique deux niveaux de visibilité (noms de l'annuaire pour le calendrier, pseudonymes seuls pour le MCP) et n'est jamais exposée sur Internet : seul le serveur MCP l'est, et il l'appelle en interne sans accès direct à la base.

## Procédure de purge exceptionnelle

La purge est la seule exception à l'immuabilité : elle efface le contenu d'enregistrements bruts, en garde une trace, et ne passe jamais par l'API d'ingestion ni par le MCP.

**Quand l'utiliser :**

- une donnée n'aurait jamais dû entrer (un bug de l'agent Mac envoie le texte d'un message) ;
- l'abandon définitif d'une source et de son historique ;
- l'effacement de toute trace liée à une personne, comptages compris.

Oublier un contact au quotidien ne demande pas de purge : supprimer sa ligne dans l'annuaire anonymise déjà son historique.

**Mécanisme retenu : rédaction en place.** La charge utile est remplacée par un marqueur « purgé » ; l'enveloppe reste (identifiant, source, date, clé de dédoublonnage). Le journal garde sa forme, les interprètes ignorent ces enregistrements, et la clé conservée empêche une source de renvoyer la donnée purgée.

**Étapes :**

1. Décrire : le sélecteur (des enregistrements précis, une source sur une période, ou un pseudonyme) et le motif.
2. Prévisualiser : le nombre d'enregistrements touchés et les journées impactées.
3. Exécuter en une seule transaction : rédaction des enregistrements et ajout d'une entrée au registre des purges.
4. Recalculer : supprimer les événements normalisés issus de ces enregistrements ; les vues, calculées à la volée, suivent immédiatement.
5. Traiter les copies : sauvegardes et exports éventuels.

**Registre des purges.** Append-only : date, sélecteur, motif, nombre d'enregistrements. Jamais le contenu purgé.

**Sauvegardes.** Une donnée purgée survit dans les sauvegardes jusqu'à leur expiration. Après toute restauration, le registre des purges est réappliqué avant la remise en service.

**Accès.** Uniquement via un outil d'administration local sur le NAS, jamais à distance.

## Sauvegardes

Le NAS n'a aujourd'hui aucune sauvegarde : c'est un prérequis avant d'y stocker un journal censé durer des années, puisque le journal est la seule source de vérité.

**Décision : le POC démarre sans sauvegarde.** Le risque est limité : l'historique des messages reste dans iCloud, celui des écoutes sur Last.fm, et l'export Apple peut être retéléchargé. Seuls les clics café, l'annuaire et la clé de pseudonymisation sont irremplaçables ; la clé au moins doit être copiée dans un gestionnaire de mots de passe dès sa création. Ce qui suit s'applique en sortie de POC.

- **Quoi** : un dump PostgreSQL quotidien (cohérent, contrairement à une copie des fichiers d'une base en marche), la configuration de déploiement et le stockage de fichiers (exports, plus tard photos).
- **Le secret de pseudonymisation** : sauvegardé à part. S'il est perdu, les nouveaux pseudonymes ne correspondent plus aux anciens et l'historique d'une personne se coupe en deux.
- **Où** : hors du NAS (disque externe et/ou stockage cloud). Le RAID ne protège ni d'un vol, ni d'un incendie, ni d'une suppression.
- **Chiffrement** : obligatoire, les sauvegardes contiennent les métadonnées de messages.
- **Rétention** : à fixer ; c'est aussi la durée pendant laquelle une donnée purgée survit.
- **Test de restauration** : au moins une fois, registre des purges réappliqué compris.

## Gestion des erreurs

Le brut est écrit avant toute interprétation, donc une erreur retarde l'affichage d'une donnée sans jamais la perdre.

&#91;embedded content: cycle de vie d'un enregistrement · refus, interprétation, rejeu\]

- **Rejet à l'entrée** : seule l'enveloppe est vérifiée (token, champs obligatoires, type déclaré pour la source). Un refus ne stocke rien ; la source garde la donnée et réessaie. La charge utile n'est jamais validée à l'entrée, pour ne pas perdre une donnée dont le format a changé.
- **Erreur d'interprétation** : l'enregistrement passe « en erreur » avec le message et la version de l'interprète, et les suivants continuent. Pas de nouvel essai automatique, le même interprète échouerait pareil.
- **Panne passagère** (base indisponible) : nouvel essai automatique, avec un délai croissant.

**Statut à part.** Le statut d'interprétation vit dans une table séparée : le journal, lui, reste immuable.

**Tout ou rien.** Un enregistrement produit tous ses événements ou aucun.

**Côté sources.** Un agent n'avance son curseur qu'après l'accusé de réception ; avec le dédoublonnage, chaque donnée compte exactement une fois.

**Interprètes avec mémoire.** Une erreur bloque la suite de cette source seulement, les autres continuent. Aucun cas dans le POC : Last.fm fournit des écoutes horodatées.

**Dans le calendrier.** Une journée qui contient des enregistrements en erreur est signalée comme incomplète.

**Rejeu.** Après correction, on rejoue les enregistrements en erreur avec la nouvelle version. Si la correction change aussi le résultat des autres, on réinterprète toute la source : reconstruction à côté, puis bascule.

**Erreurs réseau et serveur.** Coupure, délai dépassé ou erreur 5xx (base indisponible, disque plein) : contrairement à un rejet, ce n'est pas définitif. La source renvoie le lot entier plus tard, avec un délai croissant ; l'idempotence garantit qu'aucun doublon n'en résulte.

**Lots partiels.** Chaque enregistrement d'un lot reçoit son propre résultat et est dédoublonné indépendamment ; l'écriture se fait en une transaction par lot. Si le serveur s'arrête au milieu d'un lot, la source ne reçoit pas de réponse et renvoie tout : ce qui était déjà écrit revient en `duplicate`.

**Erreurs du collecteur.** Service externe en panne ou limitation de débit (réponse 429 de Last.fm) : le connecteur respecte le délai demandé, réessaie plus tard, et son curseur n'avance pas. Une limitation de débit n'est jamais signalée ; une panne prolongée l'est, par le seuil de silence de la source.

**Rejets visibles côté serveur.** Le serveur compte les rejets par source dans le registre : un rejet mis de côté par l'agent Mac apparaît aussi dans le calendrier, sans dépendre de ce que l'agent remonte.

**Alertes (POC).** Les erreurs sont signalées uniquement dans le calendrier : un indicateur sur les journées incomplètes et un compteur global des enregistrements en erreur. Les notifications iPhone (ntfy pressenti) viendront après le POC, avec une règle déjà posée : jamais de donnée personnelle dans une notification.

## Santé des sources

Une source est jugée muette d'après son dernier contact réussi, pas d'après sa dernière donnée : une journée sans écoute est normale, un connecteur qui n'arrive plus à joindre Last.fm ne l'est pas.

| Source | Signal de vie | Seuil d'alerte |
| --- | --- | --- |
| Compteur café | Aucun (saisie manuelle) | Jamais : une journée sans café est une vraie donnée |
| Connecteur Last.fm | Un battement envoyé seulement après une interrogation réussie de Last.fm | 6 heures |
| Connecteur Garmin | Un battement après une interrogation réussie de Garmin ; aucun si la clé est refusée | 12 heures |
| Extension Firefox (Twitch) | Un battement toutes les 15 min tant que Firefox est ouvert, même sans rien regarder | 3 jours : un week-end sans Firefox est normal |
| Agent Mac | Un battement à chaque passage, même sans nouveau message | 7 jours |

Agent Mac et messages : hors POC depuis le 31 oct. 2026.

**Seuil par source.** Chaque source déclare son seuil dans le registre des sources ; une nouvelle source choisit le sien en arrivant.

**Les battements ne vont pas dans le journal.** Ce sont des informations d'exploitation, pas des faits de ta vie : ils mettent seulement à jour la date de dernier contact dans le registre.

**L'état rapporté.** À côté du battement, chaque connecteur rapporte après chaque passage son état, en vocabulaire fermé (`PUT /v1/sources/me/status`) : il dit ce que j'ai à faire (une clé refusée) sans compter comme un contact. La page d'état des trackers le lit par `GET /v1/sources/status` (décidé le 5 nov. 2026, voir le journal).

**Dans le calendrier.** Une source muette s'affiche comme les erreurs. Les journées qu'elle n'a pas encore couvertes sont marquées « en attente » pour cette source, jusqu'au rattrapage.

**Fraîcheur des données.** Le signal de vie du connecteur prouve qu'il joint Last.fm, pas que le scrobbler de l'iPhone fonctionne : si FastScrobbler tombe, le connecteur continue de répondre et ne remonte simplement plus rien. Une seconde alerte, plus douce, signale l'absence de toute nouvelle écoute depuis 3 jours, à vérifier plutôt qu'erreur, puisqu'une pause musicale est possible.

## Exploitation

Le système évolue sans migration du journal, et aucun secret ni donnée personnelle ne traîne hors des endroits prévus.

**Évolution du système**

- La structure du journal (l'enveloppe) est figée ; seule la charge utile évolue, avec sa version.
- Les tables dérivées ne se migrent jamais : on les jette et on les reconstruit.
- Nouvelle version d'interprète : reconstruction à côté, puis bascule, pour que le calendrier ne soit jamais vide.
- Les référentiels (annuaire, registres) sont les seules tables à migrer avec soin, puisqu'ils ne se recalculent pas.
- Les migrations de schéma sont versionnées dans le dépôt et appliquées au démarrage du backend.
- La version majeure de PostgreSQL est figée. Une montée de version majeure passe par un export et une restauration de la base, testés avant, jamais par une simple mise à jour de l'image.

**Logs**

- Jamais de donnée personnelle : seulement des identifiants d'enregistrements.
- Conservation de 30 jours, sur le NAS uniquement.
- Logs structurés, avec un identifiant par requête qui suit un lot de l'ingestion jusqu'à l'interprétation.

**Secrets**

- Un token par source, révocable individuellement (un Mac perdu se coupe sans toucher au reste).
- Aucun secret dans le code : un fichier de configuration sur le NAS, accessible à moi seul.
- La clé de pseudonymisation vit sur le Mac : le NAS ne voit jamais un vrai numéro ni une vraie adresse.

**Déploiement**

- Un docker-compose sur le NAS, avec des versions d'images explicites (jamais « latest »).
- Déploiement continu : un changement du code des images fusionné dans `main`, CI verte, devient une release (tag `vX.Y.Z`), qui construit les images et les déploie sur le NAS, par Tailscale et une clé SSH à commande forcée ; `compose.yaml` et l'administration restent à la main (voir le journal des décisions).
- Ordre de mise à jour : déployer, puis reconstruire si un interprète a changé.
- Un service `migrate` à usage unique applique les migrations avec le rôle propriétaire de la base, avant le démarrage du backend ; le backend tourne avec un rôle qui ne peut qu'ajouter au journal et ne connaît jamais le mot de passe du propriétaire.
- L'agent Mac est versionné et envoie sa version dans l'enveloppe.

**Hébergement.** Synology DS220+ sous DSM 7.4 : Intel Celeron J4025 (2 cœurs, x86-64) et 18 Go de RAM. La mémoire n'est pas une contrainte ; le processeur est modeste mais largement suffisant pour ces volumes. Déploiement en projet Container Manager, comme Invidious : un Postgres dédié au tracker, un backend (ingestion, lecture, interprètes), le collecteur (même image, seul à sortir sur Internet), la web app, et le serveur MCP dans son propre conteneur puisqu'il est le seul exposé sur Internet.

**Stack technique.**

| Composant | Choix | Pourquoi |
| --- | --- | --- |
| Backend (ingestion, lecture, interprètes, collecteur) | Rust : axum, tokio, sqlx | Types qui vérifient formats et versions, SQL vérifié à la compilation, fiable et léger ; projet d'apprentissage voulu |
| Agent Mac | Swift | Accès natif aux API Apple utiles aux futurs trackers (contacts, calendriers, photos, localisation) et base commune avec une éventuelle app iPhone ; le format de l'enveloppe est décrit dans un schéma JSON partagé, que l'agent et le backend respectent tous les deux |
| Web app | React (Vite, application statique) | Ma techno de toujours ; pas besoin de rendu serveur |
| Serveur MCP | TypeScript, SDK MCP officiel | SDK le plus mûr, notamment pour OAuth ; conteneur séparé qui ne parle qu'en HTTP |
| Base | PostgreSQL dédié | Suffit à tout, y compris la recherche |

Les images sont construites par GitHub Actions, hors du NAS (voir le chemin de déploiement) : compiler du Rust sur le Celeron serait très lent.

**Annuaire.** Les noms affichés sont saisis à la main ; aucun nom ne quitte le Mac automatiquement. Un pseudonyme sans nom s'affiche comme « Contact » suivi d'un court identifiant.

**Surveiller le système lui-même.** Si le backend tombe, rien dans l'app ne peut le signaler. Chaque conteneur expose un contrôle de santé et redémarre automatiquement en cas d'arrêt. La page des services hébergés (fonctionnalité 13) pourra plus tard surveiller le tracker comme les autres apps.

**Un outil d'administration unique.** Une petite commande en ligne, exécutée sur le NAS, regroupe les opérations rares : créer ou révoquer une source et son token, déclarer un type de donnée, rejouer les enregistrements en erreur, reconstruire les événements, lancer une purge.

**Développement séparé de la production.** Le développement se fait sur le Mac avec sa propre base et des données de test. On ne teste jamais contre le journal de production, et aucun token de production ne sert en développement.

## Interface

Je veux une belle interface, dans l'esprit d'Apple : sobre, soignée, fluide, qui donne envie de l'ouvrir chaque jour.

- **Inspiration Apple.** Typographie lisible et hiérarchisée, beaucoup d'espace, couleurs retenues, matières translucides et profondeur avec parcimonie, animations physiques (ressorts) et interruptibles, rien de décoratif sans raison.
- **Mode clair et mode sombre**, qui suivent le réglage du système.
- **Pensée pour l'iPhone autant que pour le Mac.** La web app s'ouvre souvent depuis le téléphone (via Tailscale) : elle doit s'y sentir comme une app installée, pas comme un site.
- **Les données d'abord.** Le calendrier et les pages du jour restent lisibles en un coup d'œil ; le soin visuel sert la lecture, jamais l'inverse.

**Skills Claude Code à utiliser** pour tout travail sur la web app :

| Skill | Rôle |
| --- | --- |
| `apple-design` | La référence : l'approche d'Apple (typographie, mouvement, matières, retours visuels) transposée au web |
| `emil-design-eng` | Le polissage : les petits détails qui font qu'une interface « sent » bon |
| `mobile-native` | Le comportement sur iPhone : zone de l'encoche, zoom des champs, retours au toucher, installation en PWA |
| `animate`, `review-animations` | Créer puis relire les animations |

Le skill `design-taste-frontend` ne s'applique pas ici : il vise les pages vitrines, pas les écrans d'app.

## Questions ouvertes

- [ ] Sauvegardes du NAS et export complet : à étudier juste après le POC.
- [ ] Annuaire : assistant de nommage côté Mac, après le POC.
- [ ] Ouvrir la web app sur Internet avec une authentification : domaine, HTTPS, limitation des tentatives, second facteur ou passkey, après le POC.

## Première session Claude Code

Dans l'ordre, en arrivant dans une nouvelle session Claude Code :

- [x] **1. Concevoir le schéma de la base**, directement dans la session : journal brut, statuts d'interprétation, événements normalisés, registre des sources (avec les types déclarés et les tokens hachés), registre des métriques, annuaire, registre des purges. Il découle du contrat de l'API d'ingestion et des règles d'event sourcing. Fait le 17 oct. 2026 : voir [la base de données](database.md), les migrations dans `backend/migrations/` et les tests dans `db/test/`.
- [x] **2. Découper le POC en étapes testables.** Fait le 31 oct. 2026 : voir [le POC](poc.md).
- [x] **3. Écrire le fichier `CLAUDE.md` limité au POC.** Fait le 31 oct. 2026.
- [x] **4. Esquisser la page du jour.** Fait le 31 oct. 2026 : voir `docs/maquettes/page-du-jour.dc.html`, base provisoire reprise à l'étape 10 du [POC](poc.md).
- [x] Reste à trancher ensuite : l'authentification du serveur MCP (avant l'étape 9). Tranché le 2 nov. 2026 : voir le journal des décisions.

Deux points relevés après coup, à trancher avant ou pendant la première session :

- [x] **Authentification de la web app et de l'API de lecture : décidé.** D'autres personnes accèdent au NAS via Tailscale, donc la web app a sa propre connexion : un seul compte, mot de passe haché, session par cookie sécurisé. L'API de lecture n'accepte que cette session ou le jeton de service du serveur MCP. L'API d'ingestion reste protégée par ses tokens de source.
- [ ] **Chemin de déploiement.** Décidé dans les grandes lignes : dépôt sur GitHub, images construites par GitHub Actions sur ses propres machines (rapide, nativement x86-64) et publiées dans le registre privé de GitHub. Un runner auto-hébergé sur le NAS est envisagé, mais seulement pour l'étape de déploiement (récupérer les nouvelles images et redémarrer les conteneurs), pas pour compiler ; il impose un dépôt privé. Reste à prévoir : l'environnement de développement local sur le Mac.

## Idées de trackers (plus tard)

Notées telles quelles, faisabilité à étudier plus tard.

- [ ] Litres d'eau bus par jour
- [ ] Calories mangées et ce que j'ai mangé
- [ ] Poids
- [ ] Une photo de moi par jour
- [ ] Messages Signal, WhatsApp, Instagram, Discord (temps en vocal, avec qui, DM et messages des serveurs)
- [ ] Appels téléphoniques (combien, avec qui, combien de temps)
- [ ] Consommation électrique du logement (compteur Linky)
- [ ] Nombre de tokens Claude utilisés
- [ ] API GitHub
- [x] API Garmin Connect (entraînements et sommeil) : fait le 5 nov. 2026, par l'API personnelle, voir [les chantiers](chantiers.md)
- [ ] Temps d'écran sur l'iPhone
- [ ] Boîte mail (API Gmail ?)
- [ ] Déplacements via géolocalisation
- [ ] Recettes et dépenses sur différents comptes, pro et perso (probablement manuel)
- [ ] YouTube : ce que je regarde
- [x] Twitch : ce que je regarde : en cours depuis le 6 nov. 2026, par une extension Firefox, voir [les chantiers](chantiers.md)
- [ ] Netflix, Prime, Disney+, Canal+ : ce que je regarde
- [ ] Les livres que je lis
- [ ] Les événements auxquels je vais (concerts, spectacles, etc.)
- [ ] Les trains que je prends
- [ ] Mes collections (livres, vinyles, Pokémon, etc.)
- [ ] Combien de fois je suis allé aux toilettes
- [ ] Jeux vidéo (API Steam)
- [ ] Historique Firefox du Mac et de l'iPhone
- [ ] Humeur et énergie du jour
- [ ] Météo du jour, là où je suis
- [ ] Podcasts écoutés

Incertains :

- [ ] Lieux découverts (la première fois que je vais quelque part) : je ne bouge peut-être pas assez pour l'instant
- [ ] Personnes vues dans la journée

## Fonctionnalités futures (hors POC)

Fonctionnalités voulues à terme, notées pour plus tard.

**1. Notes et commentaires.** Noter et commenter ce que je consomme : vidéos YouTube, titres, albums, artistes, livres lus, films, séries, etc.

**2. Recommandations personnelles.** Me recommander chaque jour des choses, pas forcément avec un algorithme (à voir).

Mini-features rattachées aux recommandations :

- **Liens quotidiens** vers des sources externes : le journal de 8h de France Inter, Hacker News, Next (ex-NextInpact), etc.
- **Résumé du jour** de ces sources, à étudier : un article à lire, ou une version audio d'une dizaine de minutes.
- **Sorties des artistes** que j'écoute le plus.

**3. Notes et listes en vrac.** Une sorte de base de plein de choses un peu à l'arrache, consultable et modifiable dans l'app, consultable depuis le MCP. Exemples : idées de cadeaux pour mes proches, TODO du lendemain (avec peut-être des rappels), top 5 de mes chips préférées, liste de mes collections Pokémon ou mangas, liste de mes abonnements, liste de choses que je veux m'acheter.\
Voir pour integrer un editeur md dans l'app ou alors passer par une app tiers. Mais ces listes doivent etre au format markdown je pense.

**4. « Il y a un an aujourd'hui ».** Ce que je faisais, écoutais, où j'étais à la même date les années précédentes, avec la photo de moi prise ce jour-là (voir l'idée de tracker « une photo de moi par jour »).

**5. Rétrospectives** hebdomadaires, mensuelles et annuelles, tous domaines confondus.

**6. Une phrase par jour.** Un mini-journal : une ligne sur ma journée.

**7. Page de saisie manuelle** dans l'app, qui regroupe tous les compteurs et saisies manuels (café, eau, toilettes, humeur…).

**8. Carte de ma vie.** Mes déplacements, lieux et événements sur une carte, filtrable par période.

**9. Les premières fois de l'année.** Toutes les découvertes de l'année, tous domaines confondus.

**10. Capsule temporelle.** Un message à mon futur moi, qui s'affiche dans le calendrier dans un an, ou cinq.

**11. Export complet** de toutes mes données dans un format ouvert. Prioritaire : à étudier juste après le POC, avec les sauvegardes du NAS.

**12. Bien plus tard : un Claude intégré à l'app**, qui interroge le MCP directement depuis l'app, avec un modèle auto-hébergé quand ils seront assez bons (horizon estimé : un an). La recherche globale dans ma vie (pas sûr) rejoint cette idée.

**13. Page de mes services hébergés.** Pour centraliser ma vie dans l'app : la liste de ce que j'héberge sur le NAS, l'état de chaque app (en ligne ou non) et un lien cliquable vers chacune.

**14. Carte mentale après une vidéo.** Après avoir regardé une vidéo, une carte mentale dessinée de ce que j'ai vu. Source : soit ce que j'en ai compris, dicté ou écrit ; soit la vidéo elle-même (faisabilité incertaine).

**15. Brancher mes calendriers personnels** sur le calendrier de l'app, surtout mes calendriers Apple, pour voir mes événements à côté de ce que j'ai fait.

**16. Saisie vocale des trackers manuels.** « Dis Siri, j'ai bu un café », via un raccourci qui appelle l'API d'ingestion.

**17. Jalons.** « Ton 1 000e café », « 100e artiste découvert », « un an de tracking ». Les jalons sont à définir tracker par tracker.

**18. Partage choisi.** Rendre visible à quelqu'un une page précise sans exposer le reste : une rétrospective, ou un avis sur un livre, un film, une série, une musique (voir la fonctionnalité 1).

**19. Étiquettes de périodes (pas sûr).** Marquer une période « vacances », « déplacement », « rush pro » pour comparer mes habitudes selon le contexte. Difficile à déterminer.

**20. Importer mes historiques passés.** Les exports de données des services (Google Takeout, Netflix, Garmin, Steam…), comme l'export Apple pour la musique, pour qu'un nouveau tracker démarre avec des années d'historique. Un import est une source ponctuelle de plus.

**21. L'année en pixels.** Une grille d'un carré par jour, coloré selon une métrique (humeur, écoute, café…), comme la grille de contributions de GitHub.

**22. Rappels de saisie.** « Tu n'as pas encore noté ton humeur aujourd'hui », pour éviter les trous dans les trackers manuels.

**À regrouper : la famille « rétrospective ».** Plusieurs fonctionnalités racontent le passé et devront être pensées ensemble : « Il y a un an aujourd'hui » (4), rétrospectives (5), premières fois de l'année (9), année en pixels (21), jalons (17), partage d'une rétrospective (18), et le petit design par période des idées en vrac.

**Pistes d'architecture, à creuser le moment venu :**

- Une note ou un commentaire est une déclaration datée, comme un marquage « déjà connu » : il trouverait naturellement sa place dans le journal, avec l'historique de mes avis.
- Les listes en vrac sont l'inverse : des données qu'on modifie sans cesse, qui ne sont pas des faits datés. Elles vivraient plutôt à côté du journal, comme les référentiels.
- Le MCP reste en lecture seule : Claude pourrait consulter les listes, pas les modifier.
- Les rappels demanderaient le canal de notification iPhone reporté après le POC.

## Idées en vrac

- Générer un petit design par jour, semaine, mois ou année qui résumerait la période.

## Journal des décisions

Les décisions prises, de la plus récente à la plus ancienne.

| Date | Décision |
| --- | --- |
| 6 nov. 2026 | **Twitch : l'extension Firefox** (voir [les chantiers](chantiers.md), Twitch, étape 2 ; écrite avant l'essai sur le vrai DOM, qui reste à faire). **Forme** : `extension/`, TypeScript regroupé par esbuild, Manifest V3, Firefox 142 au moins (le premier à lire `data_collection_permissions`, déclaré `browsingActivity`), identifiant fixe `twitch@life-tracker.clegirar`, permissions `storage` et `alarms` ; l'hôte du NAS est demandé à l'enregistrement des réglages, par un motif sans port (`https://<nas>.ts.net/*`, dans `https://*.ts.net/*`), avec les deux origines de Twitch (déclarées en `host_permissions`, que Firefox laisse refuser à l'installation ; la page d'options dit si l'accès manque). Un bouton dans la barre d'outils, sans popup : il ouvre la page d'options et porte le « ! » d'un 401. **Ce qui compte** : le script de contenu (cadre principal de www.twitch.tv et clips.twitch.tv) observe chaque `<video>` chaque seconde et à ses événements ; la chaîne vient d'abord de l'URL (`/<login>`, `/<login>/clip/…`, `/popout/<login>/…`), du DOM pour une rediffusion ou un clip de clips.twitch.tv, sinon `null` ; tous les sélecteurs sont dans une seule table (`SELECTORS` de `page.ts`). Une vidéo n'est mesurée que si elle a été vue sur une page de lecture (ou dans le mini-lecteur) : l'aperçu de l'accueil ou d'un survol ne compte pas ; elle garde ensuite sa chaîne (mini-lecteur). Le nom affiché n'est retenu que s'il est le login à la casse près (jamais un titre pris pour un nom), la catégorie seulement depuis un lien vers `/directory/`. **Segments** : intervalles continus de l'horloge où `currentTime` avance (une avance de plus de 2,5 fois le temps écoulé est un saut) ; les segments de 60 s se suivent sans trou ; un segment de moins d'une seconde n'est pas envoyé. L'arrière-plan revérifie chaque segment reçu (liste blanche, login, durée) avant de le mettre en forme, avec l'heure locale de son début. **Envoi** : file dans `storage.local`, vidée chaque minute et 5 s après chaque segment, lots de 500 au plus, `X-Client-Version: twitch-extension-<version>` ; `accepted`/`duplicate` sortent, `rejected` est compté et les 200 derniers gardés ; réseau, 5xx ou autre refus : délai doublé de 1 min à 1 h, levé par un battement réussi ; 401 : arrêt jusqu'à de nouveaux réglages. **État** : battement toutes les 15 min, suivi de `PUT /v1/sources/me/status` (`ok` avec `last_collected_at`, le dernier lot reçu, ou `error/ingest_failed` tant que le dernier envoi a échoué) ; entre deux battements, l'état ne part que s'il change. **Pas de release** : `deploy/next-version.sh` ne regarde que `backend web mcp` ; l'extension se signe sur le Mac (`web-ext sign --channel=unlisted`, clés AMO dans l'environnement seulement) et s'installe à la main. |
| 6 nov. 2026 | **La forme `duration` : du temps mesuré, en union** (voir [les chantiers](chantiers.md), Twitch). La forme existait au registre sans utilisateur ; `stream.watch` est la première. **Sens générique** : chaque événement est l'intervalle `[occurred_at, occurred_at + details.played_ms]` ; le temps d'un jour est l'**union** de ces intervalles (`range_agg` sur `tstzrange`), coupée aux minuits de Paris, donc juste les jours de 23 et 25 heures ; une semaine ou un mois est la somme des unions de ses jours ; le temps d'un élément (une chaîne) est l'union de ses seuls intervalles. Deux streams regardés en même temps comptent donc une fois dans le total, et chacun en entier pour sa chaîne. Calculé à chaque lecture (`read::measured`), sans vue matérialisée ni index de plus. **1 h au plus par événement** (`MAX_MEASURED_MS`) : l'interprète refuse au-delà (erreur d'interprétation), et les lectures cherchent les intervalles commencés jusqu'à 1 h avant leur période, ce qui garde l'index `(generation, metric, occurred_at)`. Un intervalle à cheval sur minuit compte pour chaque jour sa part ; il est compté parmi les événements du jour où il commence. **Où** : `/v1/durations` (sans référence `duration_from`, `events_without_duration` vaut 0), la cellule du mois et de l'année (les minutes de l'union, non arrondies, `null` avant la première donnée comme ailleurs), et **`/v1/top` classé par minutes** (champ `minutes` en plus, puis par clé ; `occurrences` compte les événements commencés sur la période) : « ma chaîne du mois » est celle où j'ai passé le plus de temps, pas celle où j'ai le plus de segments, qui suivent la segmentation de l'extension. Le MCP n'a aucun code de plus : `cumulative_duration` et les minutes de `day_summary` suivaient déjà la forme ; seuls les textes des outils le disent. **Carte du jour** : `GET /v1/days/{date}/stream`, sous session, hors de `READ_ROUTES` comme `/video` : `minutes` (l'union), `summed_minutes` (la somme des temps par chaîne et du temps sans chaîne ; la différence est le temps à deux streams), `segments` (ceux qui touchent le jour), `channels` (login, nom, minutes, contenus, heures du premier début et de la dernière fin, qui peuvent tomber la veille ou le lendemain), `unknown_minutes`, `discoveries`. La requête des découvertes du jour de `/video` devient `read::day_discoveries`, paramétrée par la métrique et les niveaux. |
| 6 nov. 2026 | **Twitch par une extension Firefox** (à ma demande ; voir [les chantiers](chantiers.md)). L'API Twitch (Helix) ne donne pas l'historique de visionnage d'un spectateur, et je regarde uniquement dans Firefox sur le Mac : une extension personnelle mesure la lecture et l'envoie à l'API d'ingestion. **Une source « appareil »**, comme l'agent Mac prévu : sa source `twitch` (token, seuil de silence de 3 jours, libellé « Twitch »), le seul type `twitch.watch`, un battement toutes les 15 min et son état par `PUT /v1/sources/me/status`. **Le token** est collé dans la page d'options et vit dans `storage.local` de l'extension, sur le Mac seulement : il ne vaut que pour écrire des segments Twitch, et se révoque par `admin revoke-source`. **Pas de CORS** : le backend n'en donne toujours aucun ; l'extension écrit depuis son script d'arrière-plan, avec la permission d'hôte de l'origine du NAS demandée à l'usage (à confirmer par l'essai de l'étape 0, dont les constats iront aux chantiers). **Segments** : un enregistrement par segment de lecture, clé UUID créée par l'extension (renvoyer donne `duplicate`), `occurred_at` le début avec le décalage local. Un segment se ferme à la pause, au chargement, à la fin, au saut, au changement de chaîne ou de contenu, à la disparition de la vidéo, quand la page est masquée ou fermée, et **au plus tard au bout de 60 s** (une coupure perd une minute au plus) ; le temps compté est celui de l'horloge, quand `currentTime` avance ; **un trou de plus de 15 s** entre deux observations (veille, onglet gelé) n'est jamais compté ; pas de découpe à minuit, c'est la lecture qui coupe. Charge utile en liste blanche `{channel, channel_name, content, duration_ms, category}` : `channel` est le login en minuscules ou `null` (inconnu : le temps compte, sans élément), `content` vaut `live`, `vod` ou `clip`, `duration_ms` de 1 à 3 600 000 ; ni URL, ni titre, ni chat. L'interprète contrôle chaque champ (messages d'erreur fixes, sans la charge utile), met la durée dans `details.played_ms` comme Apple et produit l'élément `channel`. **La clé d'une chaîne est son login** : stable tant que la chaîne ne change pas de nom ; un changement de login coupe son historique en deux (deux éléments), accepté (rare, et l'id numérique demanderait l'API Twitch). **Les publicités comptent** : c'est du temps passé devant le stream, et les distinguer dépendrait du DOM de Twitch. La catégorie reste dans les détails ; elle pourra devenir un niveau d'élément plus tard, par rejeu. Le téléphone et la TV sont hors champ ; l'historique passé viendra de l'export RGPD de Twitch, chantier à part. |
| 6 nov. 2026 | **Web : un seul choix de métriques pour le mois, l'année et le jour ; sélection vide permise** (à ma demande). La page du jour reprend `MetricFilters` sous son en-tête, avec le même choix (`calendar.metrics`) : chaque carte suit sa métrique (Café, Musique, Vidéos), la carte Activité masque ses blocs (pas, calories, sommeil, séances) et disparaît sans aucun ; les alertes du jour, « Journée incomplète » et l'échec d'un café restent, l'alerte musique suit la musique. Au bout des pastilles, un bouton texte « Tout décocher » (tout coché) ou « Tout cocher ». La dernière métrique se décoche : `[]` gardé veut dire aucune (`null`, jamais choisi, toujours toutes) ; la vue le dit par « Choisis au moins une métrique. », au lieu de la légende dans le mois et l'année (ni chiffre, ni point, ni numéro grisé). La page État perd sa ligne de résumé : les pastilles des cartes et le point du menu suffisent. |
| 6 nov. 2026 | **Web : « Déjà connu » révélé au survol / au toucher** (à ma demande). Sur Mac (`hover: hover` et `pointer: fine`), le bouton est en opacité 0 et apparaît au survol de sa ligne ou au focus clavier (`:focus-within`), fondu de 150 ms, sans fondu en mouvement réduit. Sur iPhone (`hover: none` ou `pointer: coarse`), un toucher sur la ligne la révèle, une seule à la fois, et toucher ailleurs la referme ; tant qu'elle ne l'est pas, le bouton laisse passer les touchers (`pointer-events: none`) : pas de marquage par accident. Toujours visibles : « Marqué connu », un envoi en cours, une ligne en échec. Le bouton garde sa place, sa cible de 44 px et son libellé accessible. |
| 5 nov. 2026 | **YouTube : vidéos classiques seulement** (à ma demande, après la mise en service ; voir [les chantiers](chantiers.md)). Ni Short, ni vidéo supprimée ou privée (sa nature ne se sait plus) : rien n'en entre au journal. ≈85 % des 24 555 vues du premier export étaient des Shorts. **La règle**, par l'API officielle seule : `videos.list` avec `part=contentDetails,snippet,player` et `maxWidth=1000` rend `player.embedWidth`/`embedHeight` au rapport de la vidéo, au même quota (1 unité par lot de 50). Un Short dure de 1 à 180 s (3 min depuis oct. 2024, 60 s avant) **et** a un lecteur au moins aussi haut que large, carré compris (`youtube::is_short`) ; sans taille de lecteur ni durée, une vidéo est classique ; un direct (`P0D`) n'est jamais un Short. Sur 200 vraies vidéos récentes : 169 verticales de 3 min au plus, 28 horizontales de plus de 3 min, 1 horizontale courte, 2 verticales longues. Pas de sonde de `youtube.com/shorts/<id>` : non officielle, sans clé, et fragile. Une vidéo absente d'une réponse **réussie** est indisponible ; un appel en échec (HTTP, quota, réponse sans `items` ni `pageInfo`) ne classe rien et se refait au passage suivant. **Connecteur** : `youtube-state.json` garde le classement (`classic`, `shorts`, `unavailable` avec le jour où la redemander, 30 jours plus tard et seulement si une nouvelle vue la cite) et les vues en attente (`pending`, sans doublon, au plus celles d'un export), qui survivent à un redémarrage. 100 appels au plus par passage (5 000 vidéos) : l'historique en cinq passages. Seules les vues classiques sont envoyées, et la fiche `youtube.video_info` des seules vidéos classiques, au moment du classement. Un export ne compte comme reçu (`last_exported_at`) qu'une fois toutes ses vues classées ; la dernière vue avance aussi pour une vue écartée. **Correction du journal**, comme pour les réactions et en la généralisant : `corrections_version` (2) remplace `reactions_cancelled` ; en dessous, l'export est complet (la tâche le note, `full`), et ses vues de Shorts ou de vidéos indisponibles sont annulées sous leur clé d'alors, `<id>:<instant en ms>`, avec la fiche d'un Short (clé `<id>`) ; la version passe à 2 quand plus rien n'attend. Sans le droit `record.cancel`, la correction attend l'export complet suivant. Une règle de plus : monter la version. **Logs** : des nombres seulement, `skipped_shorts`, `skipped_unavailable`, `classified`, `awaiting_classification`, `cancelled`. La carte « Vidéo indisponible » de la web app reste, pour ce qui serait déjà entré. |
| 5 nov. 2026 | **YouTube : les réactions ne sont pas des vues** (correctif, après la mise en service). L'export de production contient, en plus des vues (« Watched », ≈30 000 avec les publicités), des recherches, des posts, des abonnements et ≈90 réactions (« Liked … », « Disliked https://www.youtube.com/watch?v=… ») dont le `titleUrl` est un `/watch?v=` sans `details` : la règle « classement sans le texte » en faisait des vues, titre compris. **Aucun champ neutre** ne sépare une vue d'une réaction (`activityControls` est traduit lui aussi) : une vue exige donc, en plus du lien, un titre qui commence par un préfixe de vue connu (`WATCHED_PREFIXES` : « Watched », « Vous avez regardé ») ; c'est la seule règle qui lit le texte. Les autres entrées `/watch` sans publicité sont écartées en `reaction` (`skipped_reactions` dans les logs) ; un titre sans préfixe connu n'est jamais affiché. **Filet** : un export qui a des entrées `/watch` mais aucune vue reconnue (une langue de compte inconnue) n'est pas lu : rien n'entre ni n'est annulé, la dernière vue et le dernier export n'avancent pas, un `warn` donne les nombres et le passage rapporte `error`/`service_unavailable` ; l'export suivant vient 24 h plus tard. Ajouter la langue à `WATCHED_PREFIXES`. Limite acceptée : un export partiel qui n'aurait que des réactions déclenche aussi le filet. **Correction du journal** : les fausses vues sont annulées par la source elle-même (`record.cancel`, clé `cancel:<clé>`, `{source, key}`, comme Garmin), sous la clé que la version d'avant leur donnait, `<id de la vidéo>:<instant en ms>` ; annuler une clé qui n'a jamais existé ne fait rien, renvoyer ne fait qu'un `duplicate`. Une fois seulement : tant que l'état du connecteur n'a pas `reactions_cancelled`, l'export est demandé sans `startTime` (tout l'historique, où sont les réactions anciennes) ; ensuite, les réactions sont seulement écartées. Les annulations partent dans leurs propres lots, après les vues : sans le droit `record.cancel`, elles sont mises de côté (`kind_not_declared` dans les logs, `error`/`ingest_failed` pour le passage), les vues entrent quand même, et l'export suivant les renvoie. `YOUTUBE_SOURCE` (`youtube` par défaut) nomme la source que désignent ces annulations. |
| 5 nov. 2026 | **YouTube : les miniatures des vidéos, côté web.** Chaque vue de la carte du jour commence par sa miniature `https://i.ytimg.com/vi/<id>/mqdefault.jpg` (320 × 180), seulement si la vue a un titre (une vidéo supprimée ou privée garde sa ligne « Vidéo indisponible », sans image) et si l'identifiant suit `^[A-Za-z0-9_-]{11}$` (sinon, pas d'image). Image décorative (`alt=""`, le titre fait le lien), `loading="lazy"`, `decoding="async"`, `referrerPolicy="no-referrer"`, dans un cadre 16/9 posé avant elle (96 px de large sur iPhone, 128 px à partir de 600 px, coins de 8 px, fond neutre pendant le chargement) ; en cas d'erreur, l'image disparaît et le cadre vide reste, la liste garde son alignement. Titre sur deux lignes au plus. La CSP de `web` ajoute `https://i.ytimg.com` à `img-src`, à côté de `tile.openstreetmap.org` (précédent : la carte d'une séance, 4 nov. 2026), et rien d'autre. **Ce que Google apprend** : le navigateur lui demande les miniatures des vidéos du jour affiché, sans Referer ; Google voit l'IP du téléphone et quelles vidéos je relis dans l'app. Accepté : ce sont des vidéos que Google m'a déjà vu regarder. Playwright sert une image locale à la place de `i.ytimg.com`. |
| 5 nov. 2026 | **YouTube, côté web app** (voir [les chantiers](chantiers.md)). **Carte du jour** sur `GET /v1/days/{date}/video` : nombre de vidéos, « Durée des vidéos » avec « Pas le temps regardé. », les vidéos sans durée comptées à part (« Sans compter 1 vidéo de durée inconnue. », ou « Durée inconnue pour… » sans aucune durée), chaîne du jour à partir de deux vidéos, nouvelles chaînes (les découvertes de niveau `channel` ; une vidéo vue avant ce jour porte « déjà vue » plutôt que de lister les vidéos découvertes, presque toutes), liste des vues. Chaque vue est un simple lien vers `youtube.com/watch?v=<id>` (nouvel onglet, `noopener noreferrer`), sans intégration ; sa miniature fait l'objet de l'entrée « miniatures » du même jour. Une vidéo sans titre (supprimée ou privée) dit « Vidéo indisponible », sans lien ni chaîne. Sans vue, la carte distingue « pas de données » (avant l'historique, ou après la dernière vue reçue, puisque l'export est quotidien) de « aucune vidéo » (dans l'historique). **Page d'état** : le bouton suit `authorization_url`, quelle que soit la source ; une demande en cours suffit à rendre la carte « À faire ». « Donner l'accord » pour `consent_required`, « Renouveler l'accord » sinon, en lien dans l'onglet même (Google revient sur l'app) ; sans URL encore, la carte dit qu'elle arrive à la prochaine passe. L'échéance de YouTube est un jeton de 7 jours : ligne « Accès Google » à l'heure près, en couleur deux jours avant, sans rendre la carte « À faire » (prévenir 14 jours avant, comme pour une clé, la laisserait toujours à faire ; le connecteur redemande l'accord la veille). `next_run_at` s'y dit « Prochain export ». **`/autorisation`** : lit `state` et `code` ou `error`, envoie exactement `{state, code}` ou `{state, error}` une seule fois, puis remplace l'adresse par `/autorisation` et garde l'issue dans l'entrée d'historique (recharger la redit sans rien renvoyer) ; le code n'est ni affiché ni journalisé. Sans session, l'écran de connexion garde l'adresse et sa requête, et la page transmet ensuite. 404 ou adresse sans réponse : « Ce lien a expiré. » ; `access_denied` : « Tu as refusé l'accord. » ; panne du serveur : « Réessayer », adresse intacte. |
| 5 nov. 2026 | **Dernière collecte réussie dans l'état d'une source** (voir [les chantiers](chantiers.md), YouTube). Le connecteur YouTube passe toutes les 5 min et Google répond à chaque fois, alors que l'export n'a lieu qu'une fois par 24 h : « Dernière passe » restait à l'instant, et des exports en échec répétés passaient inaperçus. Le vocabulaire de `PUT /v1/sources/me/status` gagne `last_collected_at` (RFC 3339, facultatif, rangé en UTC) : la dernière fois que le connecteur a reçu des données de son service et les a remises à l'API d'ingestion. Un état rapporté avant ce champ ne l'a pas ; la web app le lit comme absent. **YouTube** : `youtube-state.json` garde `last_exported_at` (l'heure où un export `COMPLETE` a été téléchargé et entièrement envoyé) et `exports_since` (le premier export demandé, accepté ou refusé jusqu'à une heure donnée) ; chaque état rapporte `last_collected_at` = `last_exported_at`. Un passage réussi, accord valable, sans export reçu depuis 48 h (ou, sans aucun export, 48 h après le premier demandé) rapporte `error`/`service_unavailable` au lieu de `ok`, avec `expires_at` et `next_run_at` ; le signal de vie continue, puisque Google répond. 48 h : deux exports permis manqués, pas un simple retard. Last.fm et Garmin ne le rapportent pas encore (ils ne gardent pas l'heure d'une collecte non vide). **Page d'état** : « Dernier export » pour `youtube` (« aucun pour l'instant » sans date), « Dernière collecte » pour une autre source qui le rapporte, au format des autres heures passées ; en panne `service_unavailable` sans export depuis 48 h, la marche à suivre dit que Google répond mais qu'aucun export n'a abouti depuis 2 jours, que le collecteur réessaie seul, et de lire ses logs si cela dure. |
| 5 nov. 2026 | **YouTube, côté backend** (voir [les chantiers](chantiers.md)). **Types et métriques** (migration `20261105130000_youtube.sql`) : `youtube.watch`, clé `<id de la vidéo>:<instant en ms>` (aucune paire ne se répète dans l'historique), charge utile en liste blanche `{video_id, title, channel_id, channel_name}`, à l'instant de la vue ; `youtube.video_info`, clé `<id>`, `{video_id, duration_s, title, channel_id, channel_name}` par l'API Data v3, un enrichissement comme la fiche d'un titre de Last.fm. Métrique `video.watch` (« Vidéos YouTube », événement riche compté, unité « vidéo », niveaux `{channel, video}` sous les identifiants stables de YouTube et leur nom affiché, découverte, fraîcheur de 3 jours, ordre 70, détails partagés `title` et `channel_name`) et sa référence `video.duration` (secondes, niveau `video`), désignée par `duration_from` : les minutes génériques et celles du jour la lisent sans code propre. **La durée est celle de la vidéo, pas le temps regardé** : Google ne le donne pas, l'interface le dit ; une vidéo sans durée connue est comptée à part, jamais à zéro. Interprètes sans état : une vue donne un événement, avec l'élément chaîne s'il est connu ; une fiche avec une durée positive donne la référence. Le titre affiché est celui de la vue ; celui de la fiche ne sert que s'il manque. **Lecture** : `GET /v1/days/{date}/video`, sous session, hors de `READ_ROUTES` (le MCP lit les mêmes faits par les lectures génériques) : chaque vue (heure, id, titre, chaîne, durée), le total des durées connues, le nombre sans durée, et les chaînes et vidéos découvertes ce jour-là, comme la musique. **Recherche par le nom affiché** : les clés de YouTube sont des identifiants, `/v1/search` lit donc aussi `lower(display)`, par un index trigramme générique (migration `20261105130100_item_display_search.sql`). **Classement sans le texte** (corrigé le même jour : une vue exige aussi un préfixe de vue connu, voir la ligne « les réactions ne sont pas des vues ») : une vue est un `titleUrl` en `/watch` avec `v=` (11 caractères, seul paramètre) et sans `details` ; une publicité a des `details` ; le reste (recherches, posts, abonnements, liens google.com, YouTube Music) est écarté et compté par motif ; les libellés (« Watched », « Vous avez regardé ») ne servent qu'à retirer leur préfixe du titre, et un titre qui n'est qu'un lien (vidéo supprimée ou privée) devient `null`. Lecture en flux du `MyActivity.json`, archive téléchargée sur le volume d'état puis effacée (200 Mo au plus) : le collecteur passe de 64 à 128 Mo de mémoire. **Connecteur** : `YOUTUBE_CLIENT_ID`, `YOUTUBE_CLIENT_SECRET`, `YOUTUBE_REDIRECT_URI`, `YOUTUBE_API_KEY` et `YOUTUBE_TOKEN` ensemble ; un passage toutes les 5 min, un export au plus toutes les 24 h, `startTime` à la dernière vue moins 48 h (son effet reste à voir ; les clés évitent les doublons) ; un refus `RESOURCE_EXHAUSTED_TIME_BASED` donne l'heure du prochain (`timestamp_after_24hrs`) et n'est pas une panne ; tout l'export repart à chaque fois, en `duplicate` pour ce qui est connu. Fiches par lots de 50, 20 appels au plus par passage, chaque vidéo une fois ; une vidéo absente de Data v3 est redemandée au bout de 30 jours. État `youtube-state.json` en 0600 : jeton de rafraîchissement et son échéance, demande d'accord en cours (état et vérificateur PKCE), dernière vue, tâche d'export, prochain export permis, vidéos enrichies ; le jeton d'accès reste en mémoire. Signal de vie après un appel réussi à Google seulement. **L'accord par la web app** : le connecteur prépare la demande (état et vérificateur de 32 octets aléatoires, PKCE S256, `access_type=offline`, `prompt=consent`) et confie l'URL par `PUT /v1/sources/me/authorization` (token de la source, sans toucher au dernier contact, refusée si elle n'est pas en https sur `accounts.google.com` avec ce même état), renvoyée à chaque passage ; le backend la garde **en mémoire seulement** (une heure, une par source, jamais dans la base, le journal ni les logs) et l'ajoute à `GET /v1/sources/status` (`authorization_url`). Le cookie étant `SameSite=Strict`, Google revient sur la page `/autorisation` de la web app, qui transmet `{state, code}` ou `{state, error}` par `POST /v1/authorizations/callback`, sous session et avec l'en-tête anti-CSRF : état comparé en temps constant, code gardé 10 minutes, demande close ; état inconnu ou expiré, 404. Le connecteur le prend une fois par `GET /v1/sources/me/authorization/code` (un passage par minute tant qu'il attend) et l'échange lui-même ; un refus ou un code refusé : nouvelle demande. Sans jeton, `action_required` avec `consent_required` (nouveau code) la première fois, `consent_expired` ensuite ; `invalid_grant` au rafraîchissement : `consent_expired`. **Renouvellement anticipé** : un jour avant l'échéance du jeton (7 jours en mode Testing), et jamais pendant un export, le connecteur retire l'accord en cours par `authorization:reset` (Google refuse un nouvel accord tant que le précédent est actif) puis en demande un : la page d'état propose « Renouveler » avant le trou. Un client OAuth (`invalid_client`) ou une clé Data v3 refusés : `action_required`/`key_rejected`. État rapporté : `expires_at`, l'échéance du jeton ; `next_run_at`, le prochain export permis. |
| 5 nov. 2026 | **Page d'état des trackers, côté backend** (voir [les chantiers](chantiers.md)). **Deux signaux** : le dernier contact (`last_seen_at`) garde son sens, le service a été interrogé avec succès ; l'état passe par `PUT /v1/sources/me/status`, token de la source seulement (ni la session, ni le jeton de service), qui ne touche ni au dernier contact ni à la version du client : un connecteur qui dirait « clé refusée » ne paraît pas vivant, et le seuil de silence alerte toujours. **Vocabulaire fermé**, sans texte libre : `state` (`ok`, `action_required`, `error`), `reason` (`key_rejected`, `consent_expired`, `rate_limited`, `service_unavailable`, `ingest_failed` ; vide pour `ok`, exigée sinon), `expires_at` et `next_run_at` (RFC 3339, facultatifs, rangés en UTC). Tout autre champ, code ou incohérence : 400 `invalid_json`, sans dire lequel ; corps de 1 Ko au plus ; jamais journalisé. Schéma `source_status_request` dans le contrat. **Stockage** : migration `20261105120000_source_status.sql`, `core.sources` gagne `status` (jsonb, un objet), `status_reported_at` et `label` (nom affiché, 80 caractères, sans espace autour) ; `tracker_app` n'a `UPDATE` que sur les deux premières. Un seul état par source, le dernier, hors du journal. Libellé par `admin create-source --label` ou `admin set-label`. **Lecture** : `GET /v1/sources/status`, sous session, hors de `READ_ROUTES` : les sources à token non révoquées qui ont un seuil de silence, un état ou un libellé (ni `webapp`, ni l'import Apple ; le libellé fait paraître le raccourci café, sans seuil puisqu'une journée sans café n'est pas un silence : écart à la conception, qui ne comptait que seuil et état), par libellé, sinon nom, sans casse, avec dernier contact, seuil, `silent` (règle de `/v1/sources/health`), état et sa date, et pour chaque métrique affichée qu'elles alimentent (`applies_to` vide) sa dernière donnée vivante. **Connecteurs** : Last.fm et Garmin rapportent leur état après chaque passage, réussi ou non, avec la prochaine passe prévue ; délai de 5 s ; un échec se note (`status not reported`) et ne change ni le passage, ni ses nouveaux essais, ni le signal de vie. Correspondance : clé refusée (Garmin 401 ou 403 à l'échange ; Last.fm codes 10 et 26, ou un 403) en `action_required`/`key_rejected` ; limitation en `error`/`rate_limited` ; autre refus ou coupure du service en `error`/`service_unavailable` ; ingestion en échec en `error`/`ingest_failed` ; un état qui ne s'écrit plus sur disque, faute d'un code à lui, en `error`/`service_unavailable`. **Échéance de la clé Garmin** : Garmin ne la donne pas ; `GARMIN_KEY_EXPIRES_ON` (`AAAA-MM-JJ`, la date affichée à la création de la clé), lue au démarrage du collecteur, qui refuse de démarrer si elle est illisible. Rapportée dans chaque état en `expires_at` : la fin de ce jour à Paris (minuit du lendemain, heure de Paris, en UTC), car la clé vaut tout le jour affiché. |
| 4 nov. 2026 | **Garmin : la fréquence cardiaque de la nuit** (à ma demande, après la mise en service de la v0.3.1). Le résumé de sommeil n'en donne pas (`avgHeartRate` vide) ; `GET /wellness-service/wellness/dailyHeartRate?date=…` rend la série d'un jour (720 échantillons `[instant en ms, bpm ou null]`, du minuit au minuit de Paris, un jour par appel ; essai sur le vrai compte, structure seulement). **Brut** : `garmin.sleep` gagne `heart_rate`, `[[secondes depuis le début, bpm], …]`, les échantillons non vides des séries de la veille du réveil et du jour du réveil compris entre `start` et `end` (bornes comprises), triés, un par instant, en calcul UTC sans fuseau ; absent sans échantillon. Seule la nuit entre au journal, pas la journée : la clé d'une nuit finie ne bouge plus quand la journée continue. **Format 1 gardé, sans convertisseur** : un champ facultatif ajouté, les nuits d'avant sont déjà des charges utiles valides de ce format (sans échantillon, rien à calculer) ; la règle 12 vaut pour un format qui change de forme, ce qui n'est pas le cas, et une version 2 obligerait à un convertisseur qui n'aurait rien à convertir. L'interprète `garmin.sleep` passe en version 2 : `lowest_hr` (le plus petit bpm) et `avg_hr` (la moyenne arrondie) dans les détails, seulement avec des échantillons ; la série reste au journal, hors des détails. Il donne le même résultat qu'avant sur les nuits d'avant : `admin rebuild` n'est pas nécessaire et donnerait 0 différence. `shared_details` de `sleep` gagne `lowest_hr` et `avg_hr` (migration `20261104130000_sleep_heart_rate.sql`, les deux de Garmin étant déployées) : le MCP voit la plus basse et la moyenne, jamais la série. **Connecteur** : après le sommeil, un appel par jour nécessaire, une seule fois par passage (une série sert à deux nuits), avec la pause d'une seconde ; 9 jours pour la fenêtre relue. **Échec** : une erreur de la série (`heart_rate` dans les logs) fait échouer le passage comme les autres appels, qui reprend avec son délai : envoyer la nuit sans ses échantillons ferait une version de plus, puis une révision au passage suivant, et une nuit relue une seule fois (celle de l'historique) resterait sans fréquence cardiaque. Exception : un 404 est un jour sans série, et ses nuits partent sans ses échantillons, pour qu'une absence durable ne bloque jamais les pas, le sommeil et les séances. **Rattrapage, une fois** : l'état gagne `heart_rate_backfilled` ; absent (état d'avant, ou neuf), le passage relit tout le sommeil des 400 jours (13 appels par tranches de 31 jours) et les séries de ses nuits (environ 150 appels), le reste sur la semaine seulement. L'état ne connaît que les nuits des 14 derniers jours : pour une nuit qu'il ne connaît pas, la version annulée est celle d'avant la fréquence cardiaque, retrouvée par sa clé (la même charge utile sans `heart_rate`, même empreinte) ; une annulation désigne source et clé, et viser une clé absente du journal est sans effet (installation neuve, état perdu). Aucune nuit ne garde deux versions vivantes, testé contre un état et un journal de la version d'avant. **Risque résiduel** : une nuit révisée par Garmin après sa sortie de l'état (au-delà de 14 jours), dont la version vivante ne correspond plus au contenu actuel, garderait deux versions ; rien de tel n'a été vu, et le remède reste l'annulation à la main. Web : « FC la plus basse 48 bpm · moyenne 55 bpm » sous les heures de la nuit, rien sans fréquence cardiaque. |
| 4 nov. 2026 | **Garmin : le MCP voit tout, à travers des zones de confidentialité** (remplace « le MCP voit les lieux, jamais les coordonnées ni les traces » de l'entrée « Garmin : lectures » du même jour, à ma demande). **Détails** : la règle « jamais de détails » des lectures génériques devient une règle par métrique et par champ, dans le registre : la colonne `core.metrics.shared_details` liste les champs que rendent les occurrences (`/v1/occurrences`, donc le MCP) ; vide pour le café et la musique, qui ne changent pas ; pour Garmin, les détails des pas, des calories (actives, au repos), du sommeil (début, fin, phases) et des séances (type, durées, distance, vitesse, cardio, calories, lieu, coordonnées de début et de fin). Chaque occurrence porte aussi un identifiant `<numéro du journal>:<rang>`. **Zones de confidentialité**, comme Strava : `PRIVACY_ZONES` de `serve` (`lat,lon,rayon_m`, plusieurs séparées par `;`, rayon de 50 km au plus), dans le fichier de secrets du backend sur le NAS seulement, jamais dans le dépôt, le journal ni les logs (le démarrage écrit « zone de confidentialité : N zone(s) », ou qu'elle est absente ou illisible). Appliquées à la lecture, aux seules réponses du jeton de service (`read::Exposure`) : toute paire `<x>_lat`/`<x>_lon` des détails dans une zone passe à `null` avec `<x>_hidden: true` ; une trace perd ses points dans une zone et se coupe en morceaux là où elle y entre, et aussi là où un segment entre deux points dehors la traverserait : jamais de ligne à travers ; un morceau d'un seul point disparaît. **Sûr par défaut** : sans zone ou avec une valeur illisible (une seule zone fausse suffit), le jeton ne reçoit aucune coordonnée (`<x>_hidden: true`) ni aucun parcours (403 `privacy_zone_missing`) ; lieux et autres détails restent. La session (web app, derrière Tailscale) voit tout, sans filtre. **Parcours pour le MCP** : `GET /v1/route?id=<occurrence>`, ajouté à `READ_ROUTES`, rend des morceaux de `[lat, lon]` au cent-millième et `trimmed` si des points ont été retirés ; générique, il cherche la référence à points (`details.points`) d'une métrique qui éclaire celle de l'occurrence (`applies_to`), rattachée par `activity_id`, le seul identifiant que partagent une séance et sa trace. Outil MCP `route` (l'identifiant d'une occurrence), proposé par l'inventaire aux métriques dont les détails partagés ont des coordonnées : un outil de plus, parce que « quel parcours » et « où ai-je couru » échouaient sans lui. **Conseil pour la zone** : ne pas la centrer sur le domicile mais 100 à 300 m à côté, rayon de 500 à 800 m ; centrée, les points de coupure de nombreuses séances dessineraient un cercle dont le centre serait la maison. **Exposition résiduelle** : ce qui passe par le MCP (lieux, détails, parcours hors des zones) part chez Anthropic, qui le garde jusqu'à 30 jours après sa suppression d'une conversation ; l'entraînement sur mes données est désactivé. Ordre d'affichage : la colonne `core.metrics.display_order` (cafés, écoutes, pas, séances, sommeil, calories brûlées), suivie par le calendrier, ses filtres et légendes, et l'inventaire du MCP ; les deux colonnes arrivent par une migration à part (`20261104120100_metric_order_and_shared_details.sql`) plutôt que dans celle de Garmin, car `core.metrics` vient du schéma déjà déployé. Calories brûlées en vrai orange (`#dc6a00` en clair, `#ff9f0a` en sombre), loin du rouge des écoutes et du brun du café. |
| 4 nov. 2026 | **Garmin : la carte d'une séance, côté web.** Leaflet 1.9.4 depuis npm, chargé à part (`React.lazy`, 44 ko gzip, et 6 ko pour sa feuille de style) seulement quand une carte approche de l'écran (`IntersectionObserver`, 200 px d'avance), dans un cadre de taille fixe (16/10, 320 px au plus) qui ne pousse rien. Tuiles raster `tile.openstreetmap.org` : la CSP de `web` les autorise en `img-src`, seule ressource extérieure ; `referrerPolicy: strict-origin` sur les tuiles, car la politique d'OpenStreetMap demande un Referer et la page envoie `no-referrer` (l'origine de l'app, jamais son chemin). OpenStreetMap voit l'IP du téléphone et la zone affichée : accepté. Mention « © les contributeurs d'OpenStreetMap » sous la carte, en lien de 44 px de haut. Au toucher, la carte ne capte pas le défilement (glisser et pincer coupés, `pointer: coarse`) ; à la souris, on la déplace. En sombre, les tuiles sont inversées (`invert` + `hue-rotate`), le tracé garde sa couleur ; fondu de 200 ms à l'arrivée des tuiles. Tracé dans la couleur des séances, départ en vert, arrivée en rouge. Carte Activité entre Café et Musique. Couleurs des métriques (systèmes d'Apple, clair et sombre) : pas en vert, calories en orange, sommeil en indigo, séances en bleu. Les pastilles des filtres passent à la ligne : six métriques ne tiennent plus sur un iPhone, et une rangée qui défile cacherait les dernières. `valueText` suit l'unité du registre (« 8 412 pas », « 2 350 kcal », « 7 h 12 », « 1 séance ») ; une case du mois donne le sommeil en heures (« 7,2 h »). |
| 4 nov. 2026 | **Garmin : lectures** (sa règle pour le MCP est remplacée par l'entrée « le MCP voit tout, à travers des zones de confidentialité »). Les lectures génériques suffisent au calendrier et au MCP ; deux routes de session s'ajoutent pour la carte Activité, `GET /v1/days/{date}/activity` (pas, calories, sommeil avec leurs détails, séances avec les leurs et `has_track`) et `GET /v1/activities/{id}/track` (les points), `no-store`, **hors de `READ_ROUTES`** : elles donnent des coordonnées. **Le MCP voit les lieux, jamais les coordonnées ni les traces** (décision confirmée, définitive) : `/v1/occurrences` donne désormais la valeur de chaque événement (km d'une séance, minutes de sommeil), jamais ses détails ; aucune route du jeton de service ne lit les détails d'un événement, et un test essaie chacune sur une séance avec GPS et trace. `day_summary` donne la valeur d'une mesure sans lister d'heure (midi est une convention), et pour une métrique à éléments sans découverte, chaque occurrence avec son heure, ses éléments et sa valeur. Le sommeil reste une mesure : pas de durée cumulée par `read::durations`, Claude additionne les valeurs. |
| 4 nov. 2026 | **Garmin : un collecteur, plusieurs connecteurs.** `collect` lance Last.fm si `LASTFM_API_KEY` est fourni (avec `LASTFM_USER` et `COLLECTOR_TOKEN`) et Garmin si `GARMIN_API_KEY` l'est (avec `GARMIN_TOKEN`), au moins l'un des deux, chacun dans sa tâche, avec son token, son état (`state.json` gardé pour Last.fm, `garmin-state.json`), ses nouveaux essais et son signal de vie : la panne de l'un n'arrête pas l'autre. Garmin : jeton redemandé une heure avant son `exp` ou sur un 401, gardé en mémoire seulement ; clé refusée (401 ou 403 à l'échange) : « clé Garmin refusée : expirée ou révoquée » dans les logs, aucun signal de vie, et le seuil de silence de 12 h alerte. Premier passage : 400 jours (résumés et sommeil par tranches de 31 jours, Garmin refusant 400 jours de résumés d'un coup, séances par pages de 100), puis chaque quart d'heure les 7 derniers jours ; une seconde entre deux appels. Garmin d'abord, puis l'ingestion, puis le signal de vie, puis les traces. L'état oublie les sujets de plus de 14 jours. Le jour de Paris se calcule sans base de fuseaux (règle européenne de l'heure d'été, `collector::garmin::paris_offset`). |
| 4 nov. 2026 | **Garmin : la trace d'une séance.** Le connecteur télécharge le fichier d'origine (`/download-service/files/activity/{id}`, un ZIP qui contient un `.FIT`, 32 Mo au plus) de chaque séance qui a un point de départ, une seule fois, le lit en Rust (`zip` sans compression C, `fitparser`), garde les messages `record` qui ont une position (demi-cercles × 180/2³¹), puis simplifie par Douglas–Peucker à 5 m, tolérance doublée tant qu'il reste plus de 1 000 points : quelques dizaines de ko, loin du Mo d'un lot. Type `garmin.activity_track`, `{activity_id, points: [[lat, lon, secondes depuis le début]]}`, même règle de révision. Le fichier FIT n'est pas gardé : pas de stockage de fichiers. Une séance sans fichier (404) ou sans trace est notée dans l'état et n'est plus redemandée. Interprète : la référence `workout.track`, rattachée à `workout`. |
| 4 nov. 2026 | **Garmin : types, métriques et révisions.** Une source `garmin`, son token, et quatre types en liste blanche, figée par un test : `garmin.daily_summary` (`calendar_date`, `steps`, `distance_m`, `calories_total`, `calories_active`, `calories_resting` ; rien pour un jour sans `includesWellnessData` ou sans pas : une journée sans données n'est jamais une journée sans activité), `garmin.sleep` (`calendar_date`, `start`, `end`, `sleep_seconds` et les quatre phases ; rien pour une nuit sans sommeil mesuré), `garmin.activity` (`activity_id`, `type`, `start`, durées, distance, vitesse, cardio, calories, **`start_lat`, `start_lon`, `end_lat`, `end_lon` et `location_name`** : le GPS des séances est voulu ; jamais le nom de la séance, le propriétaire ni les identifiants du profil), `garmin.activity_track`. Un total du jour appartient à son jour calendaire : `occurred_at` est midi heure de Paris, avec son décalage ; une nuit est rangée au jour du réveil, celui de Garmin ; une séance à son début, avec le décalage de son heure locale. Nombres normalisés (entiers, sinon au millième ; coordonnées au millionième). **Révisions (règle 9 : des faits, pas des états)** : clé `<sujet>:<16 premiers hexadécimaux du SHA-256 de la charge utile>` ; un contenu identique est un `duplicate` ; un contenu changé part en nouvel enregistrement suivi de l'annulation de la version précédente (clé gardée dans l'état du connecteur), dans le même lot, donc la même transaction : toute lecture ne voit qu'une version vivante, sans code de lecture propre. Un contenu qui revient à une version déjà annulée prend une clé voisine (`:2`, `:3`…). La source a le droit `record.cancel` ; l'annulation d'une source par elle-même passe par le même moteur. **Risque résiduel** : si l'état du connecteur est perdu et qu'une révision survient avant le passage suivant, la version d'avant reste vivante (l'état ne la connaît plus). Le calendrier et la carte prennent alors la plus récente (`last`, même date, plus grand numéro du journal), mais les occurrences en montrent deux et une séance compterait double. Remède : le volume de l'état ; sinon une annulation à la main par l'API d'ingestion avec le token de la source. Métriques : `steps`, `calories.burned`, `sleep` (mesures en `last` ; le sommeil en minutes, mesure et non durée : il a des versions, et une durée exigerait une référence), `workout` (événement riche compté, éléments `activity_type` et `place`, clé du lieu en minuscules NFC, valeur en km), référence `workout.track` ; fraîcheur de 3 jours pour les pas et le sommeil. |
| 4 nov. 2026 | **POC validé, Garmin Connect en premier tracker après le POC, par l'API personnelle de Garmin.** L'étape 10 sort du POC : la liste des idées et des fonctionnalités futures tient lieu de liste de fonctionnalités ; les reprises d'interface vont dans `TODO.md`. Pour Garmin (pas, sommeil, courses, calories), l'API personnelle ouverte le 21 oct. 2026 ([présentation](https://developer.garmin.com/gc-developer-program/personal/personal-api), [FAQ](https://developer.garmin.com/gc-developer-program/personal/personal-faq/)) : clé créée par moi, d'un an au plus, échangée contre un jeton bearer de 24 h sans jeton de rafraîchissement ; lecture seule et en tirage, donc ni callback ni exposition ; usage personnel explicitement permis. La clé est un secret de production, saisie sur le NAS seulement, à renouveler chaque année (Garmin prévient par courriel une semaine avant). Écartés : l'accès non officiel (python-garminconnect, garth), qui demande le mot de passe, a cassé au changement d'authentification de mars 2026 et se heurte aux blocages de Cloudflare, contre les conditions de Garmin ; Apple Health (Health Auto Export), qui demande une app payante, perd le score de sommeil et ne synchronise pas téléphone verrouillé ; les offres Starter et Enterprise, réservées aux entreprises. Courses en résumé par séance, sans fichier FIT. Essai sur le vrai compte le même jour (structure seulement) : l'API sert tout l'historique (depuis juin 2026), donc pas d'import de l'export ; pas et calories (active, repos, totale) en un appel par `dailysummaries` ; sommeil sans score (ma montre n'en calcule pas), nuits manquantes quand la montre n'est pas portée ; liste des activités avec GPS et nom du propriétaire, écartés par la liste blanche ; aucun en-tête de limite de débit. La clé est passée une fois par le Mac pour cet essai (une seule clé par compte) : elle est retirée du Mac, puis révoquée et recréée pour le NAS. |
| 3 nov. 2026 | **Interface, étape 10 : barre latérale, adresses, vue de l'année et musique par onglet.** Adresses par un petit routeur maison sur l'History API, sans dépendance (`web/src/route.ts`) : `/` suit aujourd'hui, `/jour/AAAA-MM-JJ`, `/mois/AAAA-MM`, `/annee/AAAA` ; `pushState` à chaque navigation (jours voisins compris), `popstate` pour Retour, une adresse invalide ou future remplacée par `/` ; nginx et `vite preview` servent déjà `index.html` pour toute adresse. Barre latérale (`Sidebar.tsx`) avec le seul « Calendrier » et « Se déconnecter » : colonne fixe de 240 px dès 900 px de large, sinon tiroir derrière un bouton rond de 44 px, en `transform` par transitions CSS interruptibles (`--ease-out`, 280 ms à l'ouverture, 220 ms à la fermeture, fondu de 150 ms en mouvement réduit), voile, Échap, focus dans le tiroir puis rendu au bouton, `inert` sur la page. Carte Musique : un seul sélecteur (`Segments`) sous les chiffres trie à la fois les découvertes (filtrées par niveau côté client, le serveur les trie déjà) et les écoutes ; 10 lignes, puis « Tout afficher (N) ». **Vue de l'année : `GET /v1/years/{AAAA}`**, même forme que le mois (`year`, `today`, `metrics`, `days`), derrière la session et hors de `READ_ROUTES` (le MCP n'en a pas besoin), `no-store`, année future ou mal formée refusée en 400. `calendar::month` devient un cas de `calendar::range(pool, début, fin)` : une seule lecture des couvertures, qui portent sur tout l'historique, au lieu de douze ; les requêtes SQL ne changent pas. Interface : douze mini-mois de six semaines, une pastille par jour. **Filtres de métriques communs au mois et à l'année** (`MetricFilters`) : une pastille par métrique du registre, donc sans code à changer pour un nouveau tracker (et réutilisable par la page du jour), point de la couleur de la métrique (plein si choisie, anneau sinon), teinte douce et libellé coloré une fois choisie ; choix multiple, la dernière métrique ne se retire pas, un seul choix gardé dans `localStorage` (`calendar.metrics`), toutes par défaut. **Une seule métrique : rien ne change.** Le mois montre ses chiffres (sans chiffre, pas de données ; 0, rien ce jour-là) ; l'année, son intensité, une pastille par jour : `null` en contour seul, 0 en teinte la plus pâle, puis quatre paliers par quartiles des valeurs non nulles de l'année (le maximum toujours au plus foncé, une année uniforme aussi), par `color-mix` de la couleur de la métrique sur le fond de carte. **Plusieurs métriques : des points façon Calendrier d'Apple, dans le mois comme dans l'année** (`DayDots`, seul rendu des deux vues ; `dayActivity` et `visibleDots` dans `metrics.ts`). Sous le numéro du jour, un point par métrique choisie active ce jour-là (valeur > 0), dans sa couleur et l'ordre du registre ; les points disent la présence, pas l'intensité. Année : le même zoom que l'intensité (mêmes colonnes de mini-mois, une case carrée par jour), numéro de 10 px, jusqu'à trois points de 4 px (5 px dès 600 px de large). Mois : des points de 7 px, quatre par ligne, huit au plus puis « +N », sans chiffres. Un jour sans données pour toutes les métriques choisies a son numéro grisé (le gris du contour vide) ; un jour connu sans activité garde un numéro normal, sans point. Aujourd'hui garde son disque, les jours futurs restent vides, les gestes ne changent pas. Les libellés d'accessibilité nomment chaque métrique par son nom (« pas de données » une seule fois si aucune n'en a), le résumé de chaque mini-mois aussi. Pourquoi les points : lisibles à la taille d'un mini-mois, ils tiennent jusqu'à une vingtaine de métriques (trois points et « +N » ne grandissent pas avec le registre), et l'année garde le même zoom que pour une seule métrique. Écartés après maquettes : les pastilles partagées en parts (`conic-gradient`, illisibles à cette taille), les chiffres dans l'année (une seule colonne de mini-mois sur iPhone pour loger « 1,2 k »), la rosace (secteurs par métrique, intensité en longueur) et les anneaux concentriques, tous illisibles au-delà de trois ou quatre métriques. Pas de case pour la nuance ni de « focus » sur une métrique : n'en choisir qu'une revient à la vue d'intensité. La légende suit le mode : une rampe ; la couleur de chaque métrique et la différence entre vide et 0 ; ou la puce de chaque métrique et « Un point par activité du jour. Numéro grisé : pas de données. » |
| 3 nov. 2026 | **Artistes crédités : un titre à plusieurs artistes compte pour chacun** (lève en partie la limite « artiste combiné » de la section Musique). `music::credited_artists(artiste, titre)` coupe le nom d'artiste à `&`, `, `, ` / `, ` + `, ` x `, ` et `, ` and ` et aux mentions feat., ft., featuring, with et avec, entre parenthèses ou non (« feat »/« ft » sans point seulement sans forme pointée : « Little Feat » reste entier), puis ajoute les invités d'une mention feat. du titre (with et avec n'y comptent qu'entre parenthèses : « Dance with Me » n'a pas d'invité). Doublons retirés par clé, noms vides écartés, un nom réduit à rien reste entier. `UNSPLIT_ARTISTS`, une clé repliée par ligne dans `music.rs`, protège les vrais groupes, seuls ou dans un crédit ; en ajouter un demande un `admin rebuild`. Le premier nommé est l'**artiste principal** : `artist_key`, `album_key` et `track_key` le prennent, donc « A2H & Dau » et « A2H & zek » partagent leur album, et « A & B – T » est le titre « A – T (feat. B) ». Une écoute porte un élément `artist` par crédit : `derived.event_items` gagne `position` (0 = principal) et sa clé devient `(generation, seq, ordinal, level, position)`, par une migration de schéma ; les données de `derived` ne se migrent toujours pas. Une écoute reste un événement : écoutes, titres et minutes comptent des événements ; `top` compte `occurrences_with_item` par événement distinct ; le contexte d'un titre joint ses artistes (« A, B & C »). La page du jour liste un artiste par crédit, les albums sous l'artiste principal. Les interprètes de musique passent en version 2 ; la clé `track_info` du collecteur suit (titres à plusieurs artistes redemandés une fois, la fiche la plus récente fait foi). Après le déploiement, `admin rebuild --allow-differences` sur le NAS : l'historique prend les nouvelles clés, et les différences sont attendues |
| 3 nov. 2026 | Image du backend en étapes **cargo-chef** (0.1.78) : les dépendances se compilent dans une couche qui ne dépend que de `Cargo.toml` et `Cargo.lock`, gardée dans le cache GitHub (`type=gha`, `mode=max`). `COPY . .` avant la compilation recompilait tout à chaque changement de `src/` : 7 min pour l'image de `v0.1.1`, contre 25 s pour `web` et `mcp`. Le binaire ne change pas ; le choix de l'éditeur de liens et du compilateur C de la compilation croisée passe par `/cross.sh`, lu par chaque étape |
| 3 nov. 2026 | **Déploiement continu à la place de release-please** (remplace l'entrée « Versions par release-please »). Plus de PR de release : quand la CI d'un push sur `main` est verte (`workflow_run`), `deploy/next-version.sh` compare le commit au dernier tag `vX.Y.Z` atteignable. Il y a release si le contenu d'une image a changé (`backend/`, `web/`, `mcp/`, hors `backend/tests/`, `web/e2e/`, `mcp/test/` et Markdown), quel que soit le type des commits : un `refactor` change ce qui tourne, et ne pas le livrer ferait diverger le NAS de `main` ; un `feat` qui ne touche que la doc ne livre rien. Le type ne fixe que le numéro : rupture (`!` ou `BREAKING CHANGE:`) mineure avant 1.0.0 et majeure ensuite, `feat` mineure, tout le reste correctif. Le job crée le tag et la GitHub Release (notes en français : Fonctionnalités, Corrections, Performances, Autres changements ; ni `docs`, ni `test`, ni `ci`), puis images et déploiement depuis le même commit, que la CI a testé. Plus de `version.txt` ni de `CHANGELOG.md` : les notes vivent dans les GitHub Releases, et `v0.1.0` sert de départ. Une release à la fois, sans annulation ; une seconde en attente remplace la précédente et reprend tous les commits depuis le dernier tag. Retour arrière par `workflow_dispatch` avec un tag existant : ni calcul ni images, déploiement depuis le checkout de ce tag pour que l'empreinte de `compose.yaml` corresponde. Un commit plus ancien que la dernière version (run relancé) ne publie rien. Les garde-fous restent : tests complets sur chaque PR puis sur `main`, refus d'un `compose.yaml` différent, retour seul à l'ancien tag |
| 3 nov. 2026 | CI en jobs parallèles (`.github/workflows/ci.yml`) : `db`, `backend` (format, clippy, tests), `web`, `mcp` et le script de déploiement, au lieu d'un `./test.sh` en série (14 min). Cache Rust partagé (`Swatinem/rust-cache`, clé `backend`, écrit par le seul job `backend`) ; Chromium en cache. `backend/test.sh` passe par cargo-nextest s'il est installé, sinon `cargo test` : les binaires de test tournent en parallèle, chaque test d'interprétation a déjà sa base, et ceux de l'ingestion leur source. Rien ne se lance pour un changement de documentation ni pour une PR de release seule. `./test.sh` reste le tout-en-un local |
| 3 nov. 2026 | Travail par PR, par convention : le dépôt privé du plan gratuit n'a ni règles de branche ni approbation d'environnement. Branche `type/sujet`, fusion en squash par l'utilisateur seul ; le titre de la PR, contrôlé par `pr-title.yml`, suit Conventional Commits (`type(scope): description en français`) et devient le commit sur `main` |
| 3 nov. 2026 | Versions par release-please : une seule version pour tout le dépôt (`simple`, `version.txt`, `CHANGELOG.md` en français), tags `vX.Y.Z`, `feat` mineure et rupture mineure avant 1.0.0, 1.0.0 à la fin du POC. Départ : manifeste à `0.0.0` (« jamais publié »), `initial-version` 0.1.0 et `bootstrap-sha` sur le dernier commit d'avant : la première release est `v0.1.0`, quel que soit le type du premier commit visible, et rien n'est à retirer ensuite. Les images ne se construisent plus à chaque push, seulement à la release, étiquetées `vX.Y.Z` |
| 3 nov. 2026 | Déploiement poussé à chaque release par GitHub Actions : nœud Tailscale éphémère `tag:ci` (client OAuth), qui ne joint que le port 22 du NAS, et clé SSH dédiée à commande forcée. Le NAS n'accepte que `deploy vX.Y.Z <sha256 de compose.yaml>` (`deploy/deploy.sh`, root par une seule règle sudoers) : il refuse si son `compose.yaml` diffère, réécrit `TRACKER_TAG`, `pull`, `up -d`, attend les services sains 3 min au plus, sinon revient à l'ancien tag. Le job ne pousse aucun fichier : `compose.yaml`, `deploy.sh` et l'administration restent à la main. Les migrations étant en ajout seul, l'ancien code tourne sur le schéma plus récent. Le collecteur compte comme sain dès qu'il tourne : son premier contrôle de santé n'a lieu qu'au bout de 5 min |
| 3 nov. 2026 | Exposition du MCP : **Funnel sur le port 443, la web app en `tailscale serve` privé sur 8443**, le repli prévu le 2 nov. Claude.ai n'atteint pas une URL de connecteur avec le port 8443 (« Couldn't reach this address », aucune requête dans les journaux du MCP, alors que l'iPhone en 4G obtient le JSON). `MCP_PUBLIC_URL` sans port ; `tailscale funnel` ne vise jamais 8443, et la web app ne revient pas sur 443 tant que Funnel y est |
| 3 nov. 2026 | Connexion au MCP réservée au tailnet : **`/authorize` exige aussi l'en-tête `Tailscale-User-Login`**, que `tailscaled` pose pour un utilisateur du tailnet et retire s'il vient du client. Le paquet Tailscale de Synology ne pose pas `Tailscale-Funnel-Request` : la page s'ouvrait depuis Internet. Le refus sur `Tailscale-Funnel-Request` reste |
| 3 nov. 2026 | Page de consentement du MCP en **`Referrer-Policy: same-origin`** (en-tête et balise `<meta name="referrer">`) : sous `no-referrer`, Chromium envoie `Origin: null` avec le formulaire, que le contrôle d'origine du SDK refuse. Le référent ne sort pas du NAS |
| 2 nov. 2026 | Exposition du MCP (étape 9) : **Tailscale Funnel sur le port 8443 seulement**, vers le conteneur `mcp` publié sur `127.0.0.1:18443` du NAS ; la web app reste en `tailscale serve` privé sur 443, et `tailscale funnel` ne vise jamais 443 (le runbook le dit). Repli si Claude.ai refuse un port : Funnel sur 443 pour le MCP, la web app déplacée en serve privé sur 8443. Sur Internet ne répondent que `/mcp`, `/token` et `/.well-known/*` ; tout le reste répond 404, et l'en-tête `Host` doit porter le nom public (protection contre le DNS rebinding du SDK) |
| 2 nov. 2026 | Authentification du MCP : **un mini serveur d'autorisation OAuth dans le conteneur MCP, à enregistrement fermé**. Un seul client, Claude, par son document CIMD (`client_id = https://claude.ai/oauth/mcp-oauth-client-metadata`) accepté sans être récupéré (aucune sortie Internet), retours codés en dur (`claude.ai` et `claude.com`) ; pas de `registration_endpoint`, client public (`none`), PKCE S256 obligatoire, `resource` vérifié à l'autorisation, à l'échange et à chaque appel (RFC 8707, audience exacte : l'URL du connecteur, chemin compris). Connexion par un secret propre au MCP, différent du mot de passe web, haché en Argon2id (paramètres de l'OWASP, une vérification à la fois) dans `secrets/mcp.env`, comparé en temps constant ; cinq échecs en quinze minutes bloquent toute connexion, limite globale puisqu'il n'y a qu'un compte et que l'adresse ne dit rien derrière Funnel. Page de consentement avec l'hôte de retour, CSRF par cookie `SameSite=Strict` doublé d'un champ, contrôle de l'origine, CSP sans script. **`/authorize` refuse toute requête portant `Tailscale-Funnel-Request`** : on se connecte depuis le tailnet, à la même adresse, que MagicDNS résout en direct vers le NAS. Jetons opaques de 32 octets, hachés en SHA-256 dans un fichier JSON du volume `mcp-state` (pas dans PostgreSQL : seul le backend touche la base), une seule autorisation à la fois (une nouvelle connexion remplace la précédente) ; accès 1 h, rafraîchissement 30 jours avec rotation à chaque usage, l'ancien rafraîchissement ou un code rejoué révoquant tout ; code d'une minute, à usage unique. Rien de tout cela dans les journaux : ni jeton, ni code, ni adresse. Serveur d'autorisation écrit à la main (les assistants du SDK v1 sont gelés) ; côté ressource, `requireBearerAuth` (avec `expectedResource`, transmis depuis `@modelcontextprotocol/express` 2.0.2) et `mcpAuthMetadataRouter` du SDK v2 |
| 2 nov. 2026 | Accès du MCP à l'API de lecture : un **jeton de service**, table `core.service_tokens` (migration de l'étape 9), créé et révoqué par `admin create-service-token` et `revoke-service-token`, haché comme les tokens de source. Une table à part et non une source : il ne fait que lire. Le contrôle d'accès unique du backend l'accepte sur une liste fermée de routes de lecture, en GET seulement, et le refuse partout ailleurs (ingestion, annulation, « déjà connu », signal de vie, déconnexion, routes futures) sans repli sur la session ; il pose le niveau « pseudonymes » (la session pose « noms ») ; aucune réponse du POC ne lit encore l'annuaire. Le conteneur MCP n'a ni la base ni Internet : réseau interne `read` vers le seul backend, et le réseau de son port publié sans NAT sortant |
| 2 nov. 2026 | Outils MCP : les dix de l'architecture, tous génériques et annotés en lecture seule (`inventory`, `search`, `occurrences`, `cumulative_duration`, `discovery`, `last_time`, `top`, `discoveries`, `day_summary`, `completeness`). Ce qui manquait est entré dans l'API de lecture, de façon générique et calculé à la lecture : `GET /v1/inventory`, `/v1/search`, `/v1/occurrences` (liste, nombre par jour, première et dernière de chaque jour), `/v1/durations` (la durée mesurée par la source, `played_ms`, sinon la dernière valeur de la référence que désigne `duration_from`), `/v1/item` (première et dernière occurrence, marquages « déjà connu », date de mise en ligne), `/v1/top`, `/v1/discoveries` ; la métrique et le niveau en paramètre, validés contre le registre, une erreur ne nommant que le paramètre. Un élément se désigne par sa clé normalisée, que Claude obtient par la recherche ; son contexte (l'artiste d'un titre) vient des niveaux plus larges de son événement le plus récent. Le résumé d'une journée et la complétude composent dans le MCP les lectures existantes (mois, santé) et les nouvelles. Résultats en JSON compact, bornés à 100 000 caractères |
| 2 nov. 2026 | Serveur MCP : TypeScript sur Node 22.16.0 (la version de la web app), SDK officiel v2 figé (`@modelcontextprotocol/server` 2.3.1, `express` 2.0.2, `node` 2.1.1), Express 5.3.0, Argon2id par `hash-wasm` (WebAssembly, rien à compiler). Transport Streamable HTTP sans état, une instance par requête : l'ère 2026-07-28 et, par le repli par défaut du SDK, les clients de l'ère 2025. Image multi-étapes, utilisateur 10001, système de fichiers en lecture seule, 128 Mo (35 Mo mesurés au repos), contrôle de santé par le `wget` de busybox sur les métadonnées. `@hono/node-server`, tiré par le SDK, a un avis de sécurité sur `serveStatic`, jamais utilisé ici |
| 2 nov. 2026 | Calendrier et complétude (étape 8), sans migration ni table : `GET /v1/months/{AAAA-MM}`, derrière la session, calculé à chaque lecture comme le reste. Générique : les métriques affichées sont celles du registre qui ne sont ni déclaration ni référence et ont un agrégat du jour. Par jour et par métrique, une valeur ou `null` (« pas de données ») : une métrique est suivie de sa première donnée vivante à aujourd'hui, sauf dans un trou entre deux sources (chaque source couvre de sa première à sa dernière donnée, deux couvertures qui se touchent n'en laissent pas) ; ailleurs, un jour sans donnée est un vrai 0. Ainsi les années d'avant le café ou d'avant l'export Apple restent vides, sans alerte. Trois signaux par jour, jamais confondus avec une journée sans activité : **incomplète** (des enregistrements de ce jour, heure de Paris d'après `occurred_at`, sont en erreur dans la génération active, même annulés) ; **en attente** (par source, et non par métrique : une source muette selon la règle de `/v1/sources/health` met en attente les jours du jour de son dernier contact, ou de sa création, à aujourd'hui ; une source sans seuil, café, iOS ou import Apple, ne l'est jamais) ; **à vérifier** (une métrique au-delà de son seuil de fraîcheur, les jours qui suivent sa dernière donnée). Le compteur global est le nombre d'erreurs d'interprétation de la génération active, ajouté à `/v1/sources/health` (`errors`). Les rejets à l'entrée n'y sont pas : aucune source du POC ne les cache (la web app affiche le refus, le collecteur et l'import les comptent), et `rejected_count` ne se remet jamais à zéro ; ils reviendront avec l'agent Mac. Rien n'est gardé : `no-store` sur l'API, et l'app relit le mois à chaque ouverture, changement de mois, minute et retour au premier plan, la copie en mémoire ne servant qu'à l'affichage immédiat. Interface : une marque par case, la plus grave d'abord, en trois formes (point, anneau, losange), le détail dans le libellé et la page du jour. Mesure sur 60 000 écoutes synthétiques, base à 0,5 CPU : 133 ms pour les couvertures (tout l'historique), 59 ms pour les valeurs d'un mois chargé, moins d'une milliseconde pour les erreurs |
| 1er nov. 2026 | Performance à l'étape 7, base à 0,5 CPU et backend natif sur un Mac, sans preuve pour le NAS. Worker : un lot de 100 par type et par transaction, chaque enregistrement gardant son point de sauvegarde et le curseur la même transaction. 30 enregistrements/s avec un seul par transaction, environ 200 par lots (28 295 en environ 145 s). Pendant ce rattrapage, un scrobble s'écrit en 50 ms et s'interprète en 0,3 à 0,7 s, une page se lit en moins de 30 ms. Lecture d'un jour : le fuseau passe de `CROSS JOIN core.settings` à une sous-requête. Cette table d'une ligne, jamais analysée, passe pour 1 250 lignes, et le plan parcourait tout le journal : de 0,8 à 1,1 s à 13 à 30 ms, pour 16 et 248 écoutes, sans migration |
| 1er nov. 2026 | Nom trop long pour l'index des éléments (54000) : une erreur d'interprétation, code exact dans le moteur, plutôt qu'une panne qui bloquerait le worker. Pas de limite ajoutée à l'interprète Last.fm : une écoute qui passe aujourd'hui doit rester identique au rebuild |
| 1er nov. 2026 | Priorité de Last.fm : une ligne de `core.source_precedence`, posée sur le NAS par `admin set-precedence`, à l'instant de la première écoute Last.fm en base. Refusé si un écart de plus de 7 jours la sépare de la suivante (scrobble ancien), `--from` tranche alors. `admin coverage` montre le trou ou le chevauchement avec la fin de l'export. Un jour entre deux couvertures dit « pas de données » : `/v1/sources/health` donne la couverture de chaque source. Un scrobble arrivé plus tard avec une date antérieure à la priorité s'ajoute aux écoutes Apple du moment (limite). Remplacer la source Last.fm (`lastfm-2`) demande de revoir cette ligne : elle ne nomme qu'une source |
| 1er nov. 2026 | Écoute sans artiste attribuable : au journal avec `artist` vide, comptée, avec sa durée, et listée « Artiste inconnu » ; aucun élément, donc ni artiste, ni album, ni découverte. Mesure : 99 écoutes sur 20 682 (0,48 %), 34 titres. Une écoute Apple n'a pas besoin de fiche Last.fm : sa durée est celle jouée, bornée par celle du titre (20 lignes annoncent plus, jusqu'à 47 h) |
| 1er nov. 2026 | Attribution de l'artiste, par l'outil d'import, jamais choisie entre plusieurs : titre et album en bibliothèque, puis titre dans Daily Tracks à l'heure près, au jour près, sur tout l'historique, puis titre en bibliothèque. Daily Tracks est en UTC (20 440 écoutes retrouvées au jour près et 19 648 à l'heure près en UTC, contre 20 127 et 2 930 en heure locale). Mesure sur les 20 682 écoutes : 18 779 par bibliothèque, 1 720 à l'heure, 69 au jour, 15 sur l'historique, 0 ambiguë, 99 inconnues. Par titre seul, 1 158 seraient ambiguës. Contrôle : là où les deux sources existent le même jour, bibliothèque et Daily Tracks s'accordent 18 632 fois sur 18 651. La méthode est au journal (`artist_source`) |
| 1er nov. 2026 | Export Apple, source de vérité : les lignes `PLAY_END` de Play Activity, horodatées à la milliseconde, et non Daily Tracks, agrégé à l'heure. L'artiste y est ajouté comme une déduction. `PLAY_START` est ignoré : il n'apporte rien au `PLAY_END` qui suit, n'existe qu'à partir du 19 oct. 2022 (d'où 1 702 contre 7 830 `PLAY_END` en 2022), et porte toutes les répétitions d'`Event ID` (76). Ignorés aussi, avec motif compté : `LYRIC_DISPLAY`, vidéo, sans heure de début (247, toutes de durée négative), sans titre, durée nulle |
| 1er nov. 2026 | Une écoute Apple suit la règle de Last.fm : un titre de plus de 30 s joué jusqu'à la moitié ou 4 min. Une ligne compte si elle franchit ce point de la piste (`début < seuil ≤ fin`). Ainsi l'interprète reste sans mémoire, et une pause ou une reprise ne compte qu'une fois : 20 682 écoutes, contre 20 616 si l'on regroupe les morceaux d'une lecture et 20 520 si l'on exige la durée sur une seule ligne. Limite : un retour en arrière qui refranchit le seuil compte une seconde écoute. Rapport au Play Count de Daily Tracks, plus généreux, par année de 2022 à 2026 : 5 032/5 423, 4 088/4 387, 2 926/3 136, 5 244/5 504, 3 392/3 541, soit 93 à 96 %. Toutes les lignes lues restent au journal, sous le seuil compris : changer la règle demande un rebuild, pas un nouvel import. Les minutes d'une écoute reprise ne comptent que le morceau qui franchit (3 à 6 % de moins que la somme des lignes, selon l'année) |
| 1er nov. 2026 | Clé d'une ligne Apple : instants de début et de fin en millisecondes et positions dans le titre. Unique sur les 28 319 lignes à durée positive (les instants seuls laissent 1 collision), indépendante de l'ordre, de l'écriture des dates, d'`Event ID`, de la bibliothèque et de tout ce qui peut changer. Une seule archive disponible : testé sur un export simulé, pas sur un vrai second export. Elle ne vaut que sous la même source : garder `apple`, qu'un nouvel export réutilisera |
| 1er nov. 2026 | Confidentialité : l'export Apple n'entre pas « tel quel » au journal, qui dure à vie. Le code ne lit que 14 colonnes de Play Activity et construit le payload champ par champ : jamais d'IP, de ville, de coordonnées, d'Apple ID, d'identifiant ou de nom d'appareil, d'abonnement. Un test fige la liste des champs et une sentinelle par colonne sensible. Remplace « colonnes telles quelles » et le principe de brut riche pour cette source |
| 1er nov. 2026 | Outil d'import : sous-commande `import-apple` de la même image, service `apple-import` sur le seul réseau `collect` (ni base, ni Internet), archive en lecture seule, token sur l'entrée standard, jamais en argument ni dans l'environnement. Pas d'état local : relancer renvoie tout, en `duplicate` pour ce qui est passé. Sans dépendance nouvelle : un petit lecteur CSV, et le JSON de la bibliothèque est dézippé à la main, ce qui n'emporte que trois fichiers sur le NAS. Client d'ingestion partagé avec le collecteur (`ingest_client.rs`). Normalisation inchangée : sur les seules données Apple, rien n'oblige à une règle nouvelle (846 noms pour 846 clés d'artiste ; 131 titres sur 3 408 sous plusieurs artistes ; seules 5 des 566 clés d'album en `- Single` ou `- EP` ont aussi leur nom de base). La comparaison à Last.fm reste à lire sur le NAS, voir le runbook |
| 1er nov. 2026 | Performance des lectures, mesurée sur 60 000 écoutes (89 600 enregistrements, 210 000 éléments) : `GET /v1/days/{date}/music` prenait 5,8 s pour un jour de 314 écoutes et 0,8 s pour un jour de 10, sur un Mac, parce que la vue cache le numéro de génération aux index et que le JIT compilait pour un coût estimé très surévalué. Les lectures fixent donc `e.generation` et le pool coupe le JIT, à 65 ms et 50 ms, sans nouvel index. Base limitée à 0,5 CPU, pour approcher le NAS sans le prouver : 17 s et 5 s avant, 0,2 à 1 s après. Juste après un `rebuild`, compter environ 0,3 s jusqu'au passage d'autovacuum |
| 1er nov. 2026 | Découvertes : calculées à la lecture dans `GET /v1/days/{date}/music`, requête de l'API comme les minutes, sans vue SQL ni migration. Un élément est découvert ce jour-là s'il n'a aucune écoute vivante avant minuit, heure de Paris, à son niveau. Un marquage agit quelle que soit sa date, passée ou future. `first_listens` garde les éléments marqués, avec leurs `known_marks`, pour pouvoir annuler depuis la même ligne ; `discoveries` ne compte que les autres. Le jour d'une découverte suit donc les annulations, les données tardives et, à l'étape 7, l'historique Apple |
| 1er nov. 2026 | « Déjà connu » : charge utile `{level, artist, album, track}`, un seul élément au niveau déclaré, sous la clé recalculée par la normalisation. Le nom du jour suffit à retrouver la clé, quelle que soit l'orthographe. Annuler vise tous les marquages vivants de l'élément, en un lot, puisque deux appareils peuvent l'avoir marqué. L'écran montre le geste tout de suite et attend de voir la clé précise côté serveur ; un second toucher est ignoré pendant ce temps. Un nom de plus de 1 024 octets est une erreur d'interprétation, car il dépasserait l'index et ferait passer le refus pour une panne. Aucun droit à accorder : la migration initiale les donne à `webapp` |
| 1er nov. 2026 | Santé et lecture de la musique : `GET /v1/days/{date}/music` et `GET /v1/sources/health`, derrière la session. Les minutes croisent écoutes et fiches à la lecture ; une écoute sans durée connue n'est pas comptée et cela s'affiche, `null` si aucune n'en a. Liste : les plus écoutés, puis par nom ; nom affiché : le dernier vu ; l'artiste du jour est le premier de sa liste. Une métrique sans aucune donnée n'est pas « à vérifier ». Une journée sans écoute ne dit « aucune écoute » que si Last.fm est relié, non muet, et que la date suit la première écoute |
| 1er nov. 2026 | Normalisation (`backend/src/music.rs`) : formes Unicode, minuscules, ponctuation typographique ; les accents restent. Un remix ou un « mix » reste un titre à part, autre œuvre et autre durée ; les autres versions (live, remaster, edit, acoustique, deluxe…) sont retirées des parenthèses, crochets et suffixes « - … ». Le nom d'un album n'est pas réduit (limite connue). Limite : « Song (Live Forever) » perd sa parenthèse. Changer ces règles demande un `admin rebuild --allow-differences` ; les clés du journal n'en dépendent pas pour les écoutes |
| 1er nov. 2026 | Clés Last.fm : `horodatage:SHA-256(artiste, titre tels que renvoyés)` pour un scrobble, SHA-256 de la clé normalisée pour une fiche, ce qui borne la longueur et garde les noms hors des clés. La fiche enveloppe la réponse brute (`{artist, track, info}`) : la durée se rattache au titre tel que scrobblé, pas à celui que Last.fm a pu corriger. Une seule fiche par titre : une durée absente ou nulle ne se redemande pas, car la clé la dédoublonnerait |
| 1er nov. 2026 | Collecte : relecture des 48 dernières heures à chaque passage (5 min), car le scrobbler rattrape avec des dates passées ; une écoute arrivée plus de 48 h en retard est manquée. Lecture de la plus ancienne page à la plus récente sur une fenêtre fermée ; le curseur avance à chaque page accusée, et un `total` qui change arrête le passage au dernier point sûr. Curseur et fiches à demander dans un volume dédié : le perdre ne coûte qu'un rattrapage en `duplicate` |
| 1er nov. 2026 | Rebuild : le rattrapage ne repasse que sur les types en retard. Avec deux types de plus, les passages à vide laissaient au worker le temps de le devancer à chaque bascule, sous un flux d'écritures soutenu |
| 1er nov. 2026 | Le collecteur tourne dans son propre conteneur, avec la même image, et non dans le backend : lui seul a une sortie vers Internet, la clé Last.fm et le token de sa source, sans accès à la base ; le backend et la base gardent zéro sortie. Sortie limitée au conteneur, pas à la destination : Docker ne filtre pas par nom, et un proxy de sortie dédié n'en vaudrait pas la peine pour le POC. TLS par rustls sur `ring` (compilable en croisé sans CMake), redirections refusées pour que la clé ne parte pas ailleurs |
| 31 oct. 2026 | Déploiement (voir [le runbook](deploiement.md)) : un fichier de secrets par service, lu par ce seul service. Réseau `data` interne et sans adresse côté hôte, pour qu'un compte du NAS ne joigne pas la base ; le web n'y est pas. Seul `web` publie un port, sur `127.0.0.1` par défaut, derrière `tailscale serve`. Le contrôle de santé signale une panne de la base sans redémarrer le backend. Rotation des logs par taille (3 × 10 Mo), Docker ne sachant pas purger par âge : les 30 jours restent un ordre de grandeur |
| 31 oct. 2026 | Reverse proxy : nginx sans privilèges. L'API reste seule juge du 413 jusqu'à 2 Mo ; au-delà, nginx refuse seul avec le même corps JSON, sans mise à jour du dernier contact. Backend en binaire statique musl compilé en croisé, sur Alpine pour le `wget` du contrôle de santé |
| 31 oct. 2026 | Web app, côté client : le + un jour passé ouvre le choix de l'heure, puisque « maintenant » n'y a pas de sens. Une heure qui existe deux fois (02:30 le 25 oct. 2026) désigne son premier passage, en heure d'été ; une heure qui n'existe pas (02:30 le 29 mars 2026) ou pas encore est refusée. Une annulation acceptée masque sa cible pour toute la vie de la page, et un café reçu reste affiché jusqu'à ce que le serveur le montre : le nombre ne contredit jamais le geste. Pas de service worker pendant le POC : l'app s'installe sans lui, et rien n'est gardé hors ligne |
| 31 oct. 2026 | API de lecture : `GET /v1/days/{date}`. `derived.live_events` choisit les événements ; le journal et le registre des sources ne donnent que la clé et le nom de la source, nécessaires à l'annulation. Aucune réponse de l'API n'est mise en cache (`no-store`) |
| 31 oct. 2026 | Sécurité de la web app : toute route exige la session sauf une liste fermée (`/health`, connexion, signal de vie par token), par un seul contrôle posé sur tout le routeur, chemins inconnus compris. CSRF : `SameSite=Strict` et en-tête `X-CSRF-Protection` obligatoire sur toute écriture par cookie, connexion comprise, un autre site ne pouvant pas l'ajouter puisque le serveur n'autorise aucun CORS. `Secure` sauf `DEV_INSECURE_COOKIE=true`, réglage de développement : le NAS devra servir la web app en HTTPS |
| 31 oct. 2026 | Compte web : Argon2id aux paramètres par défaut de la bibliothèque (19 Mio, 2 passes), deux vérifications à la fois au plus, une seule réponse à tout échec. Mot de passe de 12 caractères au moins, posé par `admin set-password`, jamais en argument ; le changer ferme les sessions. Session de 30 jours à durée fixe depuis la connexion, sans renouvellement |
| 31 oct. 2026 | La web app écrit par `POST /v1/records`, le même code que les tokens (principe 1). La source vient de l'authentification, jamais du corps : un `Authorization` passe toujours par le token, sans repli sur le cookie ; sinon la session vaut pour `webapp` et rien d'autre. `/v1/heartbeat` reste réservé aux tokens |
| 31 oct. 2026 | Rejeu (voir [la base de données](database.md)) : la reconstruction est faite par la seule commande `rebuild`, le worker ne traite que l'active. Comparaison et bascule dans une même transaction, jusqu'au plus petit des curseurs de chaque type, en ignorant horodatages et version d'interprète. Une construction interrompue est effacée, pas reprise ; l'ancienne génération est supprimée aussitôt après la bascule |
| 31 oct. 2026 | Annulations : la source visée est désignée par son nom, stable et jamais renommé, et résolue par le moteur. Toute source qui a le droit `record.cancel` peut annuler n'importe quelle source. Annuler une annulation est sans effet : on saisit de nouveau la donnée |
| 31 oct. 2026 | Interprétation : un interprète ne reçoit que l'enregistrement, et ses messages d'erreur sont des textes fixes, pour ne jamais citer la charge utile. Une sortie refusée par la base (classes 22 et 23) est une erreur d'interprétation ; toute autre erreur est une panne, jamais enregistrée comme erreur. Un format inconnu est une erreur d'interprétation : pas de convertisseur tant que seule la version 1 existe |
| 31 oct. 2026 | Ingestion : une transaction par lot, une seule instruction d'ajout. Une charge utile que jsonb ne peut pas stocker (`\u0000`, nombre hors bornes) est rejetée seule, sans quoi le lot échouerait à chaque nouvel essai ; c'est la seule limite à la charge utile. `-00:00` vaut « décalage inconnu », comme `Z` (sens de la RFC 3339), et `+00:00` un décalage nul. Un appel authentifié met à jour le dernier contact même s'il finit en 400 ou 413 |
| 31 oct. 2026 | Accès par Tailscale pendant le POC ; à terme, web app ouverte sur Internet avec authentification |
| 31 oct. 2026 | Messages sortis du POC (agent à part sur le Mac) ; POC découpé en tranches verticales, déployé sur le NAS juste après le café (voir [le POC](poc.md)) |
| 17 oct. 2026 | Schéma de la base (voir [la base de données](database.md)) : PostgreSQL 18 figé ; schémas `core` (vérité, référentiels) et `derived` (recalculable) ; reconstruction par générations avec bascule ; verrou d'ajout pour un ordre global sans trou ; immuabilité garantie par les droits et des triggers ; trois rôles (propriétaire, application, administration) et service `migrate` ; enveloppe complétée par la version du client, l'identifiant de requête et un décalage horaire vide quand il est inconnu |
| 17 oct. 2026 | Interface dans l'esprit d'Apple (clair et sombre, pensée pour l'iPhone), guidée par les skills `apple-design`, `emil-design-eng` et `mobile-native` |
| 13 oct. 2026 | Six principes ajoutés : une seule porte de sortie, un manque de données se voit, le moins exposé possible, ajouter un tracker c'est déclarer, le plus simple d'abord (pas d'interdit définitif), pensé pour durer |
| 13 oct. 2026 | Déploiement : dépôt GitHub privé, images construites par GitHub Actions et publiées dans le registre de GitHub ; un runner sur le NAS seulement pour déployer |
| 13 oct. 2026 | Web app protégée par une connexion (un seul compte) ; l'API de lecture n'accepte que cette session et le jeton de service du MCP |
| 13 oct. 2026 | Contrat de l'API d'ingestion v1 ; tout type de donnée doit être déclaré pour sa source |
| 13 oct. 2026 | Stack : backend Rust (axum, tokio, sqlx), agent Mac en Swift, web app React, serveur MCP en TypeScript, PostgreSQL dédié |
| 13 oct. 2026 | Quatre conteneurs : backend, PostgreSQL, web app, serveur MCP |
| 13 oct. 2026 | Le calendrier et le MCP lisent par l'API de lecture, jamais directement la base |
| 13 oct. 2026 | Un seul collecteur d'API avec un connecteur par service, passant par l'API d'ingestion |
| 12 oct. 2026 | POC sans sauvegarde ; clé de pseudonymisation copiée dans un gestionnaire de mots de passe |
| 12 oct. 2026 | Assistant de nommage reporté après le POC |
| 12 oct. 2026 | Annuaire : noms saisis à la main, jamais envoyés automatiquement par l'agent |
| 12 oct. 2026 | Exploitation : journal jamais migré, logs sans donnée personnelle (30 jours), token par source, clé de pseudonymisation sur le Mac, images versionnées |
| 12 oct. 2026 | Gestion des erreurs : brut écrit d'abord, enregistrement en erreur mis de côté, rejeu après correction |
| 12 oct. 2026 | Silence d'une source jugé avec un seuil propre à chaque source (café : jamais, Last.fm : 6 h, agent Mac : 7 jours) ; musique « à vérifier » après 3 jours sans nouvelle écoute |
| 12 oct. 2026 | Pour le POC, erreurs et sources muettes signalées dans le calendrier ; notifications iPhone plus tard |
| 12 oct. 2026 | Purge par rédaction en place, avec registre des purges |
| 12 oct. 2026 | Vues calculées à la volée pour le POC |
| 12 oct. 2026 | Musique : Last.fm fait foi sur la période de chevauchement avec l'export Apple |
| 12 oct. 2026 | Une journée bascule à minuit, heure de Paris (Europe/Paris) |
| 12 oct. 2026 | Claude ne voit que les pseudonymes des contacts, jamais les noms ; MCP en lecture seule |
| 12 oct. 2026 | Musique : Last.fm en direct via FastScrobbler (test concluant le 13 oct.), export Apple pour l'historique |
| 12 oct. 2026 | Pas de catalogue de titres pour le POC : clé artiste + titre normalisés |
| 12 oct. 2026 | Messages : envoyés et reçus ; conversations de groupe hors POC |
| 12 oct. 2026 | Trois découvertes musicales (artiste, album, titre), calculées en vue ; chacune peut être marquée « déjà connue » depuis l'app, le marquage n'agissant qu'à son niveau ; un élément marqué est « connu avant » la date de mise en ligne |
| 12 oct. 2026 | Musique : brut riche ; besoins = temps d'écoute par jour, titres, artistes, albums, découvertes |
| 12 oct. 2026 | Messages : uniquement le nombre et le contact, jamais le contenu |
| 12 oct. 2026 | La politique de données se décide source par source |
| 12 oct. 2026 | Journal brut immuable interprété par une couche centrale ; tout le reste est recalculable |
| 12 oct. 2026 | Messages iPhone captés via le Mac, en mode rattrapage (Mac pas toujours allumé) |
| 12 oct. 2026 | Trois types de sources : manuelle, API, appareil |
| 12 oct. 2026 | Auto-hébergé sur le NAS, avec calendrier et serveur MCP pour Claude |
