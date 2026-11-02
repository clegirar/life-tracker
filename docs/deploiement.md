# Life tracker — Déploiement sur le NAS

Runbook de l'étape 4. Tout s'exécute à la main : rien n'automatise le déploiement. État : **préparé et vérifié en local**, jamais lancé sur le vrai NAS (voir « Ce qui reste à vérifier »).

**Règle.** Rien de ce qui suit ne se copie dans le dépôt : ni `.env`, ni `secrets/`, ni token, ni mot de passe, ni sortie de commande qui en contient. Le dépôt ne reçoit que des modèles. Dans les commandes, `OWNER` est le compte GitHub, en minuscules.

## Ce qui tourne

| Service | Rôle | Reçoit | Réseaux |
| --- | --- | --- | --- |
| `db` | PostgreSQL 18.6, volume `life-tracker-prod-db-data` | `secrets/db.env` | `data` |
| `migrate` | Une fois à chaque `up`, puis s'arrête (état « arrêté » normal) | `secrets/migrate.env` (`tracker_owner`) | `data` |
| `backend` | API et interprètes | `secrets/backend.env` (`tracker_app`) | `data`, `front`, `collect` |
| `web` | nginx : app statique et proxy de `/v1` et `/health` | aucun secret | `front`, `edge` |
| `collector` | Interroge Last.fm, écrit par l'API d'ingestion ; curseur dans le volume `life-tracker-prod-collector-state` | `secrets/collector.env` (clé, utilisateur, token) | `collect`, `egress` |
| `admin` | Profil `admin`, lancé à la main | `secrets/admin.env` (`tracker_admin`) | `data` |
| `apple-import` | Profil `import`, lancé à la main : l'import de l'export Apple | Le token sur l'entrée standard, l'archive en lecture seule | `collect` |

Seul `web` publie un port, sur `127.0.0.1:18080` par défaut. `data`, `front` et `collect` n'ont aucune sortie, et `data` n'a aucune adresse côté hôte. `egress` est la seule sortie vers Internet, et seul le collecteur y est : il ne voit pas la base, et le backend ne voit pas Internet. Docker ne filtre pas par destination : le collecteur pourrait joindre autre chose que Last.fm, ce que limitent l'absence de secret de base, `read_only` et l'absence de tout port entrant. Il ne connaît que ce qu'il envoie déjà.

## Ce que je fais à la main

### 1. Dépôt privé et images

Sur le Mac, à la racine du dépôt, une fois les changements commités :

```sh
gh repo create life-tracker --private --source=. --push
```

Le push sur `main` lance `.github/workflows/images.yml` : `./test.sh`, puis les images. Une fois le workflow vert :

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

Dans `.env`, remplacer `OWNER` et `COMMIT_SHA` :

```sh
OWNER_MINUSCULES=...
SHA_COMPLET=...
sed -i "s|ghcr.io/OWNER/|ghcr.io/$OWNER_MINUSCULES/|; s|COMMIT_SHA|$SHA_COMPLET|" .env
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
5. **Obtenir le contenu de l'URL** : `https://<nas>.<tailnet>.ts.net/v1/records`, méthode **POST**, avec trois en-têtes :
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
   sudo docker compose exec -T db psql -U postgres -d tracker -Atc "select kind from core.source_kinds sk join core.sources s on s.id = sk.source_id where s.name = 'lastfm' and revoked_at is null"   # les deux types
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

## Exploitation

**Mise à jour.** Après un workflow vert, depuis le dossier du projet :

```sh
sudo sed -i 's/^TRACKER_TAG=.*/TRACKER_TAG=NOUVEAU_SHA/' .env
sudo docker compose pull
sudo docker compose up -d
sudo docker compose logs migrate
```

Si le changement touche un interprète, ensuite : `sudo docker compose run --rm admin rebuild`.

**Après le premier déploiement, les migrations existantes ne se modifient plus** : on en ajoute une nouvelle.

**Si `compose.yaml` change**, le recopier comme à l'étape 4. `01-roles.sh` ne resert jamais sur un volume existant.

**Panne de la base.** `/health` répond 503 et le backend passe `unhealthy`, sans redémarrer : `restart` ne réagit qu'à la fin d'un processus. Il reprend seul au retour de la base. Lectures et écritures répondent 500 entre-temps, et l'app le traite comme une erreur passagère.

**Collecteur.** Il réessaie seul (1, 2, 4… jusqu'à 30 min ; le délai demandé par une réponse 429), sans jamais avancer son curseur. `unhealthy` : aucun passage réussi depuis 6 h, sans redémarrage. Effacer son volume ne coûte qu'un rattrapage en doublons ignorés. Remplacer la clé : modifier `secrets/collector.env`, puis `sudo docker compose up -d collector`. Une écoute qui arrive sur Last.fm plus de 48 h après sa date n'entre pas d'elle-même ; effacer le volume la rattrape.

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
- la persistance de `tailscale serve` après un redémarrage du NAS ;
- l'import du projet dans l'interface de Container Manager, facultatif ;
- l'accès refusé depuis un autre compte du NAS, si un compte non administrateur existe ;
- tout l'étape 7 sur le NAS : l'image, le service `apple-import` (droits `10001`, lecture seule, réseau), le temps réel, et la jonction entre l'export et Last.fm. Vérifié sur le Mac par le binaire natif et `compose config`, pas par l'image construite ;
- tout l'étape 8 sur le NAS : la vue du mois sur les vraies données (écoutes depuis janvier 2022, cafés depuis le 31 oct. 2026, jonction du 12 octobre), son temps de réponse sur le Celeron, et le signal « en attente » après une vraie coupure du collecteur. Vérifié sur le Mac par les tests et des données synthétiques, pas par l'image construite sur le NAS ;
- tout le collecteur sur le NAS : sortie vers Last.fm, vraies réponses, et le critère de l'étape 5. Vérifié seulement en local, en compose, contre `example.com` et sans clé réelle.

`cpus` n'est pas utilisé dans `compose.yaml` : de mémoire, le noyau de Synology refuse cette limite, sans que j'aie pu le vérifier. Seule la mémoire est limitée.
