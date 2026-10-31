//! Outil d'administration : tourne en tracker_admin, jamais dans le serveur.

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::auth;

/// En dessous, le mot de passe protège mal : le compte n'a pas de limite de tentatives pendant le POC.
const MIN_PASSWORD_CHARS: usize = 12;

/// SHA-256 du token, tel que stocké dans core.sources.token_hash.
pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub(crate) fn generate_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("CSPRNG indisponible : {e}"))?;
    Ok(hex::encode(bytes))
}

/// Crée une source à token et renvoie le token : seul son haché est conservé.
pub async fn create_source(
    pool: &PgPool,
    name: &str,
    silence_threshold: Option<&str>,
) -> Result<String> {
    let token = generate_token()?;
    let hash = token_hash(&token);
    sqlx::query!(
        "INSERT INTO core.sources (name, auth, token_hash, silence_threshold)
         VALUES ($1, 'token', $2, $3::text::interval)",
        name,
        hash,
        silence_threshold,
    )
    .execute(pool)
    .await?;
    Ok(token)
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
