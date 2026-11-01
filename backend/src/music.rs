//! L'identité d'un titre sans catalogue : artiste et titre normalisés (voir l'architecture).
//! Fonctions pures, appelées par les interprètes ; les changer demande un `admin rebuild`.

use unicode_normalization::UnicodeNormalization;

/// Sépare artiste et titre dans une clé. `fold` réduit tout blanc à une espace : il ne peut pas
/// apparaître dans un nom, donc deux couples différents ne donnent jamais la même clé.
const KEY_SEPARATOR: char = '\t';

/// Une mention de version, mot entier. Un remix ou un mix n'en est pas une : voir `is_version`.
const VERSION_WORDS: &[&str] = &[
    "acoustic",
    "acoustique",
    "anniversary",
    "bonus",
    "clean",
    "deluxe",
    "demo",
    "direct",
    "edit",
    "edition",
    "édition",
    "explicit",
    "live",
    "mono",
    "remaster",
    "remastered",
    "remasterisé",
    "remasterisée",
    "single",
    "stereo",
    "stéréo",
    "version",
];
const FEATURE_WORDS: &[&str] = &["feat", "ft", "featuring", "with", "avec"];

/// Minuscules, formes Unicode et ponctuation typographique unifiées, blancs réduits.
pub fn fold(name: &str) -> String {
    let unified: String = name
        .nfkc()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{02BC}' | '`' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{2010}'..='\u{2015}' => '-',
            other => other,
        })
        .collect();
    let lower = unified.to_lowercase();
    lower.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
}

fn is_version(text: &str) -> bool {
    // Un remix est une autre œuvre, avec sa durée et sa propre découverte.
    if words(text).any(|w| matches!(w, "remix" | "rmx" | "mix")) {
        return false;
    }
    words(text).any(|w| VERSION_WORDS.contains(&w))
}

fn is_feature(text: &str) -> bool {
    words(text)
        .next()
        .is_some_and(|w| FEATURE_WORDS.contains(&w))
}

/// Retire les groupes entre parenthèses ou crochets que `drop` désigne, où qu'ils soient.
fn remove_groups(text: &str, drop: impl Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(['(', '[']) {
        let close = if rest[start..].starts_with('(') {
            ')'
        } else {
            ']'
        };
        let Some(length) = rest[start..].find(close) else {
            break;
        };
        let inner = &rest[start + 1..start + length];
        out.push_str(&rest[..start]);
        if !drop(inner) {
            out.push_str(&rest[start..=start + length]);
        }
        rest = &rest[start + length + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Coupe à « feat. X », hors parenthèses. Les formes avec un point d'abord : « Little Feat feat. X »
/// reste « Little Feat ». Un mot sans suite n'est pas une mention, le nom « Little Feat » reste entier.
fn cut_bare_feature(text: &str) -> &str {
    for marker in [" feat. ", " ft. ", " featuring ", " feat ", " ft "] {
        if let Some(position) = text.find(marker) {
            return &text[..position];
        }
    }
    text
}

fn cut_version_suffix(text: &str) -> &str {
    let mut current = text;
    while let Some((head, tail)) = current.rsplit_once(" - ") {
        if !is_version(tail) {
            break;
        }
        current = head;
    }
    current
}

pub fn artist_key(artist: &str) -> String {
    let folded = fold(artist);
    let without = remove_groups(&folded, is_feature);
    let key = cut_bare_feature(&without).trim();
    // Un nom réduit à sa mention ne perd jamais tout : la clé ne doit pas être vide.
    if key.is_empty() {
        folded
    } else {
        key.to_owned()
    }
}

fn title_key(title: &str) -> String {
    let folded = fold(title);
    let without = remove_groups(&folded, |inner| is_feature(inner) || is_version(inner));
    let key = cut_version_suffix(cut_bare_feature(&without)).trim();
    if key.is_empty() {
        folded
    } else {
        key.to_owned()
    }
}

/// Artiste et titre : deux versions d'un même titre ont la même clé.
pub fn track_key(artist: &str, title: &str) -> String {
    format!("{}{KEY_SEPARATOR}{}", artist_key(artist), title_key(title))
}

/// Artiste et album, sinon tous les « Greatest Hits » seraient confondus. Le nom de l'album reste
/// tel quel : « Album » et « Album - Single » restent deux albums (limite connue, sans catalogue).
pub fn album_key(artist: &str, album: &str) -> String {
    format!("{}{KEY_SEPARATOR}{}", artist_key(artist), fold(album))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        for (input, expected) in [
            // Casse, accents, blancs, ponctuation.
            ("One More Time", "one more time"),
            ("ONE  MORE\tTIME ", "one more time"),
            ("La Femme d\u{2019}Argent", "la femme d'argent"),
            ("Beyonce\u{0301}", "beyoncé"),
            ("Café", "café"),
            ("Song \u{2013} Live", "song"),
            // Mentions de featuring.
            ("Song (feat. Someone)", "song"),
            ("Song [feat. Someone]", "song"),
            ("Song (Feat Someone)", "song"),
            ("Song (ft. Someone)", "song"),
            ("Song (featuring Someone)", "song"),
            ("Song (with Someone)", "song"),
            ("Song (avec Quelqu'un)", "song"),
            ("Song feat. Someone", "song"),
            ("Song ft. Someone & Other", "song"),
            ("Song featuring Someone", "song"),
            ("Song FT Someone", "song"),
            // Mentions de version.
            ("Song (Live)", "song"),
            ("Song (Live at Wembley)", "song"),
            ("Song - Live", "song"),
            ("Song - Live at Wembley 1986", "song"),
            ("Song - Remastered", "song"),
            ("Song - 2011 Remaster", "song"),
            ("Song (2011 Remaster)", "song"),
            ("Song [Remastered 2009]", "song"),
            ("Song (Radio Edit)", "song"),
            ("Song - Radio Edit", "song"),
            ("Song (Acoustic Version)", "song"),
            ("Song (Deluxe Edition)", "song"),
            ("Song (Bonus Track)", "song"),
            ("Song (Mono)", "song"),
            ("Song (Explicit)", "song"),
            ("Song (Taylor's Version)", "song"),
            // Plusieurs mentions à la fois.
            ("Song (feat. Someone) - Remastered 2011", "song"),
            ("Song (feat. Someone) [Live]", "song"),
            ("Song - Live - Remastered", "song"),
            ("Song feat. Someone (Live)", "song"),
            // Ce qui n'est pas une mention reste.
            ("Song (Daft Punk Remix)", "song (daft punk remix)"),
            ("Song - Extended Mix", "song - extended mix"),
            ("Song (Live) (Club Remix)", "song (club remix)"),
            ("Live and Let Die", "live and let die"),
            ("Long Live", "long live"),
            ("(I Just) Died in Your Arms", "(i just) died in your arms"),
            ("Stay - Alive", "stay - alive"),
            ("With or Without You", "with or without you"),
            ("Aftermath", "aftermath"),
            ("Left Behind", "left behind"),
            ("Song (Part 2)", "song (part 2)"),
            ("Feat", "feat"),
            ("Song (inachevé", "song (inachevé"),
            // Un titre réduit à sa mention ne devient pas vide.
            ("(Live)", "(live)"),
            ("Live", "live"),
        ] {
            assert_eq!(title_key(input), expected, "{input}");
        }
    }

    #[test]
    fn artists() {
        for (input, expected) in [
            ("Daft Punk", "daft punk"),
            ("DAFT PUNK", "daft punk"),
            ("Daft Punk feat. Pharrell Williams", "daft punk"),
            ("Daft Punk ft. Pharrell Williams", "daft punk"),
            ("Daft Punk (feat. Pharrell Williams)", "daft punk"),
            ("Daft Punk featuring Pharrell Williams", "daft punk"),
            // Un artiste combiné reste distinct (limite connue de l'architecture).
            ("Simon & Garfunkel", "simon & garfunkel"),
            ("Little Feat", "little feat"),
            ("Little Feat feat. Someone", "little feat"),
            // Les versions ne concernent que les titres.
            ("Live", "live"),
            ("The Clean", "the clean"),
            ("Bonobo (Live)", "bonobo (live)"),
        ] {
            assert_eq!(artist_key(input), expected, "{input}");
        }
    }

    #[test]
    fn two_versions_are_the_same_track_but_not_the_same_album() {
        assert_eq!(
            track_key("Daft Punk", "Veridis Quo"),
            track_key("DAFT PUNK feat. Someone", "Veridis Quo - Remastered 2011"),
        );
        assert_ne!(track_key("A", "B C"), track_key("A B", "C"));
        assert_ne!(album_key("A", "Hits"), album_key("B", "Hits"));
        assert_ne!(album_key("A", "Album"), album_key("A", "Album - Single"));
        assert_eq!(album_key("a", "ALBUM "), album_key("A", "album"));
    }
}
