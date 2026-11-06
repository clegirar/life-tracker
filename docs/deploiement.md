# Life tracker — Déploiement sur le NAS

Runbook de l'étape 4, joué sur le NAS (voir « Ce qui reste à vérifier »). L'installation et les mises à jour de `compose.yaml` restent à la main ; depuis le 3 nov. 2026, tout changement du code de production fusionné dans `main` part sur le NAS de lui-même, une fois la CI verte (voir « Déploiement automatique »).

**Règle.** Rien de ce qui suit ne se copie dans le dépôt : ni `.env`, ni `secrets/`, ni token, ni mot de passe, ni sortie de commande qui en contient. Le dépôt ne reçoit que des modèles. Dans les commandes, `OWNER` est le compte GitHub, en minuscules.

## Ce qui tourne

| Service | Rôle | Reçoit | Réseaux |
| --- | --- | --- | --- |
| `db` | PostgreSQL 18.6, volume `life-tracker-prod-db-data` | `secrets/db.env` | `data` |
| `migrate` | Une fois à chaque `up`, puis s'arrête (état « arrêté » normal) | `secrets/migrate.env` (`tracker_owner`) | `data` |
| `backend` | API et interprètes | `secrets/backend.env` (`tracker_app`, zones de confidentialité du MCP) | `data`, `front`, `collect` |
| `web` | nginx : app statique et proxy de `/v1` et `/health` | aucun secret | `front`, `edge` |
| `collector` | Interroge Last.fm, Garmin et YouTube, écrit par l'API d'ingestion ; un état par connecteur (`state.json`, `garmin-state.json`, `youtube-state.json`) dans le volume `life-tracker-prod-collector-state` | `secrets/collector.env` (clés, utilisateur, un token par source) | `collect`, `egress` |
| `admin` | Profil `admin`, lancé à la main | `secrets/admin.env` (`tracker_admin`) | `data` |
| `apple-import` | Profil `import`, lancé à la main : l'import de l'export Apple | Le token sur l'entrée standard, l'archive en lecture seule | `collect` |
| `mcp` | Serveur MCP (étape 9) : OAuth et outils en lecture seule ; jetons OAuth hachés dans le volume `life-tracker-prod-mcp-state` | `secrets/mcp.env` (jeton de service, haché du secret de connexion) | `read`, `mcp-edge` |

`web` publie un port, sur `127.0.0.1:18080` par défaut, et `mcp` le sien, sur `127.0.0.1:18443` (Tailscale Funnel). `read` relie le seul MCP au backend, sans sortie ; `mcp-edge` ne porte que le port publié du MCP, sans NAT sortant : le MCP ne joint ni la base ni Internet. `data`, `front` et `collect` n'ont aucune sortie, et `data` n'a aucune adresse côté hôte. `egress` est la seule sortie vers Internet, et seul le collecteur y est : il ne voit pas la base, et le backend ne voit pas Internet. Docker ne filtre pas par destination : le collecteur pourrait joindre autre chose que Last.fm, ce que limitent l'absence de secret de base, `read_only` et l'absence de tout port entrant. Il ne connaît que ce qu'il envoie déjà.

## Ce que je fais à la main

### 1. Dépôt privé et images

Sur le Mac, à la racine du dépôt, une fois les changements commités :

```sh
gh repo create life-tracker --private --source=. --push
```

À l'origine, le push sur `main` lançait `images.yml`, qui construisait des images étiquetées par SHA de commit. Depuis le 3 nov. 2026, les images ne se construisent qu'à une release, étiquetées `vX.Y.Z`, que crée chaque changement du code de production fusionné dans `main` (voir « Déploiement automatique ») : `TRACKER_TAG` reçoit alors ce tag. Une fois le workflow vert :

```sh
gh run list --limit 1
git rev-parse HEAD   # le tag des images, à garder pour TRACKER_TAG
```

Vérifier sur GitHub, dans *Packages*, que `life-tracker/backend` et `life-tracker/web` sont privés.

### 2. Token de lecture pour le NAS

GitHub, *Settings > Developer settings > Personal access tokens > Tokens (classic)* : une date d'expiration et la seule permission `read:packages`. D'après la [documentation de GitHub Packages](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry), le registre demande un token classique. Le token va dans le gestionnaire de mots de passe, jamais ailleurs.

### 3. Prérequis sur le NAS

- Container Manager installé et SSH activé (*Panneau de configuration > Terminal*).
- Tailscale installé d'après le paquet manuel recommandé par [sa documentation](https://tailscale.com/kb/1131/synology), version 1.52 au moins pour la syntaxe de `serve`.
- Connexion en SSH avec mon compte administrateur. Les autres comptes du NAS ne doivent pas être administrateurs : un administrateur peut passer root, et aucune permission ne l'arrête.

### 4. Dossier et permissions

Sur le NAS :

```sh
sudo mkdir -p /volume1/docker/life-tracker/db/init /volume1/docker/life-tracker/secrets
```

Depuis le Mac, à la racine du dépôt (`-O` parce que DSM ne sert pas toujours le SFTP ; `NAS` est son nom SSH) :

```sh
scp -O compose.yaml .env.prod.example db/init/01-roles.sh NAS:/tmp/
```

Sur le NAS :

```sh
cd /volume1/docker/life-tracker
sudo mv /tmp/compose.yaml .
sudo mv /tmp/.env.prod.example .env
sudo mv /tmp/01-roles.sh db/init/
sudo chown -R root:root .
sudo chmod 700 . secrets
sudo chmod 600 compose.yaml .env
# Lisibles par l'utilisateur postgres du conteneur, sinon la base ne s'initialise pas.
sudo chmod 755 db db/init db/init/01-roles.sh
```

Un dossier partagé DSM peut porter des ACL qui passent outre `chmod` :

```sh
sudo synoacltool -get /volume1/docker/life-tracker
ls -ld /volume1/docker/life-tracker   # un « + » final signale une ACL
```

S'il y en a une, la retirer sur ce dossier avec `sudo synoacltool -del /volume1/docker/life-tracker` (commande de mémoire, à contrôler), puis vérifier depuis un autre compte du NAS, qui doit recevoir « Permission denied » :

```sh
sudo -u AUTRE_COMPTE ls /volume1/docker/life-tracker
sudo -u AUTRE_COMPTE cat /volume1/docker/life-tracker/secrets/backend.env
```

Le volume de la base vit sous le dossier de données de Docker, réservé à root (`sudo docker volume inspect life-tracker-prod-db-data` donne le chemin). Rien ne doit être déplacé vers un dossier partagé.

### 5. Configuration et secrets

Tout se fait en root, pour que les fichiers le restent :

```sh
sudo -i
cd /volume1/docker/life-tracker
```

Dans `.env`, remplacer `OWNER` et `vX.Y.Z` (la dernière release) :

```sh
OWNER_MINUSCULES=...
VERSION=v...
sed -i "s|ghcr.io/OWNER/|ghcr.io/$OWNER_MINUSCULES/|; s|^TRACKER_TAG=.*|TRACKER_TAG=$VERSION|" .env
```

Puis les secrets, en un seul bloc :

```sh
(
umask 077
mkdir -p secrets
pw() { openssl rand -hex 32; }
OWNER=$(pw); APP=$(pw); ADMIN=$(pw)
cat > secrets/db.env <<EOF
POSTGRES_PASSWORD=$(pw)
TRACKER_OWNER_PASSWORD=$OWNER
TRACKER_APP_PASSWORD=$APP
TRACKER_ADMIN_PASSWORD=$ADMIN
EOF
echo "MIGRATE_DATABASE_URL=postgres://tracker_owner:$OWNER@db:5432/tracker" > secrets/migrate.env
echo "DATABASE_URL=postgres://tracker_app:$APP@db:5432/tracker" > secrets/backend.env
echo "ADMIN_DATABASE_URL=postgres://tracker_admin:$ADMIN@db:5432/tracker" > secrets/admin.env
)
ls -l secrets   # quatre fichiers en -rw-------
exit
```

Si `openssl` manque : `pw() { head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n'; }`.

Ces mots de passe ne servent à créer les rôles qu'à la création du volume : les changer dans les fichiers ensuite ne change pas la base. Il faudrait aussi un `ALTER ROLE`, depuis `docker compose exec db psql -U postgres`.

### 6. Premier démarrage

Se connecter au registre. La saisie attendue est le token, sans écho :

```sh
read -rs CR_PAT; printf '%s' "$CR_PAT" | sudo docker login ghcr.io -u OWNER --password-stdin; unset CR_PAT
```

Puis, en root, car le dossier en `700` est fermé à mon compte (de même pour toutes les commandes `docker compose` de la suite) :

```sh
sudo -i
cd /volume1/docker/life-tracker
sudo docker compose config -q      # sinon : sudo docker-compose config -q, et de même partout
sudo docker compose pull
sudo docker compose up -d
sudo docker compose ps -a
sudo docker compose logs migrate   # « migrations à jour »
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:18080/health    # 200
```

**Container Manager.** Créer le projet `life-tracker` sur ce même dossier, en choisissant le `compose.yaml` existant, permet de le suivre dans l'interface. La ligne de commande reste la référence pour `admin`. Si l'interface refuse le fichier ou ne voit pas le dossier à cause du `700`, tout se fait en ligne de commande : le résultat est le même.

**Si `migrate` échoue avec `role "tracker_owner" does not exist`.** L'initialisation de la base a échoué et `restart` a relancé PostgreSQL sur un volume à moitié prêt : `docker compose logs db` montre `Skipping initialization` après une erreur sur `01-roles.sh`. Corriger les permissions de l'étape 4. Puis, **uniquement à ce stade, tant qu'il n'y a aucune donnée** : `sudo docker compose down -v && sudo docker compose up -d`.

### 7. HTTPS par Tailscale

Le cookie de session est toujours `Secure` : sans HTTPS, pas de connexion sur l'iPhone. `DEV_INSECURE_COOKIE` n'est pas une solution, et `compose.yaml` le force à `false`.

Dans la console Tailscale, onglet *DNS* : activer MagicDNS et *HTTPS Certificates*. Puis, sur le NAS :

```sh
sudo tailscale serve --bg --https=443 http://127.0.0.1:18080
sudo tailscale serve status
```

L'adresse est `https://<nas>.<tailnet>.ts.net`. Le nom apparaît dans les journaux publics de certificats. `sudo tailscale serve --https=443 off` retire la règle.

**Prise en charge sur Synology : essayée le 31 oct. 2026, `serve` fonctionne sur mon NAS.** La documentation de Tailscale n'en parle pas, ni pour, ni contre. Si le port 443 entre en conflit avec DSM : `--https=8443`, et l'adresse porte ce port. Si `serve` ne marche pas : un conteneur Tailscale dans le projet, à décider alors, sans jamais retirer `Secure`.

**Depuis l'étape 9 (3 nov. 2026), la web app est sur 8443** : `sudo tailscale serve --bg --https=8443 http://127.0.0.1:18080`, à l'adresse `https://<nas>.<tailnet>.ts.net:8443`. Le port 443 sert le MCP par Funnel (voir « Mise à jour vers l'étape 9 »).

Après un redémarrage du NAS, revérifier `sudo tailscale serve status`.

### 8. Compte web

```sh
sudo docker compose run --rm admin set-password moi
```

12 caractères au moins, saisis au clavier. Le mot de passe va dans le gestionnaire de mots de passe.

### 9. Source iOS

```sh
sudo docker compose run --rm admin create-source ios
sudo docker compose run --rm admin grant-kind ios coffee.cup
sudo docker compose run --rm admin grant-kind ios record.cancel
```

Le token ne s'affiche qu'une fois : le garder dans le gestionnaire de mots de passe. Pour le remplacer : `admin revoke-source ios`, puis une source à un nouveau nom (`ios-2`), car un nom ne se réutilise pas.

### 10. Raccourci iOS

Tailscale doit être actif sur l'iPhone. Noms d'actions de l'iOS en français, à ajuster si besoin ; ces étapes n'ont pas été jouées sur un téléphone. Je n'ai trouvé aucune action UUID native : si l'app en propose une, la préférer aux points 1 et 2.

1. Six actions **Nombre aléatoire**, à renommer A à F : A de 10000000 à 99999999 ; B de 1000 à 9999 ; C et D de 100 à 999 ; E et F de 100000 à 999999.
2. **Texte**, nommé `Clé` : `A-B-4C-8D-EF`, chaque lettre étant la variable correspondante, sans espace. Le résultat a la forme d'un UUID version 4, avec environ 98 bits aléatoires.
3. **Date actuelle**, puis **Mettre en forme la date**, format *Personnalisé* : `yyyy-MM-dd'T'HH:mm:ssxxx`. Le résultat attendu, `2026-10-31T14:35:21+01:00`, garde le décalage de Paris.
4. **Texte**, nommé `Corps`, où `Clé` et `Date` sont les variables :
   ```json
   {"records":[{"key":"Clé","kind":"coffee.cup","schema_version":1,"occurred_at":"Date","payload":{}}]}
   ```
5. **Obtenir le contenu de l'URL** : `https://<nas>.<tailnet>.ts.net:8443/v1/records` (le port 8443 depuis l'étape 9 ; sur 443 répond le MCP), méthode **POST**, avec trois en-têtes :
   - `Authorization` : `Bearer <token de l'étape 9>`
   - `Content-Type` : `application/json`
   - `X-Client-Version` : `ios-shortcut-1`

   Corps de la requête : **Fichier**, avec la variable `Corps`.
6. **Si** le résultat **contient** `accepted` : **Afficher une notification** « Café compté ». **Sinon** : afficher le résultat.

Une nouvelle clé naît à chaque lancement. Si le réseau coupe après l'écriture, relancer compte un second café : il s'annule dans la web app. Le token vit dans le raccourci : ne jamais partager ce raccourci.

## Mise à jour vers l'étape 5 (musique)

À faire une fois, sur le NAS déjà en service, après un workflow vert. **L'ordre compte :** dès que le nouveau `compose.yaml` est en place, `docker compose` exige `secrets/collector.env`. Je ne lance rien de tout cela depuis le dépôt.

1. Une clé d'API : [last.fm/api/account/create](https://www.last.fm/api/account/create). Elle va dans le gestionnaire de mots de passe, jamais ailleurs. L'API lit l'historique sans authentification : s'il est masqué dans les réglages de confidentialité de Last.fm, il faudra le rendre visible (de mémoire, à contrôler).

2. Depuis le Mac, à la racine du dépôt :

   ```sh
   scp -O compose.yaml NAS:/tmp/
   ```

3. Sur le NAS, en root dans le dossier en `700`, **avec l'ancien `compose.yaml` encore en place** :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   TOKEN=$(sudo docker compose run --rm -T admin create-source lastfm --silence-threshold "6 hours")
   [ "${#TOKEN}" = 64 ] && echo "token ok" || echo "ÉCHEC : ne pas continuer"
   sudo docker compose run --rm -T admin grant-kind lastfm lastfm.scrobble </dev/null
   sudo docker compose run --rm -T admin grant-kind lastfm lastfm.track_info </dev/null
   read -rs LASTFM_API_KEY   # la clé, sans écho
   read -r LASTFM_USER       # le nom d'utilisateur Last.fm
   (umask 077; printf 'LASTFM_API_KEY=%s\nLASTFM_USER=%s\nCOLLECTOR_TOKEN=%s\n' "$LASTFM_API_KEY" "$LASTFM_USER" "$TOKEN" > secrets/collector.env)
   unset TOKEN LASTFM_API_KEY LASTFM_USER
   ls -l secrets/collector.env   # -rw-------
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select sk.kind from core.source_kinds sk join core.sources s on s.id = sk.source_id where s.name = 'lastfm' and revoked_at is null"   # les deux types
   ```

   `</dev/null` : sans lui, `run -T` peut avaler la ligne collée suivante, ce qui a fait sauter un droit au premier déploiement (cause probable). Si un droit manque, le collecteur l'indique par `kind_not_declared` dans ses logs et reprend seul une fois le droit accordé. Le token n'est gardé que là : s'il se perd, voir l'étape 9 pour le remplacer (`lastfm-2`, reconnu aussi par la page du jour). `create-source` échoue si le nom existe déjà.

4. Toujours en root, le nouveau fichier et la nouvelle version :

   ```sh
   mv /tmp/compose.yaml .
   chown root:root compose.yaml
   chmod 600 compose.yaml
   sed -i 's/^TRACKER_TAG=.*/TRACKER_TAG=NOUVEAU_SHA/' .env
   sudo docker compose config -q
   sudo docker compose pull
   sudo docker compose up -d
   sudo docker compose ps -a
   sudo docker compose logs collector | tail   # « collect pass completed »
   exit
   ```

   Pas de migration ni de `rebuild` : aucun ancien enregistrement n'a de nouvel interprète à rejouer. Le premier passage relit tout l'historique Last.fm, page par page puis fiche par fiche, à quatre requêtes par seconde au plus : compter quelques minutes par millier de titres.

5. Vérifier le critère **« une vraie journée correspond à Last.fm »** : choisir un jour passé et complet, comparer sur la page du jour le nombre d'écoutes et la liste des titres avec last.fm/user/MOI/library?from=AAAA-MM-JJ&to=AAAA-MM-JJ. Last.fm découpe selon le fuseau de son compte : le régler sur Paris, ou s'attendre à un écart autour de minuit. Le temps estimé n'a pas d'équivalent à comparer. Regarder aussi si des erreurs d'interprétation existent, ce qui dirait que les vraies réponses diffèrent des fixtures :

   ```sh
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select count(*) from derived.interpretation_errors"   # 0
   ```

   Seulement alors, marquer l'étape 5 « Fait » dans `docs/poc.md`.

6. Vérifier l'isolement :

   ```sh
   sudo docker compose exec -T collector sh -c 'getent hosts db || echo "base inconnue"'       # base inconnue
   sudo docker compose exec -T backend sh -c 'wget -q -T5 -O /dev/null https://www.last.fm && echo SORTIE || echo "pas de sortie"'   # pas de sortie
   sudo docker inspect "$(sudo docker compose ps -q backend)" | grep -c LASTFM   # 0
   sudo docker inspect "$(sudo docker compose ps -q web)" | grep -c LASTFM       # 0
   ```

   Le collecteur doit joindre Last.fm : si ses logs montrent `"reason":"connect"` en continu, voir d'abord le DNS et le pare-feu de DSM, sans donner de sortie au backend.

**Retour arrière.** Pas de migration entre les deux versions : l'ancien `compose.yaml` et l'ancien `TRACKER_TAG` suffisent ; `lastfm` reste dans le registre, et sa santé n'est plus lue. Le journal garde ce qui est entré. Pour couper seulement la collecte : `sudo docker compose stop collector`, et la page du jour signalera le silence après 6 h.

## Mise à jour vers l'étape 6 (découvertes)

À faire une fois, sur le NAS déjà en service, après un workflow vert. `compose.yaml` ne change pas, aucun secret non plus. **Pas de nouvelle migration** : `_sqlx_migrations` reste à 2 lignes.

**Pas de `rebuild` obligatoire.** Le seul nouvel interprète, `music.mark_known`, n'a aucun ancien enregistrement à relire ; s'il y en avait, le worker les reprendrait seul par son curseur. Les écoutes ne sont pas réinterprétées. La reconstruction ci-dessous sert seulement de contrôle, et n'interrompt ni les lectures ni l'ingestion.

**Pas de droit à accorder.** La source `webapp` a `music.mark_known` et `record.cancel` depuis la migration initiale : `grant-kind` n'a rien à faire ici. La vérification de l'étape 2 le confirme ; si une ligne manquait, le bouton afficherait « refusé par le serveur », et l'accorder se ferait par `grant-kind webapp <type> </dev/null`, sur une ligne seule.

1. Sur le NAS, en root dans le dossier en `700`. `</dev/null` : sans lui, `run -T` peut avaler la ligne collée suivante.

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   sed -i 's/^TRACKER_TAG=.*/TRACKER_TAG=NOUVEAU_SHA/' .env
   sudo docker compose config -q
   sudo docker compose pull
   sudo docker compose up -d
   sudo docker compose ps -a
   sudo docker compose logs migrate | tail -3
   ```

2. Vérifier les droits, le nombre de migrations et l'absence d'erreur :

   ```sh
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select s.name, k.kind from core.source_kinds k join core.sources s on s.id = k.source_id where k.revoked_at is null order by 1, 2" </dev/null
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select (select count(*) from _sqlx_migrations), (select count(*) from derived.interpretation_errors)" </dev/null   # 2|0
   ```

   La première liste doit contenir `webapp|coffee.cup`, `webapp|music.mark_known` et `webapp|record.cancel`.

3. Sur l'iPhone, sur un jour passé qui a des écoutes : toucher « Déjà connu » sur un artiste. Le bouton passe à « Marqué connu », la ligne cesse de dire « découvert », l'album et le titre ne bougent pas. Recharger : il l'est toujours. Toucher de nouveau : « Artiste découvert » revient, ce qui confirme que `record.cancel` passe.

4. Contrôle des enregistrements du geste, puis reconstruction. Le NAS n'a que quelques centaines d'enregistrements ; à titre d'ordre de grandeur, 89 600 ont pris environ 20 min sur le Mac, machine occupée par ailleurs, et le temps sur le NAS n'est pas mesuré.

   ```sh
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select kind, count(*) from core.records where kind in ('music.mark_known', 'record.cancel') group by 1" </dev/null
   sudo docker compose run --rm -T admin rebuild </dev/null
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select count(*) from derived.interpretation_errors" </dev/null   # 0
   exit
   ```

   `rebuild` doit basculer sans aucune différence ; un code de sortie non nul laisse l'ancienne génération en service. La page du jour reste identique.

Seulement alors, marquer l'étape 6 « Fait » dans `docs/poc.md`.

**Retour arrière.** Sans migration : l'ancien `TRACKER_TAG`, `pull` et `up -d` suffisent. Les marquages restent au journal et leur curseur s'arrête ; ils ne sont simplement plus lus. Un `rebuild` de l'ancienne image verrait alors une différence, et demanderait `--allow-differences`. En revenant à la nouvelle image, le worker reprend là où il s'était arrêté.

## Mise à jour vers l'étape 7 (historique Apple)

À faire une fois, sur le NAS déjà en service, après un workflow vert. Joué le 2 nov. 2026 : interprétation finie en moins de 15 minutes sur le NAS, sans mesure plus fine. **Pas de nouvelle migration** : `_sqlx_migrations` reste à 2 lignes. **Pas de `rebuild` obligatoire** : le nouvel interprète n'a rien à relire, et les écoutes existantes ne changent pas. Dans chaque `docker compose run -T`, `</dev/null` évite d'avaler la ligne collée suivante.

**Ce qu'il faut attendre.** Essai de la vraie archive sur le Mac, base limitée à 0,5 CPU : envoi en 5 s, interprétation de 28 295 lignes en 145 s environ. Le Celeron et ses disques ne sont pas ce Mac : compter de 3 à 15 minutes, sans mesure, et lire la progression à l'étape 5. Pendant ce temps, café, Last.fm et lectures continuent : sur le Mac, un scrobble était écrit en 50 ms et interprété en moins d'une seconde.

1. Sur le Mac, préparer un dossier avec **les trois seuls fichiers lus**. `ARCHIVE` est le dossier `Apple Music Activity` de l'export. Ils contiennent IP, villes et identifiants : pas de `/tmp`, qu'un autre compte du NAS pourrait voir.

   ```sh
   mkdir -m 700 ~/apple-import
   cp "$ARCHIVE/Apple Music Play Activity.csv" "$ARCHIVE/Apple Music - Play History Daily Tracks.csv" ~/apple-import/
   unzip -j "$ARCHIVE/Apple Music Library Tracks.json.zip" -d ~/apple-import/
   ls ~/apple-import   # trois fichiers, dont « Apple Music Library Tracks.json »
   ssh NAS 'umask 077; mkdir -p ~/apple-import'
   scp -O ~/apple-import/* NAS:apple-import/
   scp -O compose.yaml NAS:apple-import/
   ```

2. Sur le NAS, en root dans le dossier en `700`. `HOME_NAS` est le dossier de mon compte. La nouvelle image et le nouveau fichier :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   HOME_NAS=/var/services/homes/MON_COMPTE
   mv "$HOME_NAS/apple-import/compose.yaml" .
   chown root:root compose.yaml
   chmod 600 compose.yaml
   sed -i 's/^TRACKER_TAG=.*/TRACKER_TAG=NOUVEAU_SHA/' .env
   sudo docker compose config -q
   sudo docker compose pull
   sudo docker compose up -d
   sudo docker compose ps -a
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select (select count(*) from _sqlx_migrations), (select count(*) from derived.interpretation_errors)" </dev/null   # 2|0
   ```

3. La source `apple` et son seul droit. Le token reste dans un fichier en `600` : l'import le lit sur l'entrée standard, il n'apparaît ni dans `ps` ni dans `docker inspect`. Un futur export devra passer par cette même source pour retomber sur les mêmes clés : ne pas la révoquer, ne pas perdre ce fichier.

   ```sh
   (umask 077; sudo docker compose run --rm -T admin create-source apple </dev/null > secrets/apple-import.token)
   [ "$(wc -c < secrets/apple-import.token)" = 65 ] && echo "token ok" || echo "ÉCHEC : ne pas continuer"
   sudo docker compose run --rm -T admin grant-kind apple apple.play_history_row </dev/null
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select k.kind from core.source_kinds k join core.sources s on s.id = k.source_id where s.name = 'apple' and k.revoked_at is null" </dev/null   # apple.play_history_row, seul
   ```

4. L'archive, lisible par le seul utilisateur du conteneur (`10001`), montée en lecture seule :

   ```sh
   mkdir -p imports/apple
   chmod 700 imports
   mv "$HOME_NAS"/apple-import/*.csv "$HOME_NAS"/apple-import/*.json imports/apple/
   rmdir "$HOME_NAS/apple-import"
   chown -R 10001:10001 imports/apple
   chmod 500 imports/apple
   chmod 400 imports/apple/*
   sudo docker compose run --rm -T --entrypoint sh apple-import -c 'id -u; ls /archive | wc -l; touch /archive/x 2>&1 | head -1; getent hosts db || echo "base inconnue"' </dev/null   # 10001, 3, refus d'écriture, base inconnue
   ```

5. L'import. La sortie ne contient que des nombres, des motifs et les empreintes de l'archive : elle se garde, c'est la note d'empreinte que demande le POC. Une coupure ? Relancer la même commande : ce qui est passé revient en `duplicate`.

   ```sh
   sudo docker compose run --rm -T apple-import < secrets/apple-import.token | tee imports/apple-import-1.txt
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select count(*) filter (where seq > coalesce((select max(last_seq) from derived.cursors c join derived.generations g on g.id = c.generation and g.state = 'active' where c.kind = 'apple.play_history_row'), 0)) from core.records where kind = 'apple.play_history_row'" </dev/null   # à relancer jusqu'à 0
   ```

   Attendu pour cette archive : 60 720 lignes lues, 28 295 `accepted`, 0 `rejected`, 32 425 ignorées (`play_start` 22 959, `lyric_display` 8 717, `no_playback` 456, `no_start_timestamp` 247, `video` 33, `no_song_name` 13).

6. Vérifier le critère **« réimporter ne donne que des `duplicate` »**, l'absence d'erreur, et **qu'aucune colonne hors liste n'est au journal** :

   ```sh
   sudo docker compose run --rm -T apple-import < secrets/apple-import.token | grep -E '^(accepted|duplicate|rejected)'   # 0, 28295, 0
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select k, count(*) from core.records r, jsonb_object_keys(r.payload) k where r.kind = 'apple.play_history_row' group by 1 order by 1" </dev/null
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select count(*) filter (where details->>'artist' is null), count(*) from derived.events e join core.records r using (seq) where r.kind = 'apple.play_history_row' and e.generation = (select id from derived.generations where state = 'active')" </dev/null   # 99|20682
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select count(*) from derived.interpretation_errors" </dev/null   # 0
   ```

   La deuxième commande doit lister exactement ces 16 champs, chacun 28 295 fois : `album`, `archive_sha256`, `artist`, `artist_source`, `end_position_ms`, `end_reason`, `ended_at`, `item_type`, `media_duration_ms`, `media_type`, `offline`, `play_duration_ms`, `song`, `start_position_ms`, `started_at`, `utc_offset_seconds`.

7. Regarder la jonction **avant** de déclarer la priorité, et trancher :

   ```sh
   sudo docker compose run --rm -T admin coverage music.listen </dev/null
   ```

   La sortie donne les dates de chaque source, l'écart entre les deux premières données, et `trou` ou `chevauchement`. Si la première écoute de `lastfm` est à quelques jours de la suivante :

   ```sh
   sudo docker compose run --rm -T admin set-precedence music.listen lastfm </dev/null
   ```

   Si elle est isolée (ancien scrobble), la commande refuse : donner le vrai démarrage, `--from 2026-10-20T00:00:00+02:00` par exemple. **Trou** : les jours non couverts afficheront « Pas de données musicales à cette date » et rien ne les invente. Le signaler pour décider ensuite ; un export Apple plus récent, importé par la même source, le comblera sans doublon.

   Comparer ensuite Apple et Last.fm sur le chevauchement, en nombres seulement. C'est la seule mesure de normalisation que le Mac n'a pas pu faire : la première ligne donne les titres Last.fm aussi vus par Apple sous la même clé, la seconde ceux qui existent chez Apple sous un autre artiste.

   ```sh
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "with g as (select id from derived.generations where state = 'active'), l as (select distinct item_key k from derived.event_items i join derived.events e using (generation, seq, ordinal) join core.sources s on s.id = e.source_id where i.generation = (select id from g) and i.level = 'track' and i.metric = 'music.listen' and s.name = 'lastfm'), a as (select distinct item_key k from derived.event_items i join derived.events e using (generation, seq, ordinal) join core.sources s on s.id = e.source_id where i.generation = (select id from g) and i.level = 'track' and i.metric = 'music.listen' and s.name = 'apple') select count(*), count(*) filter (where k in (select k from a)), count(*) filter (where k not in (select k from a) and split_part(k, chr(9), 2) in (select split_part(k, chr(9), 2) from a)) from l" </dev/null
   ```

   Un troisième nombre élevé dirait que les noms d'artistes diffèrent entre les deux sources : le signaler avant de marquer l'étape, car une découverte Last.fm n'aurait alors pas reculé.

8. Sur l'iPhone : un jour de 2023 montre ses écoutes, avec les découvertes de ce jour-là, et si besoin « dont N sans artiste connu » ; un artiste déjà écouté en 2023 n'est plus découvert en octobre 2026 ; une journée récente correspond toujours à Last.fm, sans double compte. Critères **« le chevauchement reste au journal »** et **« les vues l'ignorent »** :

   ```sh
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select (select count(*) from core.records r join core.source_precedence p on p.valid_during @> r.occurred_at where r.kind = 'apple.play_history_row'), (select count(*) from derived.live_events e join core.sources s on s.id = e.source_id join core.source_precedence p on p.valid_during @> e.occurred_at where s.name = 'apple' and e.metric = 'music.listen')" </dev/null   # N|0
   ```

9. Le contrôle à 0 différence, puis le ménage. Sur le Mac, 130 s à 0,5 CPU ; la page reste servie. **Supprimer l'archive du NAS**, recommandé : elle contient IP, villes et identifiants, et reste retéléchargeable. `imports/apple-import-1.txt` ne contient que des nombres et des empreintes : le garder. Retirer le droit : il se redonne au prochain export.

   ```sh
   sudo docker compose run --rm -T admin rebuild </dev/null
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select count(*) from derived.interpretation_errors" </dev/null   # 0
   rm -rf imports/apple
   sudo docker compose run --rm -T admin revoke-kind apple apple.play_history_row </dev/null
   exit
   ```

   Sur le Mac : `rm -rf ~/apple-import`.

Seulement alors, marquer l'étape 7 « Fait » dans `docs/poc.md`.

**Retour arrière.** Sans migration : l'ancien `compose.yaml`, l'ancien `TRACKER_TAG`, `pull`, `up -d`. Le journal garde les lignes Apple, qui ne sont plus interprétées. Pour annuler seulement la priorité, relancer `set-precedence` avec une autre date ; retirer la ligne demande `psql` : `delete from core.source_precedence where metric = 'music.listen'`, sans rejeu. Effacer les lignes Apple elles-mêmes demande une purge, à ne décider que si elles n'auraient jamais dû entrer.

## Mise à jour vers l'étape 8 (calendrier et complétude)

À faire une fois, sur le NAS déjà en service, après un workflow vert. **Pas de nouvelle migration** : `_sqlx_migrations` reste à 2 lignes. **Pas de `rebuild`** : aucun interprète ne change, la vue du mois se calcule à la lecture. **Pas de secret ni de droit nouveau.** Dans chaque `docker compose run -T` ou `exec -T`, `</dev/null` évite d'avaler la ligne collée suivante.

**`compose.yaml` ne change pas** depuis l'étape 7. Le vérifier plutôt que le supposer : sur le Mac, `shasum -a 256 compose.yaml` à la racine du dépôt doit donner `514ac3a37119cfac3779bf0356b6292a40559524287cf3229bf7e4a634bc5865`, et la même empreinte sur le NAS (étape 1). Si elle diffère sur le NAS, le remplacer comme à l'étape 7 (`scp -O compose.yaml NAS:/tmp/`, puis `mv`, `chown root:root`, `chmod 600`) avant de continuer, et revérifier l'empreinte.

1. Sur le NAS, en root dans le dossier en `700` :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   sha256sum compose.yaml   # 514ac3a3…bc5865, la même que sur le Mac
   sed -i 's/^TRACKER_TAG=.*/TRACKER_TAG=NOUVEAU_SHA/' .env
   sudo docker compose config -q
   sudo docker compose pull
   sudo docker compose up -d
   sudo docker compose ps -a
   sudo docker compose logs migrate | tail -3
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select (select count(*) from _sqlx_migrations), (select count(*) from derived.interpretation_errors)" </dev/null   # 2|0
   ```

2. La nouvelle route exige la session, comme les autres. Sans cookie, par le même nginx que l'iPhone :

   ```sh
   sudo docker compose exec -T web wget -S -q -O /dev/null http://127.0.0.1:8080/v1/months/2026-10 </dev/null 2>&1 | grep -E 'HTTP/|Cache-Control'   # 401 et no-store
   exit
   ```

3. Sur l'iPhone, page du jour : « ‹ Novembre » en haut à gauche ouvre le mois. Attendu avec les vraies données :
   - novembre et octobre 2026 : un chiffre d'écoutes (rose) chaque jour, un chiffre de cafés (brun) à partir du 31 octobre seulement ; avant, la ligne des cafés est vide, sans 0 ni marque ;
   - « Aucun enregistrement en erreur » sous les chevrons, et aucune marque si tout va bien ;
   - en reculant mois par mois : des écoutes de l'historique Apple jusqu'en janvier 2022, des 0 gris les jours sans écoute, et rien avant la première écoute (« Sans chiffre : pas de données ») ; ni trou ni double compte autour du 12 octobre 2026 ;
   - toucher un jour ouvre sa page, et le « ‹ » suivi du nom du mois y ramène. Regarder en clair et en sombre.

4. Vérifier le critère **« une source muette marque les journées non couvertes »**, sans rien casser : couper le collecteur, attendre le seuil, regarder, relancer.

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   sudo docker compose stop collector
   exit
   ```

   Plus de 6 h après le dernier passage réussi, le mois montre « Last.fm ne répond plus » et un anneau sur aujourd'hui (et sur la veille si le dernier contact y était) ; le libellé d'un jour dit « en attente de Last.fm ». Les jours d'avant n'en ont pas. Puis :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   sudo docker compose start collector
   sudo docker compose logs collector | tail -3   # « collect pass completed »
   exit
   ```

   Au passage suivant, recharger : plus aucun anneau, et les écoutes de la coupure arrivent d'elles-mêmes (relecture des 48 dernières heures). Pour ne pas attendre 6 h, ce contrôle peut se faire sur le Mac : il est couvert par `backend/tests/calendar.rs` et `web/e2e/calendar.spec.ts`.

5. Le critère **« un enregistrement en erreur marque sa journée »** n'est pas à provoquer sur la production : le journal est à vie, et une erreur fabriquée y resterait. Il est couvert par les tests ; sur le NAS, seulement vérifier que le compteur suit la base :

   ```sh
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select count(*) from derived.interpretation_errors e join derived.generations g on g.id = e.generation and g.state = 'active'" </dev/null   # le nombre affiché par le mois
   ```

6. Le critère **« les journées passées ne sont jamais mises en cache définitif »** : noter un café à une heure passée sur un jour d'il y a quelques jours, depuis sa page, puis revenir au mois : la case le compte aussitôt. L'annuler ensuite. Aucune réponse de `/v1/` n'est gardée (`no-store`, étape 2), et l'app n'a pas de service worker.

Seulement alors, marquer l'étape 8 « Fait » dans `docs/poc.md`.

**Retour arrière.** Sans migration : l'ancien `TRACKER_TAG`, `pull` et `up -d`. Rien n'a été écrit ni au journal ni dans `derived` ; la page du jour perd seulement le lien vers le mois.

## Mise à jour vers l'étape 9 (serveur MCP)

À faire une fois, sur le NAS déjà en service, après un workflow vert (qui construit maintenant trois images : `backend`, `web`, `mcp`). Joué les 2 et 3 nov. 2026, avec le tag `8e5cf06035f28d7d821d38cc91c2117cdee27c4c` : Funnel fonctionne sur le paquet Synology, mais Claude.ai n'atteint pas une URL sur le port 8443, d'où l'exposition sur 443 décrite ici, et deux corrections du MCP (`ba64cf3`, `8e5cf06`, voir le principe et le point 8). Préparé la veille sur le Mac : tests, images `linux/amd64` construites, et un essai du compose avec l'image du MCP. **Une nouvelle migration** : `_sqlx_migrations` passe à 3 lignes (`core.service_tokens`), donc pas de retour à l'ancienne image (voir « Retour arrière » plus bas). **Pas de `rebuild`** : aucun interprète ne change. Dans chaque `docker compose run -T` ou `exec -T`, `</dev/null` évite d'avaler la ligne collée suivante.

**Le principe.** Le MCP est le seul service sur Internet, par Tailscale Funnel sur le port **443** ; la web app passe en `tailscale serve` privé sur **8443**. Prévu d'abord à l'inverse (Funnel sur 8443) : essayé le 2 nov. 2026, Claude.ai répond « Couldn't reach this address » à une URL de connecteur avec le port 8443, sans qu'aucune requête n'atteigne le MCP, alors que l'iPhone en 4G obtient bien le JSON. Sur Internet ne répondent que `/mcp` (jeton OAuth exigé), `/token` et `/.well-known/*`. La page de connexion (`/authorize`) refuse toute requête arrivée par Funnel : on se connecte depuis un appareil du tailnet, Tailscale actif. **Le paquet Tailscale de Synology ne pose pas l'en-tête `Tailscale-Funnel-Request`** (vérifié : la page s'ouvrait depuis l'iPhone en 4G) ; `/authorize` exige donc aussi l'en-tête `Tailscale-User-Login`, que `tailscaled` pose pour un utilisateur du tailnet (et retire s'il vient du client), et qui manque par Funnel. Même adresse dans les deux cas : depuis le tailnet, MagicDNS résout `<nas>.<tailnet>.ts.net` vers l'adresse 100.x du NAS, et la requête arrive en direct, sans les relais de Funnel (vérifié le 3 nov. 2026 : le Mac obtient la page de consentement, l'iPhone en 4G sans Tailscale « Connexion depuis le tailnet »). Le secret de connexion est propre au MCP, différent du mot de passe web, et ne vit que dans le gestionnaire de mots de passe ; le NAS n'en garde que le haché Argon2id.

**À ne jamais faire : `tailscale funnel` sur le port 8443**, où est la web app : elle deviendrait publique. **Ni remettre la web app sur 443** tant que Funnel y sert le MCP.

0. **Points incertains, levés les 2 et 3 nov. 2026.**

   a. **Une URL de connecteur avec le port 8443** : refusée en pratique, Claude.ai ne l'atteint pas. D'où le port 443.

   b. **Le serveur du SDK v2 face à Claude.ai** : le SDK sert l'ère 2026-07-28 et, par défaut, les clients de l'ère 2025 en mode sans état. Vérifié au point 9 : Claude.ai se connecte et appelle les outils.

   c. **Funnel sur le paquet Tailscale de Synology** : fonctionne, mais sans l'en-tête `Tailscale-Funnel-Request` (voir le principe). Version 1.52 au moins :

      ```sh
      sudo tailscale version
      sudo tailscale funnel status
      ```

1. Sur le Mac, à la racine du dépôt :

   ```sh
   shasum -a 256 compose.yaml   # 55ddcc01bcfeeb96d75a482354735f85a5eef4f4de349fc22c7c054668fadb14
   scp -O compose.yaml NAS:/tmp/
   ```

2. Sur le NAS, en root dans le dossier en `700`. Le nom du NAS dans le tailnet (celui de l'adresse de la web app), puis `.env`, un `secrets/mcp.env` vide pour que Compose accepte le fichier, le nouveau `compose.yaml` et la nouvelle version :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   tailscale serve status   # l'adresse https://<nas>.<tailnet>.ts.net de la web app
   NAS_HOST=nas.TAILNET.ts.net   # à remplacer, sans https:// ni port
   printf 'MCP_PUBLIC_URL=https://%s\nMCP_HTTP_PORT=18443\n' "$NAS_HOST" >> .env
   grep -E '^MCP_' .env
   (umask 077; : > secrets/mcp.env)
   mv /tmp/compose.yaml .
   chown root:root compose.yaml
   chmod 600 compose.yaml
   sha256sum compose.yaml   # 55ddcc01…fadb14, la même que sur le Mac
   sed -i 's/^TRACKER_TAG=.*/TRACKER_TAG=NOUVEAU_SHA/' .env
   sudo docker compose config -q
   sudo docker compose pull
   sudo docker compose up -d db migrate backend web collector
   sudo docker compose ps -a
   sudo docker compose logs migrate | tail -3
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select (select count(*) from _sqlx_migrations), (select count(*) from derived.interpretation_errors)" </dev/null   # 3|0
   ```

   Le MCP n'est pas démarré : sans secrets, il s'arrêterait aussitôt.

3. Le jeton de service : lecture seule, niveau pseudonymes, refusé sur toute écriture. Il ne s'affiche qu'une fois et ne va que dans `secrets/mcp.env` ; le perdre se règle par `revoke-service-token mcp` puis un nouveau nom (`mcp-2`).

   ```sh
   TOKEN=$(sudo docker compose run --rm -T admin create-service-token mcp </dev/null)
   [ "${#TOKEN}" = 64 ] && echo "jeton ok" || echo "ÉCHEC : ne pas continuer"
   ```

4. Le secret de connexion du MCP : 16 caractères au moins, **différent du mot de passe web**, créé dans le gestionnaire de mots de passe (une phrase de passe aléatoire). Il se lit sans écho ; l'image le hache sans réseau (`--network none`) et ne rend que le haché, déjà encodé pour le fichier d'environnement :

   ```sh
   read -rs MCP_SECRET   # le secret, sans écho
   IMAGE="$(grep '^TRACKER_IMAGE_PREFIX=' .env | cut -d= -f2)/mcp:$(grep '^TRACKER_TAG=' .env | cut -d= -f2)"
   HASH_LINE=$(printf '%s\n' "$MCP_SECRET" | sudo docker run --rm -i --network none "$IMAGE" hash-password)
   unset MCP_SECRET
   case "$HASH_LINE" in MCP_LOGIN_HASH=*) echo "haché ok" ;; *) echo "ÉCHEC : ne pas continuer" ;; esac
   (umask 077; printf 'MCP_SERVICE_TOKEN=%s\n%s\n' "$TOKEN" "$HASH_LINE" > secrets/mcp.env)
   unset TOKEN HASH_LINE
   ls -l secrets/mcp.env   # -rw-------
   ```

   Changer le secret plus tard : refaire ce point (avec le jeton déjà dans le fichier : `TOKEN=$(grep '^MCP_SERVICE_TOKEN=' secrets/mcp.env | cut -d= -f2)`), puis `sudo docker compose up -d mcp`. Une autorisation déjà donnée à Claude reste valable : pour la couper aussi, voir « Couper ».

5. Démarrer le MCP et le vérifier depuis le NAS, avant toute exposition :

   ```sh
   sudo docker compose up -d
   sudo docker compose ps mcp   # running (healthy) au bout de 30 s environ
   H="Host: $NAS_HOST"
   curl -s -H "$H" http://127.0.0.1:18443/.well-known/oauth-protected-resource/mcp; echo   # "resource":"https://<nas>….ts.net/mcp"
   curl -s -o /dev/null -w '%{http_code}\n' -X POST -H "$H" http://127.0.0.1:18443/mcp   # 401
   curl -s -D - -o /dev/null -X POST -H "$H" http://127.0.0.1:18443/mcp | grep -i www-authenticate   # Bearer … resource_metadata="…"
   curl -s -o /dev/null -w '%{http_code}\n' -H "$H" -H 'Tailscale-Funnel-Request: ?1' http://127.0.0.1:18443/authorize   # 403
   curl -s -o /dev/null -w '%{http_code}\n' -H "$H" http://127.0.0.1:18443/authorize   # 403 : sans Tailscale-User-Login, comme par Funnel
   curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:18443/.well-known/oauth-authorization-server   # 403 : autre nom d'hôte
   ```

   L'isolement, et le jeton de service vu du backend (il lit, il n'écrit pas ; le jeton reste dans le conteneur, jamais sur la ligne de commande) :

   ```sh
   sudo docker compose exec -T mcp sh -c 'getent hosts db || echo "base inconnue"' </dev/null   # base inconnue
   sudo docker compose exec -T mcp sh -c 'wget -q -T5 -O /dev/null https://www.last.fm && echo SORTIE || echo "pas de sortie"' </dev/null   # pas de sortie
   sudo docker inspect "$(sudo docker compose ps -q mcp)" | grep -c -e DATABASE_URL -e tracker_   # 0
   sudo docker compose exec -T mcp sh -c 'wget -q -O - --header "Authorization: Bearer $MCP_SERVICE_TOKEN" http://backend:8080/v1/inventory | head -c 120; echo' </dev/null   # {"timezone":"Europe/Paris",…
   sudo docker compose exec -T mcp sh -c 'wget -q -O /dev/null --header "Authorization: Bearer $MCP_SERVICE_TOKEN" --post-data "{\"records\":[]}" http://backend:8080/v1/records && echo ÉCRIT || echo refusé' </dev/null   # refusé (401)
   ```

   Si `pas de sortie` ne s'affiche pas, l'option `enable_ip_masquerade` n'agit pas sur ce Docker : le noter, sans élargir quoi que ce soit, et en décider avant le point 6.

6. **Activer Funnel.** Console Tailscale, *Access controls* : le fichier de politique doit donner l'attribut `funnel` au NAS. Par défaut, un bloc l'accorde à `autogroup:member` ; s'il manque, l'ajouter (et *HTTPS Certificates* reste activé depuis l'étape 4) :

   ```json
   "nodeAttrs": [
     { "target": ["autogroup:member"], "attr": ["funnel"] }
   ]
   ```

   Puis, sur le NAS, déplacer d'abord la web app sur 8443, et seulement ensuite ouvrir 443 par Funnel, pour que 443 ne serve jamais la web app en public :

   ```sh
   sudo tailscale serve --https=443 off
   sudo tailscale serve --bg --https=8443 http://127.0.0.1:18080
   sudo tailscale funnel --bg --https=443 http://127.0.0.1:18443
   sudo tailscale funnel status   # https://<nas>… (Funnel on) → 127.0.0.1:18443 ; https://<nas>…:8443 (tailnet only) → 127.0.0.1:18080
   exit
   ```

   Si `funnel status` montre 8443 en « Funnel on » : `sudo tailscale funnel --https=8443 off` aussitôt, puis `sudo tailscale serve --bg --https=8443 http://127.0.0.1:18080`. La web app s'ouvre désormais à `https://<nas>.<tailnet>.ts.net:8443`. La première publication d'un nom par Funnel peut prendre quelques minutes (DNS public).

7. **Depuis Internet** : un appareil hors du tailnet, par exemple l'iPhone en 4G avec Tailscale coupé, ou un service de `curl` en ligne. `NAS_HOST` comme au point 2 :

   ```sh
   curl -s https://$NAS_HOST/.well-known/oauth-protected-resource/mcp; echo                  # le JSON
   curl -s https://$NAS_HOST/.well-known/oauth-authorization-server; echo                     # issuer, S256, sans registration_endpoint
   curl -s -o /dev/null -w '%{http_code}\n' -X POST https://$NAS_HOST/mcp                      # 401
   curl -s -o /dev/null -w '%{http_code}\n' "https://$NAS_HOST/authorize"                      # 403 : connexion refusée par Funnel
   curl -s -o /dev/null -w '%{http_code}\n' https://$NAS_HOST/v1/inventory                     # 404
   curl -s -m 10 -o /dev/null -w '%{http_code}\n' https://$NAS_HOST:8443/ || echo "injoignable"   # injoignable : la web app reste privée
   ```

   **Depuis le tailnet** (le Mac, Tailscale actif, MagicDNS activé) : `curl -s -o /dev/null -w '%{http_code}\n' "https://$NAS_HOST/authorize"` doit donner **400** (« Client inconnu ») et non 403 : la requête arrive en direct, avec `Tailscale-User-Login`. Si c'est 403, le Mac passe par Funnel (MagicDNS désactivé dans Tailscale, ou résolveur forcé) : régler le DNS du Mac avant le point 8. Et la web app répond sur `https://$NAS_HOST:8443`.

8. **Le connecteur dans Claude**, sur claude.ai dans le navigateur d'un appareil du tailnet (le Mac) : *Réglages → Connecteurs → Ajouter un connecteur personnalisé*. Nom `Life tracker`, URL `https://<nas>.<tailnet>.ts.net/mcp`, sans identifiant ni secret OAuth dans les réglages avancés. *Se connecter* ouvre la page du NAS « Autoriser Claude à lire le Life tracker ? », qui montre l'hôte de retour (`claude.ai`) : saisir le secret du point 4, *Autoriser*. Retour sur Claude, connecteur relié. Le connecteur vaut ensuite aussi dans l'app iPhone, sans nouvelle connexion : ce sont les serveurs de Claude qui appellent le MCP.

   Cinq échecs de secret en quinze minutes bloquent toute connexion pendant ce temps, même avec le bon secret (« Trop d'essais »). Si la page dit « Connexion depuis le tailnet », le navigateur est passé par Funnel : voir le point 7. Si *Autoriser* échoue avec `Invalid Origin header: null`, l'image est antérieure à `8e5cf06` : une politique `no-referrer` fait envoyer `Origin: null` au formulaire par Chromium, que le contrôle d'origine du SDK refuse ; la page est maintenant en `same-origin` (en-tête et balise `<meta name="referrer">`).

9. **Les questions de démo**, dans une nouvelle conversation, le connecteur activé (adapter les noms à de vraies écoutes) :

   - « Combien de fois ai-je écouté *Titre* ces 30 derniers jours ? »
   - « Combien de minutes de musique ai-je écoutées cette semaine ? »
   - « Quand ai-je découvert *Artiste* ? »
   - « Depuis quand n'ai-je pas écouté l'album *Album* ? »
   - « À quelle heure je prends mon premier café en moyenne ? »

   Claude doit appeler `inventory`, puis `search`, `occurrences`, `cumulative_duration`, `discovery`, `last_time`. Comparer avec la web app : le nombre d'écoutes d'un jour, les minutes d'un jour, l'heure du premier café. Puis les journaux, qui ne doivent contenir ni jeton, ni code, ni adresse :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   sudo docker compose logs mcp | tail -20   # POST /mcp 200, « tokens issued » ; jamais de jeton ni d'IP
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select name, revoked_at is null from core.service_tokens" </dev/null   # mcp|t
   exit
   ```

   Seulement alors, marquer l'étape 9 « Fait » dans `docs/poc.md`, avec les réponses obtenues. Fait le 3 nov. 2026 pour les quatre questions sur la musique et les journaux ; la question du café attend de vrais cafés (voir `docs/poc.md`).

**Couper.** Fermer Internet sans rien toucher d'autre : `sudo tailscale funnel --https=443 off`. Plus rien ne répond alors sur 443 ; la web app reste servie sur 8443 (vérifier `sudo tailscale serve status`). Ne pas la remettre sur 443 pour autant : rouvrir Funnel la rendrait publique. Couper aussi l'accès de Claude : retirer le connecteur dans Claude, puis effacer les jetons OAuth et arrêter le serveur, et au besoin révoquer le jeton de service :

```sh
sudo -i
cd /volume1/docker/life-tracker
sudo docker compose rm -sf mcp
sudo docker volume rm life-tracker-prod-mcp-state
sudo docker compose run --rm -T admin revoke-service-token mcp </dev/null
exit
```

Relancer ensuite demande un nouveau jeton de service (`mcp-2`) et `sudo docker compose up -d mcp`.

**Retour arrière.** La migration de l'étape 9 ne se défait pas : l'ancienne image refuserait la base (`migrate` échoue, le backend ne démarre pas). Revenir en arrière, c'est donc couper (ci-dessus) et garder la nouvelle image : le backend ne change de comportement que pour les nouvelles routes de lecture et le jeton de service, et la web app est identique. La table `core.service_tokens` reste, vide ou révoquée, sans effet.

## Mise à jour : Garmin Connect (après le POC)

À faire une fois, sur le NAS déjà en service. La PR part en production à la fusion, migration comprise (`20261104120000_garmin.sql`, par le service `migrate`) : le connecteur Garmin ne démarre que lorsque sa clé est dans `secrets/collector.env`, donc rien ne change pour Last.fm d'ici là. Je ne lance rien de tout cela depuis le dépôt, et je tape moi-même chaque secret.

1. **Avant la fusion**, recopier `compose.yaml` comme à l'étape 4 de « Mise à jour vers l'étape 5 » : son contrôle de santé vérifie l'état de chaque connecteur configuré (`state.json` pour Last.fm, `garmin-state.json` pour Garmin, réécrits à chaque passage réussi).

2. **Une clé neuve, pour le NAS seulement.** La clé de l'essai du 4 nov. 2026 est passée par le Mac : dans Garmin Connect (Paramètres, Personal API Key), la révoquer, puis en créer une autre, d'un an au plus. Elle va dans le gestionnaire de mots de passe et dans `secrets/collector.env`, nulle part ailleurs. Garmin prévient par courriel une semaine avant son expiration : la remplacer alors (étape 6).

3. Après le déploiement de la version, sur le NAS, en root dans le dossier en `700` :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   TOKEN=$(sudo docker compose run --rm -T admin create-source garmin --silence-threshold "12 hours" </dev/null)
   [ "${#TOKEN}" = 64 ] && echo "token ok" || echo "ÉCHEC : ne pas continuer"
   for kind in garmin.daily_summary garmin.sleep garmin.activity garmin.activity_track record.cancel; do
     sudo docker compose run --rm -T admin grant-kind garmin "$kind" </dev/null
   done
   read -rs GARMIN_API_KEY   # la clé neuve, sans écho
   (umask 077; printf 'GARMIN_API_KEY=%s\nGARMIN_TOKEN=%s\n' "$GARMIN_API_KEY" "$TOKEN" >> secrets/collector.env)
   unset TOKEN GARMIN_API_KEY
   ls -l secrets/collector.env   # -rw-------
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select sk.kind from core.source_kinds sk join core.sources s on s.id = sk.source_id where s.name = 'garmin' and sk.revoked_at is null"   # les cinq types
   sudo docker compose up -d collector
   ```

   `record.cancel` est nécessaire : une révision de Garmin (synchro tardive de la montre) part avec l'annulation de la version précédente. Sans ce droit, le collecteur écrit `kind_not_declared` dans ses logs et reprend seul une fois le droit accordé. Si la source doit un jour être recréée (`garmin-2`), ajouter `GARMIN_SOURCE=garmin-2` au fichier : c'est le nom que désignent ses annulations.

4. Le premier passage remonte 400 jours : 13 appels pour les pas et calories, 13 pour le sommeil (tranches de 31 jours), la liste des séances, puis le fichier FIT de chaque séance avec GPS, à une seconde d'intervalle. Compter quelques minutes. Ensuite, un passage tous les quarts d'heure relit les 7 derniers jours.

   ```sh
   sudo docker compose logs collector | grep garmin | tail -3   # « garmin pass completed »
   sudo docker compose ps collector                            # healthy après le premier passage
   ```

   « clé Garmin refusée : expirée ou révoquée » dans les logs : la clé est mauvaise ou expirée ; aucun signal de vie ne part, et le mois signale le silence de Garmin après 12 h.

5. Vérifier les critères du chantier Garmin dans [les chantiers](chantiers.md) : un jour passé et complet, comparé à Garmin Connect (pas, calories totales, actives et au repos, sommeil et ses phases, séances, distance, allure, fréquence cardiaque) sur la page du jour et par le MCP (`day_summary`) ; puis le rejeu :

   ```sh
   sudo docker compose run --rm -T admin rebuild </dev/null   # 0 différence
   sudo docker compose run --rm -T admin coverage steps </dev/null
   ```

6. **Remplacer la clé** (chaque année) : en créer une dans Garmin Connect, remplacer la ligne `GARMIN_API_KEY=` de `secrets/collector.env` (en root, éditeur sans sauvegarde, ou `sed -i` avec la clé lue par `read -rs`), puis `sudo docker compose up -d collector`, et révoquer l'ancienne.

**Zone de confidentialité du MCP.** Sans elle, Claude reçoit les détails des séances mais aucune coordonnée ni aucun parcours. Elle se règle dans `secrets/backend.env`, et je la saisis moi-même, jamais dans le dépôt ni dans une conversation :

1. Choisir un centre **décalé de 100 à 300 m du domicile** (un carrefour voisin, par exemple) et un **rayon de 500 à 800 m**, qui couvre le domicile avec de la marge. Pourquoi : la zone coupe chaque parcours là où il y entre ; centrée sur la maison, les points de coupure de dizaines de séances dessineraient un cercle dont le centre serait la maison. Décalée et assez large, elle cache le domicile sans le désigner. Une zone par lieu à protéger (travail, famille), séparées par `;`.
2. Sur le NAS, en root dans le dossier en `700` :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   read -r ZONES   # lat,lon,rayon_m ; plusieurs séparées par « ; » ; en degrés décimaux, rayon en mètres
   (umask 077; printf 'PRIVACY_ZONES=%s\n' "$ZONES" >> secrets/backend.env)
   unset ZONES
   ls -l secrets/backend.env   # -rw-------
   sudo docker compose up -d backend
   sudo docker compose logs backend | grep "zone de confidentialité"   # « zone de confidentialité : 1 zone(s) »
   ```

   « absente » ou « illisible » : le MCP ne reçoit aucune coordonnée, ce qui est sûr ; corriger la ligne et relancer. Les logs ne montrent jamais les valeurs.
3. Vérifier depuis Claude : demander le parcours d'une séance partie de chez moi ; il doit commencer hors de la zone (`trimmed: true`), et son départ être masqué (`start_hidden`).

**Ce qui part chez Anthropic.** Tout ce que lit le MCP (lieux, détails, parcours hors des zones) passe dans la conversation et reste chez Anthropic jusqu'à 30 jours après sa suppression ; l'entraînement sur mes données est désactivé.

**Carte des séances.** La page du jour charge les tuiles d'OpenStreetMap (`tile.openstreetmap.org`) depuis le navigateur ; avec les miniatures de YouTube (`i.ytimg.com`, carte YouTube, sans Referer : Google voit l'IP du téléphone et les vidéos du jour affiché), ce sont les seules ressources extérieures autorisées par la CSP de `web`. OpenStreetMap voit donc l'adresse IP du téléphone, l'origine de l'app (en-tête Referer, sans le chemin) et la zone affichée. Rien ne passe par le NAS.

**Fréquence cardiaque de la nuit** (4 nov. 2026). Aucune action sur le NAS : la migration (`20261104130000_sleep_heart_rate.sql`) part avec l'image, sans `admin rebuild` (les nuits d'avant donnent le même résultat). Le premier passage après le déploiement relit une seule fois tout le sommeil de l'historique avec sa fréquence cardiaque : environ 13 appels de sommeil et 150 appels de séries (`dailyHeartRate`, un par jour), une seconde entre deux, donc quelques minutes, puis une nouvelle version de chaque nuit avec l'annulation de l'ancienne (`cancelled` de l'ordre du nombre de nuits dans « garmin pass completed »). Ensuite, chaque passage ajoute environ 9 appels. Vérifier : la page d'un jour passé montre « FC la plus basse … · moyenne … », et `admin rebuild` donne toujours 0 différence. Une erreur de série apparaît comme `garmin refused` avec `call: heart_rate` et se réessaie seule. Revenir à une image d'avant ne casse rien : l'ancienne version ignore l'indicateur de l'état, et les nuits des 14 derniers jours y repartiraient sans fréquence cardiaque, en révision.

**Retour arrière.** Couper Garmin seul : retirer les deux lignes `GARMIN_*` de `secrets/collector.env`, puis `sudo docker compose up -d collector`. Le journal garde ce qui est entré ; `admin revoke-source garmin` coupe le token. Revenir à une image d'avant : la migration ajoute des registres sans rien changer d'autre, l'ancienne version les ignore.

## Mise à jour : page d'état des trackers

À faire une fois, sur le NAS déjà en service. La PR part en production à la fusion, migration comprise (`20261105120000_source_status.sql`, par le service `migrate` : trois colonnes de `core.sources`, pas de `rebuild`). `compose.yaml` ne change pas. Dès le déploiement, Last.fm et Garmin rapportent leur état après chaque passage ; la page d'état arrive avec la web app. Je ne lance rien de tout cela depuis le dépôt. Dans chaque `docker compose run -T` ou `exec -T`, `</dev/null` évite d'avaler la ligne collée suivante.

1. Vérifier la migration :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   sudo docker compose logs migrate | tail -3   # « migrations à jour »
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select count(*) from _sqlx_migrations" </dev/null   # 8
   ```

2. **Les libellés**, le nom que montre la page (sans libellé, elle montre le nom) :

   ```sh
   sudo docker compose run --rm -T admin set-label garmin "Garmin Connect" </dev/null
   sudo docker compose run --rm -T admin set-label lastfm "Musique" </dev/null
   sudo docker compose run --rm -T admin set-label ios "Raccourci café" </dev/null
   ```

   Le libellé fait aussi paraître une source sans seuil de silence, comme `ios` : une journée sans café n'est pas un silence, il n'en aura pas. `apple`, l'import ponctuel, n'en reçoit pas et n'apparaît pas. Une source recréée (`lastfm-2`) reçoit le libellé de l'ancienne ; l'ancienne, révoquée, disparaît de la page.

3. **L'échéance de la clé Garmin**, la date que Garmin affiche à côté de la clé (Garmin Connect, Paramètres, Personal API Key), en `AAAA-MM-JJ` :

   ```sh
   read -r EXPIRES   # par exemple 2027-11-04
   (umask 077; printf 'GARMIN_KEY_EXPIRES_ON=%s\n' "$EXPIRES" >> secrets/collector.env)
   unset EXPIRES
   sudo docker compose up -d collector
   sudo docker compose logs collector | tail -3   # « garmin pass completed » ; rien sur GARMIN_KEY_EXPIRES_ON
   ```

   Une date illisible empêche le collecteur de démarrer, Last.fm compris (« GARMIN_KEY_EXPIRES_ON illisible ») : corriger la ligne et relancer. À chaque nouvelle clé (étape 6 de « Mise à jour : Garmin Connect »), remplacer aussi cette ligne.

4. Vérifier les états rapportés, sans rien de personnel :

   ```sh
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select name, label, status, status_reported_at from core.sources where status is not null or label is not null order by name" </dev/null
   ```

   `garmin` et `lastfm` : `{"state": "ok", …}` avec `next_run_at`, et `expires_at` pour Garmin (la fin du jour affiché, heure de Paris, en UTC). « status not reported » dans les logs du collecteur : le rapport a échoué, la collecte continue.

**Retour arrière.** Revenir à une image d'avant : la migration ajoute des colonnes que l'ancienne version ignore, et l'ancienne image ignore `GARMIN_KEY_EXPIRES_ON`, une variable qu'elle ne lit pas. Les états restent dans `core.sources`, sans effet.

## Mise à jour : YouTube

À faire une fois, sur le NAS déjà en service. La PR part en production à la fusion, migrations comprises (`20261105130000_youtube.sql` et `20261105130100_item_display_search.sql`, par le service `migrate` ; pas de `rebuild`) : le connecteur YouTube ne démarre que lorsque ses secrets sont dans `secrets/collector.env`, donc rien ne change pour Last.fm et Garmin d'ici là. Je ne lance rien de tout cela depuis le dépôt, et je tape moi-même chaque secret.

1. **Avant la fusion**, recopier `compose.yaml` comme à l'étape 4 de « Mise à jour vers l'étape 5 » : le collecteur passe à 128 Mo de mémoire, et son contrôle de santé vérifie aussi `youtube-state.json`.

2. **Google Cloud**, dans le projet de l'essai du 5 nov. 2026 :
   - API activées : **Data Portability API** et **YouTube Data API v3** ;
   - écran de consentement en mode **Testing**, mon adresse dans les utilisateurs de test (sans elle : `403 access_denied`) ;
   - un client OAuth de type **Application Web**, adresse de retour `https://<nas>.<tailnet>.ts.net:8443/autorisation` (l'adresse de la web app, celle de `tailscale serve`) ; le client « Desktop » de l'essai est supprimé, et son accord retiré (myaccount.google.com, Sécurité, Applications tierces) ;
   - une **clé d'API** restreinte à YouTube Data API v3.

   Le client ID, son secret et la clé vont dans le gestionnaire de mots de passe et dans `secrets/collector.env`, nulle part ailleurs. Le collecteur joint `oauth2.googleapis.com`, `dataportability.googleapis.com`, `storage.googleapis.com` et `www.googleapis.com` par le réseau `egress`.

3. Après le déploiement de la version, sur le NAS, en root dans le dossier en `700` :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   TOKEN=$(sudo docker compose run --rm -T admin create-source youtube --silence-threshold "3 days" --label "YouTube" </dev/null)
   [ "${#TOKEN}" = 64 ] && echo "token ok" || echo "ÉCHEC : ne pas continuer"
   for kind in youtube.watch youtube.video_info record.cancel; do
     sudo docker compose run --rm -T admin grant-kind youtube "$kind" </dev/null
   done
   read -r CLIENT_ID; read -rs CLIENT_SECRET; read -rs API_KEY   # sans écho pour les deux secrets
   read -r REDIRECT_URI   # https://<nas>.<tailnet>.ts.net:8443/autorisation, comme chez Google
   (umask 077; printf 'YOUTUBE_CLIENT_ID=%s\nYOUTUBE_CLIENT_SECRET=%s\nYOUTUBE_REDIRECT_URI=%s\nYOUTUBE_API_KEY=%s\nYOUTUBE_TOKEN=%s\n' \
     "$CLIENT_ID" "$CLIENT_SECRET" "$REDIRECT_URI" "$API_KEY" "$TOKEN" >> secrets/collector.env)
   unset TOKEN CLIENT_ID CLIENT_SECRET API_KEY REDIRECT_URI
   ls -l secrets/collector.env   # -rw-------
   sudo docker compose up -d collector
   sudo docker compose logs collector | grep youtube | tail -2   # « youtube pass completed », awaiting_consent true
   ```

   Il faut les cinq ensemble : un seul manquant empêche le collecteur de démarrer, Last.fm et Garmin compris.

4. **Le premier accord**, depuis la page État de la web app (iPhone ou Mac) : la carte YouTube est « À faire » avec « Donner l'accord ». Google affiche l'avertissement d'app non validée (mode Testing), puis le consentement ; il revient sur `/autorisation`, qui transmet le code. Le collecteur le prend dans la minute, l'échange, et lance l'export de tout l'historique (11 min environ), lu au passage qui suit sa fin :

   ```sh
   sudo docker compose logs collector | grep "youtube pass completed" | tail -1   # exported true, watches, accepted, skipped_*
   ```

   Puis les fiches des vidéos, 1 000 par passage de 5 min : l'historique entier en quelques heures (`to_enrich` descend à 0 ; `unknown_videos` compte les vidéos supprimées ou privées). Google refuse un accord tant que le précédent est actif (« Incremental auth is not allowed ») : le connecteur retire lui-même l'accord qu'il connaît, mais si son état est perdu, retirer l'accès à la main (myaccount.google.com, Sécurité, Applications tierces) avant de recommencer.

5. **Chaque semaine** : le jeton dure 7 jours en mode Testing. Un jour avant son échéance, le connecteur retire l'accord en cours et en demande un autre : la carte passe à « Renouveler ». Un oubli ne perd rien : l'export suivant reprend tout depuis la dernière vue ; seul le silence de 3 jours alerte.

6. Vérifier les critères du chantier YouTube dans [les chantiers](chantiers.md) : un jour réel comparé à mon historique YouTube, sur la page du jour et par le MCP (`day_summary`) ; puis la couverture et le rejeu :

   ```sh
   sudo docker compose run --rm -T admin coverage video.watch </dev/null
   sudo docker compose run --rm -T admin rebuild </dev/null   # 0 différence
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select status from core.sources where name = 'youtube'" </dev/null   # ok, expires_at, next_run_at, last_collected_at
   sudo docker compose exec -T collector stat -c '%a' /var/lib/life-tracker/youtube-state.json   # 600
   ```

   Les logs ne montrent que des nombres et des motifs : jamais un jeton, un code, une URL, un titre ni un id. « client OAuth ou clé YouTube refusés » : le secret client ou la clé est faux, la carte le dit (`key_rejected`).

**Correctif : les réactions YouTube** (5 nov. 2026). La version d'avant a fait des vidéos vues des réactions de l'export (« Disliked https://… », ≈90). Le correctif les écarte et annule les fausses vues par `record.cancel` : la source doit avoir ce droit.

1. **Avant la fusion** (avant le déploiement du correctif), sur le NAS :

   ```sh
   sudo docker compose run --rm -T admin grant-kind youtube record.cancel </dev/null
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select s.name, k.kind from core.source_kinds k join core.sources s on s.id = k.source_id where s.name = 'youtube'" </dev/null   # youtube|record.cancel présent
   ```

   Sans ce droit, rien ne casse : les vues entrent, les annulations sont mises de côté (`kind_not_declared` dans les logs, carte en `error`/`ingest_failed` pour le passage) et repartent à l'export suivant.

2. **Après le déploiement**, l'export suivant (au plus un par 24 h, à l'heure de « Prochain export » sur la carte) reprend tout l'historique une fois :

   ```sh
   sudo docker compose logs collector | grep "youtube pass completed" | grep '"exported":true' | tail -1   # skipped_reactions ≈ 90, cancelled ≈ 90, rejected 0
   ```

   Puis, par le MCP, `day_summary` du 2023-05-05 : plus aucune vidéo « Disliked https://… ». « youtube export without any recognized watch » dans les logs : la langue du compte a changé, voir le journal des décisions.

**Vidéos classiques seulement** (5 nov. 2026, même déploiement). Les Shorts et les vidéos supprimées ou privées n'entrent plus, et ceux que le premier export a fait entrer (≈85 % des 24 555 vues) sont annulés par le même `record.cancel` : aucune autre action sur le NAS. Le premier export complet après le déploiement classe ≈22 000 vidéos par Data v3 (≈440 unités de quota, 100 appels au plus par passage) : cinq passages, une vingtaine de minutes ; « Dernier export » n'avance qu'à la fin.

```sh
sudo docker compose logs collector | grep "youtube pass completed" | grep -v '"classified":0' | tail -6   # skipped_shorts et cancelled en milliers, awaiting_classification jusqu'à 0, rejected 0
sudo docker compose logs collector | grep "youtube journal corrections done"   # une ligne, à la fin
```

Puis, par le MCP, `inventory` : le nombre de « Vidéos YouTube » tombe à ≈15 % d'avant ; `day_summary` d'un jour récent ne montre plus de Short ni de « Vidéo indisponible ».

**Retour arrière.** Couper YouTube seul : retirer les cinq lignes `YOUTUBE_*` de `secrets/collector.env`, puis `sudo docker compose up -d collector`. Le journal garde ce qui est entré ; `admin revoke-source youtube` coupe le token, et retirer l'accès dans le compte Google coupe le jeton. Revenir à une image d'avant : les migrations ajoutent des registres et un index que l'ancienne version ignore.

## Mise à jour : Twitch

À faire une fois, sur le NAS déjà en service. La PR du backend part en production à la fusion, migration comprise (`20261106120000_twitch.sql`, par le service `migrate` : un type et une métrique, pas de `rebuild`). `compose.yaml` ne change pas, et aucun secret ne va dans `secrets/` : le token de la source vit dans l'extension Firefox, sur le Mac. La métrique « Twitch » apparaît dans les filtres du calendrier dès le déploiement, vide jusqu'au premier segment. Je ne lance rien de tout cela depuis le dépôt.

1. Après le déploiement de la version, sur le NAS, en root dans le dossier en `700` :

   ```sh
   sudo -i
   cd /volume1/docker/life-tracker
   sudo docker compose logs migrate | tail -3   # « migrations à jour »
   TOKEN=$(sudo docker compose run --rm -T admin create-source twitch --silence-threshold "3 days" --label "Twitch" </dev/null)
   [ "${#TOKEN}" = 64 ] && echo "token ok" || echo "ÉCHEC : ne pas continuer"
   sudo docker compose run --rm -T admin grant-kind twitch twitch.watch </dev/null
   ```

   Le token ne s'affiche qu'une fois : le copier dans le gestionnaire de mots de passe (pour la page d'options de l'extension), puis `unset TOKEN`. Seulement `twitch.watch` : l'extension n'annule rien, et ce token n'écrit rien d'autre.

2. **L'extension Firefox**, sur le Mac (`extension/`, sans release : elle ne part pas avec les images).
   - **Signer** (une fois par version ; AMO refuse de signer deux fois la même) : augmenter `version` dans `extension/package.json`, puis, avec les clés d'API d'AMO (addons.mozilla.org, « Outils » → « Gérer les clés d'API »), lues depuis le gestionnaire de mots de passe :

     ```sh
     read -rs WEB_EXT_API_KEY && read -rs WEB_EXT_API_SECRET   # collés, sans écho ni historique
     export WEB_EXT_API_KEY WEB_EXT_API_SECRET
     extension/sign.sh
     unset WEB_EXT_API_KEY WEB_EXT_API_SECRET
     ```

     Le script lance `extension/test.sh`, puis `web-ext sign --channel=unlisted` : AMO signe sans rien publier. Le `.xpi` signé arrive dans `extension/web-ext-artifacts/` (ignoré par git). Les clés ne vont ni dans le dépôt, ni dans un fichier, ni en CI.
   - **Installer** : about:addons, roue dentée, « Installer un module depuis un fichier… », le `.xpi`. Firefox demande l'accès à www.twitch.tv et clips.twitch.tv. Pas de mise à jour automatique : chaque nouvelle version se signe puis se réinstalle par-dessus (même identifiant, les réglages et la file restent).
   - **Régler** : le bouton de l'extension (ou about:addons → Préférences) ouvre sa page d'options. Adresse `https://<nas>.<tailnet>.ts.net:8443`, token de la source `twitch`, « Enregistrer » : Firefox demande alors la permission pour ce seul hôte (`https://<nas>.<tailnet>.ts.net/*`, un motif ne porte pas de port). Puis « Tester » : un battement, la file, l'état. Le token n'est jamais réaffiché ; un champ vide garde celui enregistré.
   - **Suivre** : la page d'options montre les segments en attente, le dernier envoi, les segments rejetés et la dernière erreur. Un « ! » rouge sur le bouton : le token est refusé (401) et l'envoi est arrêté ; créer une nouvelle source `twitch-2` (l'étape 1 sous ce nom, avec son `grant-kind` ; `create-source` refuse un nom qui existe déjà, même révoqué), puis `admin revoke-source twitch` ; enregistrer le nouveau token, « Tester ». `twitch-2` reçoit le libellé « Twitch » et la page d'état la reconnaît.
   - **Essai sans signature** (développement) : `cd extension && npm ci && npm run build`, puis about:debugging → « Ce Firefox » → « Charger un module complémentaire temporaire… » → `extension/dist/manifest.json`. Il disparaît à la fermeture de Firefox. Jamais le token de production contre un backend de développement, ni l'inverse.

3. Vérifier les critères du chantier Twitch dans [les chantiers](chantiers.md) : une vraie soirée sur la page du jour et par le MCP (`cumulative_duration` sur une chaîne, `level=channel`, `key=<login>`) ; puis le rejeu :

   ```sh
   sudo docker compose run --rm -T admin coverage stream.watch </dev/null
   sudo docker compose run --rm -T admin rebuild </dev/null   # 0 différence
   ```

**Retour arrière.** Couper Twitch seul : `admin revoke-source twitch` ; l'extension voit alors un 401, s'arrête d'envoyer et affiche « ! ». Le journal garde ce qui est entré. Revenir à une image d'avant : la migration ajoute des registres que l'ancienne version ignore, sauf le calendrier, qui montre alors la métrique comme une somme vide ; rien n'est perdu.

## Vérifier les critères « Terminé quand »

**Un café saisi depuis l'iPhone, par la web app comme par le raccourci, apparaît.**

1. Ouvrir l'adresse `ts.net` dans Safari, se connecter, toucher +1 : le café s'affiche, et il reste après rechargement. Regarder aussi l'affichage en clair et en sombre : c'est le contrôle sur iPhone laissé par l'étape 3.
2. Lancer le raccourci, recharger la page : un second café s'affiche, de source `ios`. L'annuler depuis la web app.

**Un redéploiement garde les données, et `migrate` relancé ne change rien.**

```sh
Q='select (select count(*) from core.records), (select count(*) from _sqlx_migrations)'
sudo docker compose exec -T db psql -U postgres -d tracker -Atc "$Q"
sudo docker compose run --rm migrate
sudo docker compose down && sudo docker compose up -d
sudo docker compose exec -T db psql -U postgres -d tracker -Atc "$Q"   # mêmes nombres
```

Jamais `down -v` : il efface le volume.

**La base n'est joignable par aucun autre conteneur ni utilisateur du NAS.**

```sh
DB=$(sudo docker compose ps -q db)
IP=$(sudo docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$DB")
sudo docker port "$DB"                                                       # vide
sudo docker run --rm --network bridge postgres:18.6 pg_isready -t 4 -h "$IP" # no response
sudo docker run --rm --network host postgres:18.6 pg_isready -t 4 -h "$IP"   # no response
sudo docker inspect "$(sudo docker compose ps -q backend)" | grep -c tracker_owner   # 0
sudo -u AUTRE_COMPTE ls /volume1/docker/life-tracker                          # refusé
```

Si `--network host` répond, l'option `inhibit_ipv4` n'a pas d'effet sur ce NAS : le mot de passe aléatoire reste seul à protéger la base depuis l'hôte, et le critère n'est pas rempli tant que ce point n'est pas réglé. Un administrateur du NAS passe toujours, voir l'étape 3.

## Déploiement automatique

Mis en place le 3 nov. 2026 (release `v0.1.0`), en déploiement continu depuis le même jour : les points incertains sont listés au point 0. Le flux, dans `.github/workflows/release.yml` :

- **Fusion d'une PR qui change le contenu d'une image** : `backend/`, `web/` ou `mcp/`, hors tests (`backend/tests/`, `web/e2e/`, `mcp/test/`) et Markdown. Une PR qui ne touche que `docs/`, la CI, les tests ou `deploy/` ne livre rien, quel que soit son type.
- **CI verte sur `main`.** Release part de la fin du workflow CI (`workflow_run`) et ne fait rien si elle a échoué. Un commit de documentation seule ne lance pas la CI, donc rien.
- **Version** : `deploy/next-version.sh` compare le commit au dernier tag et calcule `vX.Y.Z` d'après les types des commits depuis ce tag. Le type ne décide que du numéro : une rupture (`!` ou `BREAKING CHANGE:`) donne une mineure tant qu'on est sous 1.0.0, une majeure ensuite ; un `feat`, une mineure ; tout le reste (`fix`, `perf`, `refactor`, `build`, `chore`…), un correctif.
- **Tag et GitHub Release**, avec les notes : Fonctionnalités, Corrections, Performances, Autres changements, chaque ligne étant le titre de la PR et son numéro. Les notes de version ne vivent que là.
- **Images** `vX.Y.Z` sur ghcr.io, construites depuis le commit que la CI a testé.
- **NAS** : le job `deploy`, depuis le même commit.

Une release à la fois, jamais interrompue : deux fusions rapprochées se suivent, et une release en attente qui en remplace une autre reprend tous les commits depuis le dernier tag. Un commit dont la CI échoue ne part pas : ses changements partent avec la prochaine fusion dont la CI est verte.

Le job `deploy` :

1. rejoint le tailnet comme nœud éphémère `tag:ci`, par un client OAuth Tailscale ;
2. se connecte en SSH au NAS avec une clé dédiée, `known_hosts` épinglé ;
3. envoie `deploy vX.Y.Z <sha256 de compose.yaml>`, et rien d'autre : la clé n'a pas de shell, seulement une commande forcée.

Sur le NAS, `deploy.sh` (copie de `deploy/deploy.sh`), en root par une seule règle sudoers :

- refuse toute autre commande, et refuse si son `compose.yaml` diffère de celui de la release. Le job ne pousse jamais de fichier : **un `compose.yaml` modifié se copie sur le NAS avant de fusionner la PR qui le change**, comme à l'étape 4 ; sinon le déploiement est refusé, le NAS reste sur l'ancien tag, et on relance par un retour arrière vers la nouvelle version (plus bas) une fois le fichier copié ;
- note l'ancien `TRACKER_TAG`, écrit le nouveau dans `.env`, puis `pull` et `up -d` ;
- attend 3 min au plus que chaque service soit sain : `healthy`, ou en marche s'il n'a pas de contrôle, `migrate` sorti à 0. Le collecteur compte dès qu'il tourne, car son premier contrôle n'a lieu qu'au bout de 5 min ;
- sinon remet l'ancien tag, relance `up -d` et sort en erreur : le job est rouge. Les migrations sont en ajout seul, donc l'ancien code tourne sur le schéma plus récent ;
- n'affiche que des noms de services et des états.

Reste à la main : les copies de `compose.yaml` et de `deploy.sh`, et toute administration que demande la description de la PR (`admin rebuild`, `set-precedence`, un secret…), après le déploiement.

Dans les commandes, `MOI` est mon compte DSM d'administrateur, `NAS_HOST` le nom du NAS dans le tailnet (`nas.TAILNET.ts.net`) ou son adresse `100.x`, toujours le même partout. Aucune valeur secrète ne passe par la conversation avec Claude ni par le dépôt.

0. **Points incertains, à vérifier au premier passage.**

   a. **Le serveur SSH de DSM et la commande forcée** : DSM embarque OpenSSH, qui applique `command=` et `restrict` dans `authorized_keys`, mais je ne l'ai pas vérifié pour un compte administrateur autre que root. Le point 9 le contrôle.

   b. **`/etc/sudoers.d` sur DSM** : il faut que `/etc/sudoers` l'inclue (`sudo grep -n includedir /etc/sudoers`) et qu'une mise à jour de DSM ne l'efface pas. Si l'inclusion manque, ne pas modifier `/etc/sudoers` sans `visudo` ; couper plutôt le déploiement automatique.

   c. **Le dossier personnel** : `~/.ssh` exige le service de dossiers personnels (*Panneau de configuration > Utilisateur et groupe > Avancé*). Le serveur SSH ignore `authorized_keys` si le dossier personnel est ouvert en écriture au groupe ou aux autres.

   d. **Les outils de DSM** : `sha256sum`, `docker compose` (sinon `docker-compose`, que le script prend en repli) et `docker` dans `/usr/local/bin`.

   **Après chaque mise à jour de DSM**, refaire l'essai du point 9 : une mise à jour peut réinitialiser sudoers ou la configuration SSH. Le déploiement échouerait alors sans rien changer, job rouge.

1. **Client OAuth Tailscale.** Console Tailscale, d'abord la politique (point 2) pour que `tag:ci` existe. Puis *Settings > OAuth clients > Generate OAuth client* : la seule permission *Auth Keys* en écriture, avec le tag `tag:ci`. L'identifiant et le secret vont au gestionnaire de mots de passe, puis dans les secrets GitHub (point 7).

2. **Politique Tailscale.** `tag:ci` ne joint que le port SSH du NAS (203 sur le mien, voir *Panneau de configuration > Terminal et SNMP*), et personne ne joint `tag:ci`. À fusionner avec la politique existante, dans *Access controls* :

   ```jsonc
   {
     "tagOwners": {
       "tag:ci": ["autogroup:admin"],
     },
     "hosts": {
       "nas": "100.x.y.z", // l'adresse Tailscale du NAS
     },
     "grants": [
       // Le job de déploiement : SSH vers le NAS, rien d'autre.
       { "src": ["tag:ci"], "dst": ["nas"], "ip": ["tcp:203"] },
     ],
     "tests": [
       { "src": "tag:ci", "accept": ["nas:203"], "deny": ["nas:22", "nas:443", "nas:8443", "nas:5001"] },
     ],
   }
   ```

   **Attention à la règle par défaut** `{"src": ["*"], "dst": ["*"], "ip": ["*"]}` (ou son équivalent en `acls`) : `*` couvre aussi les appareils étiquetés, donc `tag:ci` aurait accès à tout, et tout le tailnet à lui. La remplacer par une règle limitée à mes appareils, par exemple `{"src": ["autogroup:member"], "dst": ["autogroup:self"], "ip": ["*"]}` si le NAS est à mon nom (sinon, viser le NAS par son tag), puis ajouter aux `tests` `{"src": "tag:ci", "deny": ["<un autre appareil>:22"]}`. La console refuse d'enregistrer une politique dont un test échoue. Vérifier ensuite depuis l'iPhone et le Mac que la web app (8443) et le MCP répondent toujours.

3. **Clé SSH dédiée**, sur le Mac, sans phrase de passe puisque le job l'utilise seul :

   ```sh
   ssh-keygen -t ed25519 -N '' -C life-tracker-deploy -f ~/.ssh/life-tracker-deploy
   cat ~/.ssh/life-tracker-deploy.pub
   ```

4. **`authorized_keys` sur le NAS**, dans mon compte, en une ligne. `restrict` coupe pty et redirections ; `from=` n'accepte que le tailnet et la boucle locale : le paquet Tailscale de DSM, en mode userspace, présente les connexions venues du tailnet comme locales (sans elle : « Refused by key options », vérifié le 3 nov. 2026) ; le réseau local reste refusé ; `sudo -n` échoue au lieu de demander un mot de passe :

   ```sh
   mkdir -p ~/.ssh && chmod 700 ~/.ssh
   PUB='ssh-ed25519 AAAA... life-tracker-deploy'   # le contenu du .pub
   printf 'command="sudo -n /volume1/docker/life-tracker/deploy.sh",restrict,from="127.0.0.1,::1,100.64.0.0/10,fd7a:115c:a1e0::/48" %s\n' "$PUB" >> ~/.ssh/authorized_keys
   chmod 600 ~/.ssh/authorized_keys
   ls -ld ~ ~/.ssh ~/.ssh/authorized_keys   # ni ~ ni ~/.ssh ouverts en écriture au groupe
   ```

   Si `restrict` est refusé (OpenSSH trop ancien), le remplacer par `no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding`.

5. **Règle sudoers.** Mon compte lance ce seul script, sans argument (`""`), et `SSH_ORIGINAL_COMMAND` passe : sudo efface le reste de l'environnement. Vérifiée avant installation :

   ```sh
   printf '%s\n' \
     'Defaults!/volume1/docker/life-tracker/deploy.sh env_keep += "SSH_ORIGINAL_COMMAND"' \
     'MOI ALL=(root) NOPASSWD: /volume1/docker/life-tracker/deploy.sh ""' > /tmp/life-tracker-deploy
   sudo visudo -cf /tmp/life-tracker-deploy   # « parsed OK »
   sudo cp /tmp/life-tracker-deploy /etc/sudoers.d/life-tracker-deploy
   sudo chown root:root /etc/sudoers.d/life-tracker-deploy
   sudo chmod 440 /etc/sudoers.d/life-tracker-deploy
   rm /tmp/life-tracker-deploy
   sudo visudo -c
   sudo grep -n includedir /etc/sudoers   # sinon, voir le point 0 b
   ```

6. **`deploy.sh`.** Sur le Mac, depuis le dépôt, puis sur le NAS. Le dossier reste en `700` root : mon compte ne peut ni lire ni modifier le script qu'il a le droit de lancer en root.

   ```sh
   shasum -a 256 deploy/deploy.sh   # à noter
   scp -O -P 203 deploy/deploy.sh MOI@NAS_HOST:/tmp/
   ```

   ```sh
   sudo cp /tmp/deploy.sh /volume1/docker/life-tracker/deploy.sh
   sudo chown root:root /volume1/docker/life-tracker/deploy.sh
   sudo chmod 700 /volume1/docker/life-tracker/deploy.sh
   sudo sha256sum /volume1/docker/life-tracker/deploy.sh   # la même empreinte
   rm /tmp/deploy.sh
   ```

   Quand `deploy/deploy.sh` change dans le dépôt, le recopier de même.

7. **Secrets GitHub**, depuis le Mac à la racine du dépôt. `gh secret set` demande la valeur sans l'afficher, ou la lit dans un fichier :

   ```sh
   ssh-keyscan -p 203 -t ed25519 NAS_HOST > /tmp/nas_known_hosts
   ssh-keygen -lf /tmp/nas_known_hosts   # à comparer, sur le NAS, à : ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
   gh secret set NAS_KNOWN_HOSTS < /tmp/nas_known_hosts && rm /tmp/nas_known_hosts
   gh secret set NAS_SSH_KEY < ~/.ssh/life-tracker-deploy
   gh secret set NAS_HOST           # le même nom que pour ssh-keyscan
   gh secret set NAS_USER           # MOI
   gh secret set NAS_SSH_PORT       # 203
   gh secret set TS_OAUTH_CLIENT_ID
   gh secret set TS_OAUTH_SECRET
   gh secret list
   ```

   La clé privée reste sur le Mac le temps du point 9, puis `rm ~/.ssh/life-tracker-deploy` : une nouvelle clé remplace une clé perdue.

8. **Réglages du dépôt**, tous du plan gratuit. Fusion en squash seule, avec le titre et la description de la PR comme message ; branche supprimée après fusion ; les Actions n'ont pas à approuver ni ouvrir de PR :

   ```sh
   gh api -X PATCH repos/OWNER/life-tracker \
     -F allow_squash_merge=true -F allow_merge_commit=false -F allow_rebase_merge=false \
     -f squash_merge_commit_title=PR_TITLE -f squash_merge_commit_message=PR_BODY \
     -F delete_branch_on_merge=true
   gh api -X PUT repos/OWNER/life-tracker/actions/permissions/workflow \
     -f default_workflow_permissions=read -F can_approve_pull_request_reviews=false
   ```

   Le token `read:packages` du NAS (étape 2) doit rester valide : expiré, le `pull` échoue et le déploiement revient à l'ancien tag.

9. **Essai à blanc**, depuis le Mac relié au tailnet. Rien ne change sur le NAS :

   ```sh
   K=~/.ssh/life-tracker-deploy
   ssh -i "$K" -p 203 -o IdentitiesOnly=yes MOI@NAS_HOST 'deploy v0.0.0 0000000000000000000000000000000000000000000000000000000000000000'
   # REFUS : compose.yaml du NAS différent… (commande forcée, sudo et script en place)
   ssh -i "$K" -p 203 -o IdentitiesOnly=yes MOI@NAS_HOST 'ls /'        # REFUS : commande attendue…
   ssh -i "$K" -p 203 -o IdentitiesOnly=yes -t MOI@NAS_HOST            # REFUS, aucun shell
   ```

   `sudo: a password is required` signale une règle sudoers absente ou ignorée (point 0 b) ; un shell ouvert, une commande forcée ignorée (point 0 a) : retirer la clé tout de suite.

10. **Premier déploiement continu.** La première release, `v0.1.0`, date du 3 nov. 2026 ; le calcul part de son tag. La prochaine PR qui change le code des images (un `feat` ou un `fix` de l'étape 10) doit, une fois fusionnée et la CI de `main` verte, créer `v0.2.0` ou `v0.1.1` avec ses notes (`gh release view`), construire les images, puis déployer. Avant de la fusionner, recopier `compose.yaml` s'il a changé. Ensuite, sur le NAS :

    ```sh
    sudo grep TRACKER_TAG /volume1/docker/life-tracker/.env   # le nouveau tag
    cd /volume1/docker/life-tracker && sudo docker compose ps -a
    ```

    Une PR qui ne change que la doc, la CI ou les tests lance Release sans rien publier : le job `version` écrit « Rien à publier ».

**Couper le déploiement.** Retirer la ligne `life-tracker-deploy` de `~/.ssh/authorized_keys` : le job échoue à la connexion, le NAS ne change pas. Ou désactiver le workflow : `gh workflow disable Release` arrête releases et déploiements, et `gh workflow enable Release` les reprend ; la première release suivante reprend tous les commits depuis le dernier tag. Pour aller plus loin, supprimer `/etc/sudoers.d/life-tracker-deploy`.

**Retour arrière par tag.** Les images de chaque release restent sur ghcr.io. *Actions > Release > Run workflow*, avec le tag `vX.Y.Z` d'une version publiée, redéploie ce tag sans calcul de version ni construction d'images ; de même `gh workflow run Release -f tag=vX.Y.Z`. Le job fait le checkout de ce tag : l'empreinte envoyée est celle de son `compose.yaml`, et le NAS refuse si le sien a changé depuis. Sinon, à la main sur le NAS : `sudo sed -i 's/^TRACKER_TAG=.*/TRACKER_TAG=vX.Y.Z/' .env`, puis `pull` et `up -d`. Une migration ne se défait pas : voir « Retour arrière ».

## Exploitation

**Mise à jour.** D'ordinaire d'elle-même, à la fusion d'un changement du code de production : voir « Déploiement automatique ». À la main, depuis le dossier du projet, avec un tag `vX.Y.Z` publié :

```sh
sudo sed -i 's/^TRACKER_TAG=.*/TRACKER_TAG=vX.Y.Z/' .env
sudo docker compose pull
sudo docker compose up -d
sudo docker compose logs migrate
```

Si le changement touche un interprète, ensuite : `sudo docker compose run --rm admin rebuild`.

**Après le premier déploiement, les migrations existantes ne se modifient plus** : on en ajoute une nouvelle.

**Si `compose.yaml` change**, le recopier comme à l'étape 4. `01-roles.sh` ne resert jamais sur un volume existant.

**Panne de la base.** `/health` répond 503 et le backend passe `unhealthy`, sans redémarrer : `restart` ne réagit qu'à la fin d'un processus. Il reprend seul au retour de la base. Lectures et écritures répondent 500 entre-temps, et l'app le traite comme une erreur passagère.

**Collecteur.** Chaque connecteur réessaie seul (1, 2, 4… jusqu'à 30 min ; le délai demandé par une réponse 429), sans jamais avancer son curseur, et la panne de l'un n'arrête pas l'autre. `unhealthy` : un connecteur configuré n'a fait aucun passage réussi depuis 6 h, sans redémarrage. Effacer son volume ne coûte qu'un rattrapage en doublons ignorés, à une réserve près pour Garmin : une révision survenue entre-temps ne peut plus annuler la version d'avant (voir l'architecture, 4 nov. 2026). Remplacer la clé : modifier `secrets/collector.env`, puis `sudo docker compose up -d collector`. Une écoute qui arrive sur Last.fm plus de 48 h après sa date n'entre pas d'elle-même ; effacer le volume la rattrape.

**Logs.** `sudo docker compose logs <service>`. Rotation par taille : 3 fichiers de 10 Mo par service. Docker ne sait pas purger par âge : la règle des 30 jours est un ordre de grandeur, et un service peu bavard garde ses logs plus longtemps. Le backend ne journalise aucune donnée personnelle, et nginx n'écrit pas de journal d'accès.

## Retour arrière

**Sans nouvelle migration entre les deux versions.** Remettre l'ancien `TRACKER_TAG`, puis `pull` et `up -d`. Si un interprète a changé : `admin rebuild --allow-differences`, car `derived` se reconstruit.

**Ce qui ne se défait pas :**

- **Une migration appliquée.** Il n'y a pas de version descendante, et sqlx refuse une base qui porte une migration que l'image ne connaît pas : `migrate` échoue, donc le backend ne démarre pas. On corrige en avançant, par une nouvelle image et une nouvelle migration.
- **`down -v` ou la suppression du volume.** Sans sauvegarde, tout est perdu.
- **Une purge.** C'est son but.
- **Un token perdu, une source révoquée.** On crée une nouvelle source. Le passé reste.
- **HSTS.** Un navigateur déjà passé par l'adresse exige HTTPS sur ce nom pendant un an. Sans gêne tant que ce nom reste servi par `tailscale serve`.
- **La montée de version majeure de PostgreSQL.** Jamais par changement d'image : par export et restauration.

## Sauvegardes

Le POC tourne sans sauvegarde : c'est décidé (voir [l'architecture](architecture.md)). **Les clics café sont irremplaçables**, et, quand ils existeront, l'annuaire et la clé de pseudonymisation. Le compte, les tokens et les mots de passe se recréent ; pas le journal, qui ne vit que dans ce volume. Le RAID ne protège ni d'une suppression, ni d'un vol. Les écoutes et l'historique Apple se retéléchargent.

## Ce qui reste à vérifier

Confirmé le 31 oct. 2026 : le workflow et la publication sur ghcr.io, le démarrage sur le NAS, `tailscale serve` et son certificat, la web app en HTTPS sur l'iPhone, un café conservé après `down` puis `up`, `migrate` sans effet une fois relancé, et la base injoignable depuis les réseaux `bridge` et `host` (`inhibit_ipv4` agit bien sur ce noyau).

Reste à confirmer :

- le raccourci sur un vrai iPhone (étape 10), reporté ;
- le déploiement automatique, de bout en bout : les points incertains de « Déploiement automatique » (commande forcée et sudoers sur DSM), l'essai à blanc, le premier déploiement continu (point 10), un retour arrière par *Run workflow*, puis un échec voulu (fausse empreinte, clé retirée) qui laisse le NAS sur l'ancien tag ;
- la persistance de `tailscale serve` après un redémarrage du NAS ;
- l'import du projet dans l'interface de Container Manager, facultatif ;
- l'accès refusé depuis un autre compte du NAS, si un compte non administrateur existe ;
- de l'étape 9 sur le NAS (le reste est confirmé le 3 nov. 2026 : Funnel sur 443, Claude.ai et le SDK v2, la connexion réservée au tailnet) : la question du premier café posée à Claude.ai, une fois de vrais cafés saisis, `enable_ip_masquerade` sur le Docker de DSM, et les temps de réponse des nouvelles lectures sur le Celeron, non relevés.

`cpus` n'est pas utilisé dans `compose.yaml` : de mémoire, le noyau de Synology refuse cette limite, sans que j'aie pu le vérifier. Seule la mémoire est limitée.
