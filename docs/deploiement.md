# Life tracker — Déploiement sur le NAS

Runbook de l'étape 4. Tout s'exécute à la main : rien n'automatise le déploiement. État : **préparé et vérifié en local**, jamais lancé sur le vrai NAS (voir « Ce qui reste à vérifier »).

**Règle.** Rien de ce qui suit ne se copie dans le dépôt : ni `.env`, ni `secrets/`, ni token, ni mot de passe, ni sortie de commande qui en contient. Le dépôt ne reçoit que des modèles. Dans les commandes, `OWNER` est le compte GitHub, en minuscules.

## Ce qui tourne

| Service | Rôle | Reçoit | Réseaux |
| --- | --- | --- | --- |
| `db` | PostgreSQL 18.6, volume `life-tracker-prod-db-data` | `secrets/db.env` | `data` |
| `migrate` | Une fois à chaque `up`, puis s'arrête (état « arrêté » normal) | `secrets/migrate.env` (`tracker_owner`) | `data` |
| `backend` | API et interprètes | `secrets/backend.env` (`tracker_app`) | `data`, `front` |
| `web` | nginx : app statique et proxy de `/v1` et `/health` | aucun secret | `front`, `edge` |
| `admin` | Profil `admin`, lancé à la main | `secrets/admin.env` (`tracker_admin`) | `data` |

Seul `web` publie un port, sur `127.0.0.1:18080` par défaut. `data` et `front` n'ont aucune sortie, et `data` n'a aucune adresse côté hôte. À l'étape 5, le backend aura besoin d'une sortie vers Last.fm.

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
- l'accès refusé depuis un autre compte du NAS, si un compte non administrateur existe.

`cpus` n'est pas utilisé dans `compose.yaml` : de mémoire, le noyau de Synology refuse cette limite, sans que j'aie pu le vérifier. Seule la mémoire est limitée.
