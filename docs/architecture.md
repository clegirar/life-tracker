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
12. **Ajouter un tracker, c'est déclarer, pas reconstruire.** Une nouvelle source se déclare dans les registres, avec son interprète et éventuellement un connecteur, sans rien changer au journal, aux outils MCP ni au calendrier générique. Un tracker qui exigerait une refonte est le signal qu'il faut repenser quelque chose.
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

**Collecte des API.** Un seul collecteur héberge un connecteur par service (Last.fm aujourd'hui, d'autres demain).

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

Authentification : `Authorization: Bearer <token>`. Le token identifie la source : la source n'est jamais écrite dans le corps, une source ne peut donc pas se faire passer pour une autre. Les tokens sont stockés hachés dans le registre des sources. Le client envoie sa version dans l'en-tête `X-Client-Version`. Tout appel authentifié met à jour la date de dernier contact ; un connecteur n'envoie donc son signal de vie qu'après avoir réussi à interroger son service. Exception à l'authentification par token : la web app passe par la session de connexion, qui vaut pour la source « web app », pour qu'aucun token ne soit jamais exposé dans le navigateur.

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
| Import Apple | `apple.play_history_row` | Empreinte de la ligne | Une ligne du CSV, colonnes telles quelles |

L'archive ZIP de l'export Apple suit la règle des gros fichiers : elle est rangée dans le stockage de fichiers du NAS, et un outil d'import en envoie les lignes une à une par l'API, comme n'importe quelle source.

## Fiches par source

Chaque source a sa fiche : ce qu'on veut en tirer décide ce que le brut contient et ce qui est filtré avant d'entrer. Messages : hors POC depuis le 31 oct. 2026.

|  | Café | Messages | Musique |
| --- | --- | --- | --- |
| Type de source | Manuelle | Appareil (agent Mac) | API + export ponctuel |
| Ce que je veux savoir | Nombre de cafés par jour | Nombre de messages envoyés et reçus, et avec qui | Temps d'écoute par jour, titres, artistes, albums, découvertes |
| Sensibilité | Faible | Forte | Faible |
| Contenu du brut | Un clic +1, ou une annulation | Métadonnées : identifiant du message, date, service (iMessage, SMS, RCS), sens (envoyé ou reçu), pseudonyme du contact | Brut riche, tel que fourni par la source |
| Filtré avant d'entrer | Rien | Le texte et l'identité en clair du contact | Rien |
| Interprète | Sans mémoire | Sans mémoire | Sans mémoire (Last.fm fournit des écoutes horodatées) |
| Référentiel | Aucun | Annuaire des contacts | Aucun pour le POC |
| Point d'attention | Bouton web ou raccourci iOS | Captation via `chat.db` sur le Mac, en mode rattrapage | Temps d'écoute estimé ; découverte calculée en vue |

**Messages.** L'iPhone n'est pas lisible directement ; avec Messages dans iCloud et le transfert des SMS, le Mac reçoit tout l'historique. L'agent tourne quand le Mac est allumé et reprend où il s'était arrêté : les dates restent exactes, seule la journée en cours peut être en retard. L'iPhone conserve les messages indéfiniment. Le journal ne contient jamais de nom : l'annuaire relie chaque pseudonyme à un nom affiché, que je saisis moi-même, et supprimer une ligne de l'annuaire anonymise l'historique de cette personne. Claude ne voit que les pseudonymes, jamais les noms. Le pseudonyme est une empreinte à clé (HMAC) du numéro ou de l'adresse, normalisé avant calcul (+33612… et 0612… donnent le même) ; un simple hachage serait réversible en essayant tous les numéros. Une même personne peut avoir plusieurs pseudonymes (numéro, e-mail) : l'annuaire en relie plusieurs à une seule personne.

**Musique.** Une « découverte » est un titre écouté pour la première fois de ma vie ; un artiste découvert se calcule de la même façon. Pour le POC, pas de catalogue : un titre est identifié par son artiste et son titre normalisés (minuscules, sans « feat. » ni mention de version). Un catalogue pourra venir plus tard, et les découvertes se recalculeront. Le temps d'écoute demande la durée de chaque titre, obtenue par un enrichissement enregistré dans le journal ; comme il croise deux enregistrements, il se calcule en vue. Si l'export Apple fournit la durée réelle d'écoute, la métrique sera plus précise avant le démarrage de Last.fm qu'après. Source en direct : Last.fm, alimenté par un scrobbler (FastScrobbler, validé en test, la même app sur iPhone et Mac). iOS pouvant suspendre l'app en arrière-plan, quelques écoutes peuvent arriver en retard ou manquer ; l'app rattrape en partie via l'historique Apple Music, et un raccourci y lance une analyse de l'historique chaque jour à 8h et 20h. L'export de données Apple (privacy.apple.com) fournit l'historique passé, en une fois.

**Marquer comme déjà connu.** Sans catalogue, tout titre jamais vu dans le journal est une découverte. Depuis le calendrier, je peux marquer un artiste, un album ou un titre « déjà connu » : c'est une déclaration de ma part, donc un enregistrement de la source manuelle dans le journal (comme un clic café), annulable par une annulation. Les vues des découvertes excluent les éléments marqués, chacun à son niveau. Le marquage garde l'artiste et le titre tels qu'affichés, pas seulement la clé normalisée, pour rester valable si la normalisation ou un futur catalogue change.

**Trois découvertes, trois marquages.**

- **Artiste découvert** : la première fois que j'écoute un artiste que je n'avais jamais écouté.
- **Album découvert** : la première fois que j'écoute un titre d'un album dont je n'avais jamais rien écouté.
- **Titre découvert** : la première fois que j'écoute un titre.

Je peux marquer un artiste, un album ou un titre « déjà connu » ; il ne compte alors plus comme découverte à son niveau. Chaque marquage n'agit qu'à son propre niveau : marquer un artiste ne dit rien de ses albums ni de ses titres, et marquer un album ne dit rien de ses titres.

Exemple : je marque Daft Punk « déjà connu », puis j'écoute pour la première fois un de leurs titres. Daft Punk n'est pas un artiste découvert, mais ce titre est bien un titre découvert.

Limite connue sans catalogue : les noms viennent tels que la source les envoie. Un artiste marqué ne couvre pas un artiste combiné (« A & B »), et un même album peut exister sous deux noms (« Album » et « Album — Single »).

### Musique : ce que montre la web app

Par jour :

- nombre de titres écoutés ;
- liste complète des titres, des artistes et des albums ou EP ;
- minutes d'écoute (estimées à partir de la durée des titres).

Ajouts retenus :

- **découvertes du jour** (titres, albums, artistes), avec le bouton « déjà connu » à côté de chacune ;
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

## Réconciliation multi-sources

Quand deux sources décrivent la même réalité, chaque fait risque d'exister deux fois ; la réconciliation se fait dans les vues, jamais dans le journal.

Premier cas : l'export Apple et le tracking en direct se recouvrent sur une période. Le cas reviendra avec d'autres trackers (pas comptés par l'iPhone et par une montre), d'où un mécanisme générique. Deux approches possibles, à choisir pour chaque nouveau cas :

- **Source prioritaire par période** : sur un intervalle donné, une seule source fait foi pour une métrique.
- **Rapprochement** : deux faits identiques à quelques minutes près sont fusionnés.

**Décision pour la musique : Last.fm fait foi sur le chevauchement.** L'export Apple n'est utilisé que pour la période antérieure au démarrage de Last.fm. Les écoutes de l'export qui chevauchent restent dans le journal, mais les vues les ignorent.

## Exposition et accès

Seul le serveur MCP est exposé sur Internet, car Claude.ai l'appelle depuis ses propres serveurs ; tout le reste reste privé derrière Tailscale.

| Composant | Joignable depuis | Protection |
| --- | --- | --- |
| Web app (calendrier) | Mes appareils, via Tailscale | Réseau privé + connexion (un seul compte) |
| API d'ingestion | Agents et raccourcis, via Tailscale | Un token par source |
| Serveur MCP | Internet (reverse proxy Synology avec domaine, ou Tailscale Funnel) | HTTPS + OAuth ; pseudonymes uniquement ; en lecture seule |
| Base de données | Le NAS uniquement | Non exposée |

Cette exposition vaut pour le POC. À terme, la web app sera elle aussi ouverte sur Internet, avec une authentification : voir [le POC](poc.md). Sa sécurité ne compte donc jamais sur Tailscale, et toute route exige la session.

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
| Agent Mac | Un battement à chaque passage, même sans nouveau message | 7 jours |

Agent Mac et messages : hors POC depuis le 31 oct. 2026.

**Seuil par source.** Chaque source déclare son seuil dans le registre des sources ; une nouvelle source choisit le sien en arrivant.

**Les battements ne vont pas dans le journal.** Ce sont des informations d'exploitation, pas des faits de ta vie : ils mettent seulement à jour la date de dernier contact dans le registre.

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
- [ ] Reste à trancher ensuite : l'authentification du serveur MCP (avant l'étape 9).

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
- [ ] API Garmin Connect (entraînements et sommeil)
- [ ] Temps d'écran sur l'iPhone
- [ ] Boîte mail (API Gmail ?)
- [ ] Déplacements via géolocalisation
- [ ] Recettes et dépenses sur différents comptes, pro et perso (probablement manuel)
- [ ] YouTube : ce que je regarde
- [ ] Twitch : ce que je regarde
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
