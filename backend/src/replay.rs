//! Rejeu, par l'outil d'administration : `rebuild` construit une génération à côté, la compare
//! puis bascule ; `retry-errors` réinterprète les enregistrements en erreur de l'active.

use anyhow::{Context, Result, bail};
use sqlx::{Connection, PgPool, Postgres, Transaction};

use crate::engine::{self, Applied, RawRecord, Target};
use crate::interpreters::Interpreter;

/// Une rebuild à la fois, le temps de sa connexion : une `building` trouvée sans ce verrou est
/// forcément le reste d'une rebuild interrompue. Le verrou de la bascule, lui, est sur les lignes.
const REBUILD_LOCK: i64 = 7461002;
/// Chaque tour ne rattrape que ce que le worker a traité pendant le précédent : cela converge vite.
const MAX_SWITCH_ATTEMPTS: u32 = 100;

/// Une ligne qui n'existe pas à l'identique dans les deux générations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    pub table: String,
    pub seq: i64,
}

#[derive(Debug)]
pub struct Comparison {
    pub differences: Vec<Difference>,
    /// Jusqu'où la comparaison est allée, type par type.
    pub compared_up_to: Vec<(String, i64)>,
}

#[derive(Debug)]
pub enum SwitchOutcome {
    Switched(Comparison),
    Differs(Comparison),
    /// Le worker a dépassé la génération construite : la rattraper puis recommencer.
    Lagging,
}

#[derive(Debug)]
pub struct RebuildReport {
    pub generation: i32,
    pub switched: bool,
    pub comparison: Comparison,
}

/// Compare deux générations, chaque type jusqu'au plus petit de leurs deux curseurs : au-delà, l'une
/// n'a simplement pas encore travaillé. Ni le numéro de génération, ni les horodatages de traitement,
/// ni la version d'interprète : seul compte ce qui est produit. Une seule instruction, donc une seule
/// photo de la base. Ne renvoie que des identifiants, jamais de contenu.
pub async fn compare(
    conn: &mut sqlx::PgConnection,
    a: i32,
    b: i32,
) -> Result<Vec<Difference>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"WITH horizon AS (
             SELECT COALESCE(x.kind, y.kind) AS kind, LEAST(x.last_seq, y.last_seq) AS last_seq
             FROM (SELECT * FROM derived.cursors WHERE generation = $1) x
             FULL JOIN (SELECT * FROM derived.cursors WHERE generation = $2) y USING (kind)
           ), content AS (
             SELECT 'events' AS tbl, generation, seq,
                    jsonb_build_array(ordinal, metric, source_id, occurred_at, utc_offset_minutes,
                                      value, details) AS row
             FROM derived.events WHERE generation IN ($1, $2)
             UNION ALL
             SELECT 'event_items', generation, seq,
                    jsonb_build_array(ordinal, level, item_key, display, metric, occurred_at)
             FROM derived.event_items WHERE generation IN ($1, $2)
             UNION ALL
             SELECT 'cancellations', generation, cancel_seq,
                    jsonb_build_array(target_source_id, target_key)
             FROM derived.cancellations WHERE generation IN ($1, $2)
             UNION ALL
             SELECT 'interpretation_errors', generation, seq, jsonb_build_array(error)
             FROM derived.interpretation_errors WHERE generation IN ($1, $2)
           )
           SELECT DISTINCT c.tbl AS "table!", c.seq AS "seq!"
           FROM content c
           JOIN core.records r ON r.seq = c.seq
           JOIN horizon h ON h.kind = r.kind AND c.seq <= h.last_seq
           GROUP BY c.tbl, c.seq, c.row
           HAVING count(*) FILTER (WHERE c.generation = $1)
                  <> count(*) FILTER (WHERE c.generation = $2)
           ORDER BY 1, 2"#,
        a,
        b,
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Difference {
            table: r.table,
            seq: r.seq,
        })
        .collect())
}

async fn horizons(
    conn: &mut sqlx::PgConnection,
    a: i32,
    b: i32,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT kind AS "kind!", LEAST(x.last_seq, y.last_seq) AS "last_seq!"
           FROM (SELECT * FROM derived.cursors WHERE generation = $1) x
           FULL JOIN (SELECT * FROM derived.cursors WHERE generation = $2) y USING (kind)
           ORDER BY 1"#,
        a,
        b,
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|r| (r.kind, r.last_seq)).collect())
}

/// Efface les restes (construction interrompue, anciennes générations) et ouvre une génération neuve.
pub async fn begin_building(pool: &PgPool, registry: &[Interpreter]) -> Result<i32, sqlx::Error> {
    remove_inactive(pool).await?;
    sqlx::query_scalar!(
        "INSERT INTO derived.generations (state, interpreter_versions)
         VALUES ('building', $1::text::jsonb) RETURNING id",
        engine::versions_json(registry),
    )
    .fetch_one(pool)
    .await
}

async fn remove_inactive(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM derived.generations WHERE state IN ('building', 'retired')")
        .execute(pool)
        .await?;
    Ok(())
}

/// Avance la génération jusqu'au bout du journal tel qu'il est maintenant, pas au-delà.
pub async fn catch_up(
    pool: &PgPool,
    registry: &[Interpreter],
    building: i32,
) -> Result<(), sqlx::Error> {
    let up_to =
        sqlx::query_scalar!(r#"SELECT COALESCE(max(seq), 0) AS "up_to!" FROM core.records"#)
            .fetch_one(pool)
            .await?;
    let target = Target::Building {
        id: building,
        up_to,
    };
    // Seulement les types qui ont du retard : un passage à vide coûte une transaction par type, et
    // la bascule n'aboutit que si elle devance le prochain enregistrement du worker.
    let kinds: Vec<&str> = registry.iter().map(|i| i.kind).collect();
    let behind = sqlx::query_scalar!(
        r#"SELECT k AS "kind!" FROM unnest($1::text[]) AS k
           WHERE EXISTS (
             SELECT 1 FROM core.records r
             WHERE r.kind = k AND r.seq <= $2
               AND r.seq > COALESCE((SELECT c.last_seq FROM derived.cursors c
                                     WHERE c.generation = $3 AND c.kind = k), 0))"#,
        &kinds as &[&str],
        up_to,
        building,
    )
    .fetch_all(pool)
    .await?;
    let behind: Vec<Interpreter> = registry
        .iter()
        .filter(|i| behind.iter().any(|kind| kind == i.kind))
        .copied()
        .collect();
    while engine::pass_batch(pool, &behind, target, engine::WORKER_BATCH).await? > 0 {}
    Ok(())
}

/// Compare et bascule dans une seule transaction, les deux lignes de génération verrouillées : le
/// worker attend, l'active ne bouge plus entre la comparaison et le changement d'état. Les lectures
/// ne sont jamais bloquées, et l'ingestion non plus : ce qui arrive pendant ce temps reste au-delà du
/// curseur de la nouvelle active, et le worker le traitera.
pub async fn try_switch(
    pool: &PgPool,
    registry: &[Interpreter],
    building: i32,
    allow_differences: bool,
) -> Result<SwitchOutcome> {
    let mut tx = pool.begin().await?;
    let active = lock_for_switch(&mut tx, building).await?;
    let kinds: Vec<&str> = registry.iter().map(|i| i.kind).collect();
    // Un type retiré du registre ne rattrapera jamais : il ne bloque pas, il apparaît en différence.
    let lagging = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM derived.cursors a
             LEFT JOIN derived.cursors b ON b.generation = $2 AND b.kind = a.kind
             WHERE a.generation = $1 AND a.kind = ANY($3::text[])
               AND a.last_seq > COALESCE(b.last_seq, 0)
           ) AS "lagging!""#,
        active,
        building,
        &kinds as &[&str],
    )
    .fetch_one(&mut *tx)
    .await?;
    if lagging {
        return Ok(SwitchOutcome::Lagging);
    }
    let comparison = Comparison {
        differences: compare(&mut tx, active, building).await?,
        compared_up_to: horizons(&mut tx, active, building).await?,
    };
    if !comparison.differences.is_empty() && !allow_differences {
        return Ok(SwitchOutcome::Differs(comparison));
    }
    // Dans cet ordre : l'index n'accepte qu'une seule active.
    sqlx::query!(
        "UPDATE derived.generations SET state = 'retired' WHERE id = $1",
        active,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE derived.generations SET state = 'active', activated_at = now() WHERE id = $1",
        building,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(SwitchOutcome::Switched(comparison))
}

async fn lock_for_switch(tx: &mut Transaction<'_, Postgres>, building: i32) -> Result<i32> {
    let rows = sqlx::query!(
        r#"SELECT id, state::text AS "state!" FROM derived.generations
           WHERE state = 'active' OR id = $1 ORDER BY id FOR UPDATE"#,
        building,
    )
    .fetch_all(&mut **tx)
    .await?;
    if !rows
        .iter()
        .any(|r| r.id == building && r.state == "building")
    {
        bail!("génération {building} : plus en construction");
    }
    rows.iter()
        .find(|r| r.state == "active")
        .map(|r| r.id)
        .context("aucune génération active")
}

pub async fn rebuild(
    pool: &PgPool,
    registry: &[Interpreter],
    allow_differences: bool,
) -> Result<RebuildReport> {
    // Une connexion à part, hors du pool : la fermer libère le verrou, même en cas d'erreur.
    let mut lock = pool.acquire().await?.detach();
    let locked = sqlx::query_scalar!(
        r#"SELECT pg_try_advisory_lock($1) AS "locked!""#,
        REBUILD_LOCK,
    )
    .fetch_one(&mut lock)
    .await?;
    if !locked {
        bail!("une reconstruction est déjà en cours");
    }
    let result = rebuild_locked(pool, registry, allow_differences).await;
    // Fermer seule ne suffit pas : le serveur libère le verrou un peu après, et une
    // reconstruction lancée aussitôt le trouverait encore pris.
    let _ = sqlx::query_scalar!(
        r#"SELECT pg_advisory_unlock($1) AS "unlocked!""#,
        REBUILD_LOCK,
    )
    .fetch_one(&mut lock)
    .await;
    let _ = lock.close().await;
    result
}

async fn rebuild_locked(
    pool: &PgPool,
    registry: &[Interpreter],
    allow_differences: bool,
) -> Result<RebuildReport> {
    // Sans active, rien à comparer : c'est le worker qui l'établit, au démarrage de serve.
    let active = sqlx::query_scalar!("SELECT id FROM derived.generations WHERE state = 'active'")
        .fetch_optional(pool)
        .await?;
    if active.is_none() {
        bail!("aucune génération active : démarrer serve d'abord");
    }

    let building = begin_building(pool, registry).await?;
    for _ in 0..MAX_SWITCH_ATTEMPTS {
        catch_up(pool, registry, building).await?;
        match try_switch(pool, registry, building, allow_differences).await? {
            SwitchOutcome::Lagging => {}
            SwitchOutcome::Switched(comparison) => {
                remove_inactive(pool).await?;
                return Ok(RebuildReport {
                    generation: building,
                    switched: true,
                    comparison,
                });
            }
            // La génération reste en place pour être examinée ; la prochaine rebuild la remplace.
            SwitchOutcome::Differs(comparison) => {
                return Ok(RebuildReport {
                    generation: building,
                    switched: false,
                    comparison,
                });
            }
        }
    }
    bail!("le journal avance trop vite : bascule abandonnée, relancer rebuild")
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RetryReport {
    pub cleared: Vec<i64>,
    pub kept: Vec<i64>,
}

/// Réinterprète chaque enregistrement en erreur de l'active, une transaction par enregistrement.
/// Un enregistrement purgé depuis sort des erreurs : il ne produira plus rien.
pub async fn retry_errors(pool: &PgPool, registry: &[Interpreter]) -> Result<RetryReport> {
    let mut report = RetryReport::default();
    let mut after = 0;
    loop {
        let mut tx = pool.begin().await?;
        let generation = engine::lock_generation(&mut tx, Target::Active)
            .await?
            .context("aucune génération active")?;
        let raw = sqlx::query_as!(
            RawRecord,
            r#"SELECT r.seq, r.source_id, r.kind, r.schema_version, r.occurred_at,
                      r.utc_offset_minutes, r.request_id, r.payload::text AS payload
               FROM derived.interpretation_errors e
               JOIN core.records r ON r.seq = e.seq
               WHERE e.generation = $1 AND e.seq > $2
               ORDER BY e.seq LIMIT 1
               FOR UPDATE OF e"#,
            generation,
            after,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(raw) = raw else {
            return Ok(report);
        };
        after = raw.seq;

        // Sans interprète dans ce binaire : rien à tenter, l'erreur reste telle quelle.
        let Some(interpreter) = registry.iter().find(|i| i.kind == raw.kind) else {
            report.kept.push(raw.seq);
            continue;
        };
        match engine::apply(&mut tx, generation, interpreter, &raw).await? {
            Applied::Failed(message) => {
                engine::store_error(&mut tx, generation, raw.seq, interpreter.version, &message)
                    .await?;
                report.kept.push(raw.seq);
            }
            Applied::Purged | Applied::Written(_) => {
                sqlx::query!(
                    "DELETE FROM derived.interpretation_errors WHERE generation = $1 AND seq = $2",
                    generation,
                    raw.seq,
                )
                .execute(&mut *tx)
                .await?;
                report.cleared.push(raw.seq);
            }
        }
        tx.commit().await?;
    }
}
