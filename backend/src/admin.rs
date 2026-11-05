//! Outil d'administration : tourne en tracker_admin, jamais dans le serveur.

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::auth;

/// En dessous, le mot de passe protège mal : le compte n'a pas de limite de tentatives pendant le POC.
const MIN_PASSWORD_CHARS: usize = 12;
/// Une première écoute séparée de la suivante par plus que cela n'est pas un démarrage : c'est une date
/// ancienne, un test ou un import, et elle ne doit pas donner la priorité sur tout ce qui la suit.
const MAX_START_GAP_DAYS: i64 = 7;
/// La limite de `core.sources.label`.
const MAX_LABEL_CHARS: usize = 80;

/// SHA-256 du token, tel que stocké dans core.sources.token_hash.
pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub(crate) fn generate_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("CSPRNG indisponible : {e}"))?;
    Ok(hex::encode(bytes))
}

/// Crée une source à token et renvoie le token : seul son haché est conservé. Le libellé est le nom
/// affiché par la page d'état ; sans lui, elle montre le nom.
pub async fn create_source(
    pool: &PgPool,
    name: &str,
    silence_threshold: Option<&str>,
    label: Option<&str>,
) -> Result<String> {
    let label = label.map(check_label).transpose()?;
    let token = generate_token()?;
    let hash = token_hash(&token);
    sqlx::query!(
        "INSERT INTO core.sources (name, auth, token_hash, silence_threshold, label)
         VALUES ($1, 'token', $2, $3::text::interval, $4)",
        name,
        hash,
        silence_threshold,
        label,
    )
    .execute(pool)
    .await?;
    Ok(token)
}

/// Fixe le nom affiché d'une source, révoquée ou non.
pub async fn set_label(pool: &PgPool, name: &str, label: &str) -> Result<()> {
    let label = check_label(label)?;
    let updated = sqlx::query!(
        "UPDATE core.sources SET label = $2 WHERE name = $1",
        name,
        label,
    )
    .execute(pool)
    .await?;
    if updated.rows_affected() == 0 {
        bail!("source inconnue : {name}");
    }
    Ok(())
}

/// Les règles de la base (`core.sources.label`), dites ici en clair plutôt que par un refus de contrainte.
fn check_label(label: &str) -> Result<&str> {
    let label = label.trim();
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        bail!("libellé vide ou trop long : {MAX_LABEL_CHARS} caractères au plus");
    }
    Ok(label)
}

pub async fn revoke_source(pool: &PgPool, name: &str) -> Result<()> {
    let revoked = sqlx::query!(
        "UPDATE core.sources SET revoked_at = now() WHERE name = $1 AND revoked_at IS NULL",
        name,
    )
    .execute(pool)
    .await?;
    if revoked.rows_affected() == 0 {
        bail!("source inconnue ou déjà révoquée : {name}");
    }
    Ok(())
}

/// Crée un jeton de service de l'API de lecture (le serveur MCP) et le renvoie : seul son haché est
/// conservé. Il ne lit qu'au niveau « pseudonymes » et n'ouvre aucune écriture (voir `api::guard`).
pub async fn create_service_token(pool: &PgPool, name: &str) -> Result<String> {
    let token = generate_token()?;
    let hash = token_hash(&token);
    sqlx::query!(
        "INSERT INTO core.service_tokens (name, token_hash) VALUES ($1, $2)",
        name,
        hash,
    )
    .execute(pool)
    .await?;
    Ok(token)
}

pub async fn revoke_service_token(pool: &PgPool, name: &str) -> Result<()> {
    let revoked = sqlx::query!(
        "UPDATE core.service_tokens SET revoked_at = now() WHERE name = $1 AND revoked_at IS NULL",
        name,
    )
    .execute(pool)
    .await?;
    if revoked.rows_affected() == 0 {
        bail!("jeton de service inconnu ou déjà révoqué : {name}");
    }
    Ok(())
}

/// Déclare un type pour une source ; une déclaration révoquée redevient active.
pub async fn grant_kind(pool: &PgPool, source: &str, kind: &str) -> Result<()> {
    let granted = sqlx::query!(
        "INSERT INTO core.source_kinds (source_id, kind)
         SELECT id, $2 FROM core.sources WHERE name = $1
         ON CONFLICT (source_id, kind) DO UPDATE SET granted_at = now(), revoked_at = NULL
         WHERE core.source_kinds.revoked_at IS NOT NULL",
        source,
        kind,
    )
    .execute(pool)
    .await?;
    if granted.rows_affected() == 0 {
        let exists = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM core.sources WHERE name = $1) AS "exists!""#,
            source,
        )
        .fetch_one(pool)
        .await?;
        if !exists {
            bail!("source inconnue : {source}");
        }
    }
    Ok(())
}

pub async fn revoke_kind(pool: &PgPool, source: &str, kind: &str) -> Result<()> {
    let revoked = sqlx::query!(
        "UPDATE core.source_kinds sk SET revoked_at = now()
         FROM core.sources s
         WHERE s.id = sk.source_id AND s.name = $1 AND sk.kind = $2 AND sk.revoked_at IS NULL",
        source,
        kind,
    )
    .execute(pool)
    .await?;
    if revoked.rows_affected() == 0 {
        bail!("aucune déclaration active de {kind} pour {source}");
    }
    Ok(())
}

/// Crée ou remplace le compte unique. Changer le mot de passe ferme toutes les sessions.
pub async fn set_password(pool: &PgPool, username: &str, password: &str) -> Result<()> {
    if username.trim().is_empty() {
        bail!("identifiant vide");
    }
    if password.chars().count() < MIN_PASSWORD_CHARS {
        bail!("mot de passe trop court : {MIN_PASSWORD_CHARS} caractères au moins");
    }
    let hash = auth::hash_password(password)?;
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "INSERT INTO core.account (username, password_hash) VALUES ($1, $2)
         ON CONFLICT (id) DO UPDATE
         SET username = EXCLUDED.username, password_hash = EXCLUDED.password_hash, updated_at = now()",
        username,
        hash,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM core.sessions")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Ce qu'une source a produit pour une métrique, et que rien n'a jamais masqué : toute la génération
/// active, y compris ce qu'une priorité écarte. Des nombres et des dates, jamais de contenu.
#[derive(Debug, PartialEq)]
pub struct SourceCoverage {
    pub source: String,
    pub events: i64,
    pub first_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
    /// La deuxième donnée : un grand écart avec la première dit que celle-ci est isolée.
    pub second_at: Option<DateTime<Utc>>,
}

pub async fn coverage(pool: &PgPool, metric: &str) -> Result<Vec<SourceCoverage>> {
    let rows = sqlx::query!(
        r#"SELECT src.name AS source, count(*) AS "events!",
                  min(e.occurred_at) AS "first_at!", max(e.occurred_at) AS "last_at!",
                  (array_agg(e.occurred_at ORDER BY e.occurred_at))[2] AS second_at
           FROM derived.events e
           JOIN derived.generations g ON g.id = e.generation AND g.state = 'active'
           JOIN core.sources src ON src.id = e.source_id
           WHERE e.metric = $1
           GROUP BY src.name
           ORDER BY min(e.occurred_at)"#,
        metric,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SourceCoverage {
            source: row.source,
            events: row.events,
            first_at: row.first_at,
            last_at: row.last_at,
            second_at: row.second_at,
        })
        .collect())
}

/// `source` fait foi pour `metric` à partir de `from` (voir la réconciliation dans l'architecture). Sans
/// date, sa première donnée : refusé si cette donnée est isolée, car elle ferait disparaître tout ce
/// qu'une autre source sait de la période. Remplace la priorité déclarée pour cette métrique, sans
/// toucher au journal ni aux événements : les vues suivent à la prochaine lecture. Renvoie la date.
pub async fn set_precedence(
    pool: &PgPool,
    metric: &str,
    source: &str,
    from: Option<DateTime<Utc>>,
) -> Result<DateTime<Utc>> {
    let from = match from {
        Some(from) => from,
        None => {
            let covered = coverage(pool, metric).await?;
            let Some(own) = covered.iter().find(|c| c.source == source) else {
                bail!("aucune donnée de {source} pour {metric} : préciser --from");
            };
            let isolated = own
                .second_at
                .is_some_and(|next| (next - own.first_at).num_days() > MAX_START_GAP_DAYS);
            if isolated {
                bail!(
                    "la première donnée de {source} ({}) est isolée : préciser --from",
                    own.first_at.format("%Y-%m-%d")
                );
            }
            own.first_at
        }
    };
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "DELETE FROM core.source_precedence WHERE metric = $1",
        metric
    )
    .execute(&mut *tx)
    .await?;
    let written = sqlx::query!(
        "INSERT INTO core.source_precedence (metric, source_id, valid_during)
         SELECT $1, s.id, tstzrange($3, NULL, '[)') FROM core.sources s WHERE s.name = $2",
        metric,
        source,
        from,
    )
    .execute(&mut *tx)
    .await?;
    if written.rows_affected() == 0 {
        bail!("source inconnue : {source}");
    }
    tx.commit().await?;
    Ok(from)
}
