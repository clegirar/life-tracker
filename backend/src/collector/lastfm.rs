//! Le connecteur Last.fm : interroge le service et rend du brut, sans rien savoir du reste.
//! Ses erreurs ne portent que des codes : une URL contient la clé d'API et le nom d'utilisateur.

use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use reqwest::{Client, Response, StatusCode};
use serde_json::Value;

/// Le maximum de `user.getRecentTracks`.
pub const PAGE_SIZE: u32 = 200;
/// Ni le code ni la réponse n'indiquent de délai : on s'en tient à ce que la limite met à retomber.
pub const DEFAULT_RATE_LIMIT_DELAY: Duration = Duration::from_secs(300);
const RATE_LIMIT_CODE: i64 = 29;
/// Paramètres invalides ou ressource inconnue : pour `track.getInfo`, un titre que Last.fm n'a pas.
const NOT_FOUND_CODES: [i64; 2] = [6, 7];

#[derive(Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// Réponse 429 ou code 29 : jamais signalé, il suffit d'attendre.
    RateLimited { retry_after: Duration },
    /// Réponse d'erreur du service : son statut et son code, jamais son message.
    Failed { status: u16, code: Option<i64> },
    /// Rien d'exploitable : coupure, délai dépassé, réponse illisible.
    Unavailable { reason: &'static str },
}

#[derive(Clone)]
pub struct Lastfm {
    pub http: Client,
    pub url: String,
    pub api_key: String,
    pub user: String,
}

/// Une page d'écoutes, chaque élément tel que renvoyé.
#[derive(Debug, PartialEq)]
pub struct RecentPage {
    pub total: u64,
    pub total_pages: u32,
    pub tracks: Vec<Value>,
}

/// Toujours un motif fixe : l'affichage d'une erreur de reqwest citerait l'URL.
pub fn network_reason(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "timeout"
    } else if err.is_connect() {
        "connect"
    } else if err.is_decode() || err.is_body() {
        "body"
    } else {
        "request"
    }
}

fn retry_after(response: &Response) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
        .map_or(DEFAULT_RATE_LIMIT_DELAY, Duration::from_secs)
}

/// Un entier, que Last.fm l'écrive en texte ou en nombre.
fn integer(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::String(text) => text.trim().parse().ok(),
        other => other.as_u64(),
    }
}

/// `track` vaut un tableau, un objet s'il n'y a qu'un élément, ou rien du tout.
pub fn parse_recent(body: &Value) -> Option<RecentPage> {
    let recent = body.get("recenttracks")?.as_object()?;
    // Une liste vide peut porter ses compteurs à la racine et un `#text` à la place des titres.
    let attr = recent.get("@attr").unwrap_or(&Value::Null);
    let field = |name: &str| integer(attr.get(name).or_else(|| recent.get(name)));
    let tracks = match recent.get("track") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(tracks)) => tracks.clone(),
        Some(track @ Value::Object(_)) => vec![track.clone()],
        Some(_) => return None,
    };
    Some(RecentPage {
        total: field("total").unwrap_or(0),
        total_pages: field("totalPages")
            .and_then(|pages| u32::try_from(pages).ok())
            .unwrap_or(0),
        tracks,
    })
}

impl Lastfm {
    /// Le corps JSON d'une réponse réussie, ou la raison de son échec. Last.fm peut signaler une
    /// erreur sous n'importe quel statut, 200 compris : c'est le champ `error` qui compte.
    async fn call(&self, params: &[(&str, String)]) -> Result<Value, ServiceError> {
        let response = self
            .http
            .get(&self.url)
            .query(&[("api_key", self.api_key.as_str()), ("format", "json")])
            .query(params)
            .send()
            .await
            .map_err(|err| ServiceError::Unavailable {
                reason: network_reason(&err),
            })?;
        let status = response.status();
        let delay = retry_after(&response);
        let body: Result<Value, _> = response.json().await;
        let code = body
            .as_ref()
            .ok()
            .and_then(|b| b.get("error"))
            .and_then(Value::as_i64);
        if status == StatusCode::TOO_MANY_REQUESTS || code == Some(RATE_LIMIT_CODE) {
            return Err(ServiceError::RateLimited { retry_after: delay });
        }
        if !status.is_success() || code.is_some() {
            return Err(ServiceError::Failed {
                status: status.as_u16(),
                code,
            });
        }
        body.map_err(|err| ServiceError::Unavailable {
            reason: network_reason(&err),
        })
    }

    /// Du plus récent au plus ancien. `from` et `to` en secondes UTC.
    pub async fn recent_tracks(
        &self,
        from: Option<i64>,
        to: i64,
        page: u32,
    ) -> Result<RecentPage, ServiceError> {
        let mut params = vec![
            ("method", "user.getrecenttracks".to_owned()),
            ("user", self.user.clone()),
            ("limit", PAGE_SIZE.to_string()),
            ("page", page.to_string()),
            ("to", to.to_string()),
        ];
        if let Some(from) = from {
            params.push(("from", from.to_string()));
        }
        let body = self.call(&params).await?;
        parse_recent(&body).ok_or(ServiceError::Unavailable { reason: "shape" })
    }

    /// La fiche d'un titre, ou `None` s'il est inconnu de Last.fm. `autocorrect` : plus de chances
    /// d'avoir une durée sous une orthographe voisine.
    pub async fn track_info(
        &self,
        artist: &str,
        track: &str,
    ) -> Result<Option<Value>, ServiceError> {
        let params = [
            ("method", "track.getInfo".to_owned()),
            ("artist", artist.to_owned()),
            ("track", track.to_owned()),
            ("autocorrect", "1".to_owned()),
        ];
        match self.call(&params).await {
            Ok(body) => Ok(Some(body)),
            Err(ServiceError::Failed {
                code: Some(code), ..
            }) if NOT_FOUND_CODES.contains(&code) => Ok(None),
            Err(other) => Err(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_single_track_comes_as_an_object() {
        let body = json!({"recenttracks": {"track": {"name": "T"},
            "@attr": {"total": "1", "totalPages": "1", "page": "1"}}});
        let page = parse_recent(&body).unwrap();
        assert_eq!(page.tracks, vec![json!({"name": "T"})]);
        assert_eq!((page.total, page.total_pages), (1, 1));
    }

    #[test]
    fn an_empty_list_has_several_shapes() {
        for body in [
            json!({"recenttracks": {"track": [], "@attr": {"total": "0", "totalPages": "0"}}}),
            json!({"recenttracks": {"#text": "\n", "user": "x", "total": "0"}}),
            json!({"recenttracks": {}}),
        ] {
            let page = parse_recent(&body).unwrap();
            assert!(page.tracks.is_empty(), "{body}");
            assert_eq!(page.total_pages, 0);
        }
    }

    #[test]
    fn anything_else_is_not_a_list() {
        for body in [
            json!({}),
            json!({"recenttracks": "x"}),
            json!({"recenttracks": {"track": "x"}}),
        ] {
            assert_eq!(parse_recent(&body), None, "{body}");
        }
    }
}
