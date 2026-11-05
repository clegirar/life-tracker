//! Le relais d'un accord OAuth entre un connecteur et la web app (voir le journal des décisions de
//! l'architecture, 5 nov. 2026). Le connecteur prépare la demande (état aléatoire et vérificateur PKCE,
//! qu'il garde) et ne confie au backend que l'URL de consentement ; Google renvoie le code à la web app,
//! sous session, qui le transmet ici ; le connecteur vient le chercher, une seule fois, et l'échange
//! lui-même. Le code seul ne sert à rien sans le vérificateur.
//!
//! Tout reste en mémoire, jamais dans la base, le journal ni les logs : un redémarrage du backend perd
//! la demande, que le connecteur renvoie à son passage suivant. Une demande par source, la dernière.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Une demande sans réponse s'efface au bout d'une heure ; le connecteur la renouvelle à chaque passage.
pub const REQUEST_TTL: Duration = Duration::from_secs(3600);
/// Un code reçu attend le connecteur dix minutes au plus.
pub const CODE_TTL: Duration = Duration::from_secs(600);

const MIN_STATE_CHARS: usize = 43;
const MAX_STATE_CHARS: usize = 128;
pub const MAX_URL_CHARS: usize = 2048;
const MAX_CODE_CHARS: usize = 512;
const MAX_ERROR_CHARS: usize = 64;
/// Le seul hôte de consentement accepté : le backend n'affiche jamais un lien vers ailleurs.
const CONSENT_HOST: &str = "accounts.google.com";

/// Ce que Google a renvoyé : un code, ou le motif d'un refus (`access_denied`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Code(String),
    Error(String),
}

struct Pending {
    url: String,
    state: String,
    expires: Instant,
}

struct Delivered {
    outcome: Outcome,
    /// L'état de la demande à laquelle il répond.
    state: String,
    expires: Instant,
}

#[derive(Default)]
pub struct Store {
    pending: HashMap<i16, Pending>,
    delivered: HashMap<i16, Delivered>,
}

/// Une comparaison en temps constant, à longueur égale : l'état est le seul secret d'une demande.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// Les caractères non réservés d'une URL (RFC 3986), 43 à 128 : assez d'aléa, rien à échapper.
pub fn valid_state(state: &str) -> bool {
    (MIN_STATE_CHARS..=MAX_STATE_CHARS).contains(&state.len())
        && state
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// Une URL de consentement de Google, en https, qui porte cet état et lui seul.
pub fn valid_url(url: &str, state: &str) -> bool {
    if url.len() > MAX_URL_CHARS {
        return false;
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let states: Vec<_> = parsed
        .query_pairs()
        .filter(|(name, _)| name == "state")
        .collect();
    parsed.scheme() == "https"
        && parsed.host_str() == Some(CONSENT_HOST)
        && parsed.port().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.fragment().is_none()
        && matches!(&states[..], [(_, value)] if value == state)
}

/// Un code d'autorisation : des caractères visibles, sans blanc.
pub fn valid_code(code: &str) -> bool {
    (1..=MAX_CODE_CHARS).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_graphic())
}

/// Un code d'erreur OAuth (`access_denied`…) : minuscules et `_`, jamais de texte libre.
pub fn valid_error(error: &str) -> bool {
    (1..=MAX_ERROR_CHARS).contains(&error.len())
        && error.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

impl Store {
    fn purge(&mut self, now: Instant) {
        self.pending.retain(|_, p| p.expires > now);
        self.delivered.retain(|_, d| d.expires > now);
    }

    /// Remplace la demande de la source. Un code déjà reçu pour un autre état ne vaut plus rien ; pour
    /// le même état (le connecteur renouvelle sa demande), il attend toujours.
    pub fn request(&mut self, source: i16, url: String, state: String, now: Instant) {
        self.purge(now);
        if self
            .delivered
            .get(&source)
            .is_some_and(|d| !same(&d.state, &state))
        {
            self.delivered.remove(&source);
        }
        self.pending.insert(
            source,
            Pending {
                url,
                state,
                expires: now + REQUEST_TTL,
            },
        );
    }

    /// L'URL de consentement, tant que la demande attend.
    pub fn url(&mut self, source: i16, now: Instant) -> Option<String> {
        self.purge(now);
        self.pending.get(&source).map(|p| p.url.clone())
    }

    /// Le retour de Google, rattaché à sa demande par son état. `false` : aucune demande n'attend cet
    /// état (inconnu ou expiré). La demande est close : son URL disparaît de la page d'état.
    pub fn deliver(&mut self, state: &str, outcome: Outcome, now: Instant) -> bool {
        self.purge(now);
        // Toutes les demandes sont comparées, sans s'arrêter à la première : le temps ne dit rien.
        let mut found = None;
        for (source, pending) in &self.pending {
            if same(&pending.state, state) {
                found = Some(*source);
            }
        }
        let Some(source) = found else {
            return false;
        };
        let pending = self.pending.remove(&source).expect("trouvée ci-dessus");
        self.delivered.insert(
            source,
            Delivered {
                outcome,
                state: pending.state,
                expires: now + CODE_TTL,
            },
        );
        true
    }

    /// Le retour reçu pour la source, une seule fois. Le prendre clôt aussi une demande renouvelée
    /// entre-temps par le connecteur.
    pub fn take(&mut self, source: i16, now: Instant) -> Option<Outcome> {
        self.purge(now);
        let delivered = self.delivered.remove(&source)?;
        self.pending.remove(&source);
        Some(delivered.outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = "abcdefghijklmnopqrstuvwxyz0123456789-._~ABCDEFG";

    fn url(state: &str) -> String {
        format!("https://accounts.google.com/o/oauth2/v2/auth?client_id=x&state={state}")
    }

    #[test]
    fn only_a_consent_url_of_google_with_its_state_is_accepted() {
        assert!(valid_state(STATE));
        assert!(valid_url(&url(STATE), STATE));
        for bad in [
            "short",
            &"a".repeat(129),
            &format!("{STATE} "),
            &format!("{STATE}/"),
        ] {
            assert!(!valid_state(bad), "{bad}");
        }
        for bad in [
            url(STATE).replace("https", "http"),
            url(STATE).replace("accounts.google.com", "accounts.google.com.evil.example"),
            url(STATE).replace("accounts.google.com", "evil.example"),
            url(STATE).replace("accounts.google.com", "accounts.google.com:8443"),
            url(STATE).replace("accounts.google.com", "user@accounts.google.com"),
            url("autre"),
            format!("{}&state={STATE}", url(STATE)),
            format!("{}#x", url(STATE)),
            "https://accounts.google.com/o/oauth2/v2/auth".to_owned(),
            format!("{}&x={}", url(STATE), "a".repeat(MAX_URL_CHARS)),
            "pas une url".to_owned(),
        ] {
            assert!(!valid_url(&bad, STATE), "{bad}");
        }
        assert!(valid_code("4/0AbCdEf-gh_ij.kl"));
        assert!(!valid_code(""));
        assert!(!valid_code("a b"));
        assert!(!valid_code(&"a".repeat(513)));
        assert!(valid_error("access_denied"));
        assert!(!valid_error("Access denied"));
        assert!(!valid_error(""));
    }

    #[test]
    fn a_code_is_delivered_by_state_and_taken_once() {
        let now = Instant::now();
        let mut store = Store::default();
        store.request(1, url(STATE), STATE.to_owned(), now);
        assert_eq!(store.url(1, now), Some(url(STATE)));
        assert_eq!(store.url(2, now), None);
        assert_eq!(store.take(1, now), None);
        assert!(!store.deliver("inconnu", Outcome::Code("c".to_owned()), now));
        assert!(store.deliver(STATE, Outcome::Code("c".to_owned()), now));
        assert_eq!(store.url(1, now), None, "la demande est close");
        assert!(
            !store.deliver(STATE, Outcome::Code("d".to_owned()), now),
            "un état ne sert qu'une fois"
        );
        assert_eq!(store.take(1, now), Some(Outcome::Code("c".to_owned())));
        assert_eq!(store.take(1, now), None, "une seule fois");
    }

    #[test]
    fn a_request_and_a_code_expire() {
        let now = Instant::now();
        let mut store = Store::default();
        store.request(1, url(STATE), STATE.to_owned(), now);
        assert_eq!(store.url(1, now + REQUEST_TTL), None);
        assert!(!store.deliver(STATE, Outcome::Code("c".to_owned()), now + REQUEST_TTL));

        store.request(1, url(STATE), STATE.to_owned(), now);
        assert!(store.deliver(STATE, Outcome::Error("access_denied".to_owned()), now));
        assert_eq!(store.take(1, now + CODE_TTL), None);
    }

    #[test]
    fn a_renewed_request_keeps_its_code_and_a_new_one_drops_it() {
        let now = Instant::now();
        let mut store = Store::default();
        store.request(1, url(STATE), STATE.to_owned(), now);
        assert!(store.deliver(STATE, Outcome::Code("c".to_owned()), now));
        // Le connecteur renouvelle sa demande avant d'avoir vu le code : le code attend toujours, et le
        // prendre clôt la demande renouvelée.
        store.request(1, url(STATE), STATE.to_owned(), now);
        assert_eq!(store.take(1, now), Some(Outcome::Code("c".to_owned())));
        assert_eq!(store.url(1, now), None);

        store.request(1, url(STATE), STATE.to_owned(), now);
        assert!(store.deliver(STATE, Outcome::Code("c".to_owned()), now));
        let other = STATE.replace('a', "b");
        store.request(1, url(&other), other.clone(), now);
        store.request(1, url(&other), other, now);
        assert_eq!(store.take(1, now), None, "le code d'une autre demande");
    }
}
