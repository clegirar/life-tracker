//! L'état qu'une source rapporte pour la page d'état des trackers (`PUT /v1/sources/me/status`).
//! Un vocabulaire fermé, sans texte libre : ni message d'erreur, ni donnée personnelle, ni secret ne
//! peut s'y glisser. La web app traduit chaque code en phrase et en marche à suivre.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Ok,
    /// Il faut que j'agisse : une clé à renouveler, un accord à redonner.
    ActionRequired,
    /// Une panne qui devrait passer seule : service indisponible, limite atteinte.
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Le service refuse la clé : expirée ou révoquée.
    KeyRejected,
    /// Le service attend un premier accord (OAuth) : la page d'état propose de le donner.
    ConsentRequired,
    /// L'accord donné au service a expiré, ou va expirer : la page d'état propose de le renouveler.
    ConsentExpired,
    /// Le profil du service n'est pas public : Steam ne donne les succès que d'un profil dont les
    /// « Détails des jeux » sont publics. Le reste de la collecte continue.
    ProfilePrivate,
    RateLimited,
    ServiceUnavailable,
    /// L'API d'ingestion a refusé ou n'a pas répondu.
    IngestFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceStatus {
    pub state: State,
    /// Vide pour `ok`, exigée sinon.
    pub reason: Option<Reason>,
    /// L'échéance de la clé ou du jeton.
    pub expires_at: Option<DateTime<Utc>>,
    /// La prochaine passe prévue.
    pub next_run_at: Option<DateTime<Utc>>,
    /// La dernière collecte réussie : la dernière fois que le connecteur a reçu des données de son
    /// service et les a remises à l'API d'ingestion (pour YouTube, le dernier export).
    pub last_collected_at: Option<DateTime<Utc>>,
}

impl SourceStatus {
    pub fn ok() -> Self {
        Self {
            state: State::Ok,
            reason: None,
            expires_at: None,
            next_run_at: None,
            last_collected_at: None,
        }
    }

    pub fn failed(state: State, reason: Reason) -> Self {
        Self {
            state,
            reason: Some(reason),
            expires_at: None,
            next_run_at: None,
            last_collected_at: None,
        }
    }

    /// `ok` sans raison, les autres états avec.
    pub fn is_consistent(&self) -> bool {
        (self.state == State::Ok) == self.reason.is_none()
    }
}
