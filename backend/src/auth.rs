//! Le compte web : mot de passe en Argon2id, sessions tirées au hasard et stockées en SHA-256.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::http::HeaderMap;
use axum::http::header::COOKIE;
use sqlx::PgPool;

use crate::admin::token_hash;

pub const SESSION_COOKIE: &str = "lt_session";
/// Durée fixe depuis la connexion, sans renouvellement.
pub const SESSION_SECONDS: i64 = 30 * 24 * 3600;

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|_| anyhow::anyhow!("hachage du mot de passe impossible"))
}

/// Les paramètres viennent du haché : les renforcer plus tard ne casse pas le mot de passe existant.
pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    PasswordHash::new(stored_hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

/// `Secure` sauf réglage de développement explicite. `Strict` ne gêne rien : les pages sont
/// statiques et publiques, seuls les appels de l'app, de même origine, portent le cookie.
pub fn session_cookie(value: &str, max_age: i64, insecure: bool) -> String {
    let secure = if insecure { "" } else { "; Secure" };
    format!(
        "{SESSION_COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict{secure}"
    )
}

pub fn session_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == SESSION_COOKIE && !value.is_empty()).then_some(value)
        })
}

/// Crée la session d'un token tiré par `generate_token` : seul son haché est conservé.
pub async fn create_session(pool: &PgPool, token: &str) -> Result<(), sqlx::Error> {
    let hash = token_hash(token);
    // Rien d'autre ne fait le ménage, et les connexions sont rares.
    sqlx::query!("DELETE FROM core.sessions WHERE expires_at <= now()")
        .execute(pool)
        .await?;
    sqlx::query!(
        "INSERT INTO core.sessions (token_hash, expires_at)
         VALUES ($1, now() + make_interval(secs => $2))",
        hash,
        SESSION_SECONDS as f64,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// La session existe et n'a pas expiré ; note son dernier usage.
pub async fn session_is_valid(pool: &PgPool, token: &str) -> Result<bool, sqlx::Error> {
    let hash = token_hash(token);
    let found = sqlx::query!(
        "UPDATE core.sessions SET last_seen_at = now()
         WHERE token_hash = $1 AND expires_at > now()",
        hash,
    )
    .execute(pool)
    .await?;
    Ok(found.rows_affected() == 1)
}

pub async fn delete_session(pool: &PgPool, token: &str) -> Result<(), sqlx::Error> {
    let hash = token_hash(token);
    sqlx::query!("DELETE FROM core.sessions WHERE token_hash = $1", hash)
        .execute(pool)
        .await?;
    Ok(())
}
