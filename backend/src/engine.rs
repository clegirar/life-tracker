//! Le moteur d'interprétation : avance dans le journal par type, une transaction par lot, et un point
//! de reprise par enregistrement : chacun écrit tous ses événements ou aucun. Les événements, les
//! erreurs et le curseur sont validés ensemble : une coupure ne perd ni ne double rien, et repart au
//! curseur. Tourne en tracker_app (serve) comme en tracker_admin (rebuild, retry-errors).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::{Connection, PgPool, Postgres, Transaction};
use tokio::sync::watch;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::interpreters::{Interpreter, Output, Record};

/// Le lot du worker et de la reconstruction : gros gain de débit (une seule écriture disque par lot)
/// sans retarder les autres types de plus que le temps de ce lot, moins d'une seconde sur le NAS
/// (mesure dans l'architecture).
pub const WORKER_BATCH: i64 = 100;

/// La génération sur laquelle travailler.
#[derive(Clone, Copy, Debug)]
pub enum Target {
    Active,
    /// Jusqu'à un numéro fixé : un journal qui avance plus vite ne retient pas la construction.
    Building {
        id: i32,
        up_to: i64,
    },
}

/// Ce que le journal dit d'un enregistrement ; `payload` est vide après une purge.
pub(crate) struct RawRecord {
    pub seq: i64,
    pub source_id: i16,
    pub kind: String,
    pub schema_version: i16,
    pub occurred_at: DateTime<Utc>,
    pub utc_offset_minutes: Option<i16>,
    pub request_id: Option<Uuid>,
    pub payload: Option<String>,
}

pub(crate) enum Applied {
    Purged,
    Written(usize),
    Failed(String),
}

pub fn versions_json(registry: &[Interpreter]) -> String {
    let versions: Map<String, Value> = registry
        .iter()
        .map(|i| (i.kind.to_owned(), i.version.into()))
        .collect();
    Value::Object(versions).to_string()
}

/// Crée la génération active d'une base neuve. L'index unique départage les démarrages concurrents.
pub async fn ensure_active(pool: &PgPool, registry: &[Interpreter]) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO derived.generations (state, activated_at, interpreter_versions)
         VALUES ('active', now(), $1::text::jsonb)
         ON CONFLICT DO NOTHING",
        versions_json(registry),
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Verrou partagé sur la génération, tenu jusqu'à la fin de la transaction : la bascule attend le
/// travail en cours, puis le travail suivant voit la nouvelle active.
pub(crate) async fn lock_generation(
    tx: &mut Transaction<'_, Postgres>,
    target: Target,
) -> Result<Option<i32>, sqlx::Error> {
    let building = match target {
        Target::Active => None,
        Target::Building { id, .. } => Some(id),
    };
    sqlx::query_scalar!(
        "SELECT id FROM derived.generations
         WHERE ($1::integer IS NULL AND state = 'active') OR (id = $1 AND state = 'building')
         FOR SHARE",
        building,
    )
    .fetch_optional(&mut **tx)
    .await
}

fn to_record(raw: &RawRecord, payload: &str) -> Option<Record> {
    Some(Record {
        seq: raw.seq,
        source_id: raw.source_id,
        schema_version: raw.schema_version,
        occurred_at: raw.occurred_at,
        utc_offset_minutes: raw.utc_offset_minutes,
        payload: serde_json::from_str(payload).ok()?,
    })
}

/// Interprète un enregistrement et écrit sa sortie, entière ou pas du tout. `Err` est une panne :
/// la transaction est perdue et l'enregistrement sera repris.
pub(crate) async fn apply(
    tx: &mut Transaction<'_, Postgres>,
    generation: i32,
    interpreter: &Interpreter,
    raw: &RawRecord,
) -> Result<Applied, sqlx::Error> {
    let Some(payload) = &raw.payload else {
        return Ok(Applied::Purged);
    };
    if raw.schema_version != interpreter.schema_version {
        return Ok(Applied::Failed(format!(
            "unsupported schema_version {}",
            raw.schema_version
        )));
    }
    let Some(record) = to_record(raw, payload) else {
        return Ok(Applied::Failed("unreadable payload".to_owned()));
    };
    let outputs = match catch_unwind(AssertUnwindSafe(|| (interpreter.interpret)(&record))) {
        Ok(Ok(outputs)) => outputs,
        Ok(Err(failure)) => return Ok(Applied::Failed(failure.0.to_owned())),
        Err(_) => return Ok(Applied::Failed("interpreter panicked".to_owned())),
    };

    let count = outputs.len();
    let mut savepoint = (**tx).begin().await?;
    match write_outputs(
        &mut savepoint,
        generation,
        interpreter.version,
        &record,
        outputs,
    )
    .await
    {
        Ok(()) => {
            savepoint.commit().await?;
            Ok(Applied::Written(count))
        }
        Err(Refused::Permanent(message)) => {
            savepoint.rollback().await?;
            Ok(Applied::Failed(message))
        }
        Err(Refused::Outage(err)) => Err(err),
    }
}

enum Refused {
    /// La sortie elle-même est invalide : la rejouer échouerait pareil.
    Permanent(String),
    Outage(sqlx::Error),
}

impl From<sqlx::Error> for Refused {
    fn from(err: sqlx::Error) -> Self {
        // Données (22) et contraintes (23). Le code seul : le message pourrait citer des valeurs.
        // 54000 : un nom trop long pour l'index des éléments. Rejouer échouerait pareil, et le prendre
        // pour une panne bloquerait le worker en boucle. Ce code seul de la classe 54 : les autres
        // limites disent un défaut de la requête, pas de la sortie.
        match err.as_database_error().and_then(|db| db.code()) {
            Some(code) if code.starts_with("22") || code.starts_with("23") || code == "54000" => {
                Self::Permanent(format!("output refused by the database ({code})"))
            }
            _ => Self::Outage(err),
        }
    }
}

async fn write_outputs(
    tx: &mut Transaction<'_, Postgres>,
    generation: i32,
    version: i32,
    record: &Record,
    outputs: Vec<Output>,
) -> Result<(), Refused> {
    let mut ordinal = 0i16;
    for output in outputs {
        match output {
            Output::Event(event) => {
                sqlx::query!(
                    "INSERT INTO derived.events (generation, seq, ordinal, metric, source_id,
                       occurred_at, utc_offset_minutes, value, details, interpreter_version)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::numeric, $9::text::jsonb, $10)",
                    generation,
                    record.seq,
                    ordinal,
                    event.metric,
                    record.source_id,
                    event.occurred_at,
                    event.utc_offset_minutes,
                    event.value,
                    event.details.to_string(),
                    version,
                )
                .execute(&mut **tx)
                .await?;
                for item in &event.items {
                    sqlx::query!(
                        "INSERT INTO derived.event_items (generation, seq, ordinal, level,
                           position, item_key, display, metric, occurred_at)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                        generation,
                        record.seq,
                        ordinal,
                        item.level,
                        item.position,
                        item.item_key,
                        item.display,
                        event.metric,
                        event.occurred_at,
                    )
                    .execute(&mut **tx)
                    .await?;
                }
                ordinal = ordinal
                    .checked_add(1)
                    .ok_or_else(|| Refused::Permanent("too many events".to_owned()))?;
            }
            Output::Cancellation { source, key } => {
                // Les sources ne sont ni supprimées ni renommées : le nom se résout toujours pareil.
                let written = sqlx::query!(
                    "INSERT INTO derived.cancellations
                       (generation, cancel_seq, target_source_id, target_key)
                     SELECT $1, $2, s.id, $4 FROM core.sources s WHERE s.name = $3",
                    generation,
                    record.seq,
                    source,
                    key,
                )
                .execute(&mut **tx)
                .await?;
                if written.rows_affected() == 0 {
                    return Err(Refused::Permanent("unknown target source".to_owned()));
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn store_error(
    tx: &mut Transaction<'_, Postgres>,
    generation: i32,
    seq: i64,
    version: i32,
    message: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO derived.interpretation_errors (generation, seq, interpreter_version, error)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (generation, seq) DO UPDATE
         SET interpreter_version = EXCLUDED.interpreter_version, error = EXCLUDED.error,
             failed_at = now()",
        generation,
        seq,
        version,
        message,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

enum Step {
    NoGeneration,
    Processed(usize),
}

/// Traite les prochains enregistrements de ce type, `limit` au plus. Le verrou sur le curseur sérialise
/// les workers. Une panne perd tout le lot, curseur compris : il sera repris en entier. `apply` isole
/// chaque enregistrement par un point de sauvegarde, et les messages n'en citent jamais le contenu.
async fn process_batch(
    pool: &PgPool,
    interpreter: &Interpreter,
    target: Target,
    limit: i64,
) -> Result<Step, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let Some(generation) = lock_generation(&mut tx, target).await? else {
        return Ok(Step::NoGeneration);
    };
    sqlx::query!(
        "INSERT INTO derived.cursors (generation, kind) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        generation,
        interpreter.kind,
    )
    .execute(&mut *tx)
    .await?;
    let last_seq = sqlx::query_scalar!(
        "SELECT last_seq FROM derived.cursors WHERE generation = $1 AND kind = $2 FOR UPDATE",
        generation,
        interpreter.kind,
    )
    .fetch_one(&mut *tx)
    .await?;
    let up_to = match target {
        Target::Active => i64::MAX,
        Target::Building { up_to, .. } => up_to,
    };
    let records = sqlx::query_as!(
        RawRecord,
        "SELECT seq, source_id, kind, schema_version, occurred_at, utc_offset_minutes, request_id,
                payload::text AS payload
         FROM core.records WHERE kind = $1 AND seq > $2 AND seq <= $3 ORDER BY seq LIMIT $4",
        interpreter.kind,
        last_seq,
        up_to,
        limit,
    )
    .fetch_all(&mut *tx)
    .await?;
    let Some(last) = records.last().map(|raw| raw.seq) else {
        tx.commit().await?;
        return Ok(Step::Processed(0));
    };

    let mut outcomes = Vec::with_capacity(records.len());
    for raw in &records {
        let applied = apply(&mut tx, generation, interpreter, raw).await?;
        if let Applied::Failed(message) = &applied {
            store_error(&mut tx, generation, raw.seq, interpreter.version, message).await?;
        }
        outcomes.push(applied);
    }
    sqlx::query!(
        "UPDATE derived.cursors SET last_seq = $3 WHERE generation = $1 AND kind = $2",
        generation,
        interpreter.kind,
        last,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // Des identifiants seulement. Le message d'erreur reste en base.
    let kind = interpreter.kind;
    for (raw, applied) in records.iter().zip(outcomes) {
        let (seq, request_id) = (raw.seq, raw.request_id);
        match applied {
            Applied::Purged => info!(seq, kind, ?request_id, "purged record skipped"),
            Applied::Written(outputs) => {
                info!(seq, kind, ?request_id, outputs, "record interpreted");
            }
            Applied::Failed(_) => warn!(seq, kind, ?request_id, "record in error"),
        }
    }
    Ok(Step::Processed(records.len()))
}

/// Un enregistrement au plus par type ; renvoie le nombre de pas utiles.
pub async fn pass(
    pool: &PgPool,
    registry: &[Interpreter],
    target: Target,
) -> Result<usize, sqlx::Error> {
    pass_batch(pool, registry, target, 1).await
}

/// Un lot au plus par type, chacun à son tour : un gros import ne fait attendre un café que le temps
/// d'un lot. `Target::Active` crée la génération d'une base neuve ; une génération en construction
/// disparue est une erreur.
pub async fn pass_batch(
    pool: &PgPool,
    registry: &[Interpreter],
    target: Target,
    limit: i64,
) -> Result<usize, sqlx::Error> {
    let mut progress = 0;
    for interpreter in registry {
        match process_batch(pool, interpreter, target, limit).await? {
            Step::Processed(count) => progress += count,
            Step::NoGeneration => match target {
                Target::Active => {
                    ensure_active(pool, registry).await?;
                    progress += 1;
                }
                Target::Building { .. } => return Err(sqlx::Error::RowNotFound),
            },
        }
    }
    Ok(progress)
}

/// Délai avant le n-ième nouvel essai : 1 s, 2 s, 4 s… jusqu'à une minute.
pub fn backoff(failures: u32) -> Duration {
    Duration::from_secs(1u64 << failures.saturating_sub(1).min(6)).min(Duration::from_secs(60))
}

/// La tâche de fond de `serve`. Une panne ne produit jamais d'erreur d'interprétation : la
/// transaction est perdue, l'enregistrement sera repris après un délai croissant.
pub async fn run(
    pool: PgPool,
    registry: &'static [Interpreter],
    poll: Duration,
    mut stop: watch::Receiver<bool>,
) {
    let mut failures = 0;
    while !*stop.borrow() {
        let delay = match pass_batch(&pool, registry, Target::Active, WORKER_BATCH).await {
            Ok(progress) => {
                failures = 0;
                if progress > 0 {
                    continue;
                }
                poll
            }
            Err(err) => {
                failures += 1;
                match err.as_database_error() {
                    Some(db) => error!(code = db.code().as_deref(), failures, "database error"),
                    None => error!(error = %err, failures, "database unavailable"),
                }
                backoff(failures)
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = stop.changed() => {}
        }
    }
}
