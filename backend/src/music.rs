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

/// Les vrais groupes dont le nom contient un séparateur : seul ou dans un crédit, un nom de la liste
/// n'est jamais coupé. Une clé repliée (`fold`) par ligne ; en ajouter une demande un `admin rebuild`.
#[rustfmt::skip]
const UNSPLIT_ARTISTS: &[&str] = &[
    "bigflo & oli",
    "earth, wind & fire",
    "kool & the gang",
    "simon & garfunkel",
    "tyler, the creator",
];

/// Entre deux artistes d'un crédit, en minuscules ASCII. « / » et « + » seulement entre deux espaces,
/// la virgule seulement suivie d'une espace : « AC/DC » ou « 10,000 Maniacs » restent entiers.
const LIST_SEPARATORS: &[&str] = &["&", ", ", " / ", " + ", " x ", " et ", " and "];
/// Les mentions d'invité hors parenthèses. Les formes avec un point d'abord : « Little Feat feat. X ».
const MARKED_FEATURES: &[&str] = &[" feat. ", " ft. ", " featuring "];
/// Sans point, seulement en l'absence des précédentes : un mot sans suite n'est pas une mention.
const BARE_FEATURES: &[&str] = &[" feat ", " ft "];
/// Propres au nom d'artiste : dans un titre, « Dance with Me » n'a pas d'invité.
const ARTIST_ONLY_FEATURES: &[&str] = &[" with ", " avec "];

/// Un artiste crédité sur une écoute : sa clé et son nom tel qu'affiché.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credit {
    pub key: String,
    pub name: String,
}

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
fn remove_groups(text: &str, mut drop: impl FnMut(&str) -> bool) -> String {
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

/// La mention « feat. X » hors parenthèses, dans un texte en minuscules : sa position et sa longueur.
/// Les formes avec un point d'abord : « Little Feat feat. X » coupe après « Little Feat ». Un mot sans
/// suite n'est pas une mention, le nom « Little Feat » reste entier.
fn find_bare_feature(text: &str) -> Option<(usize, usize)> {
    MARKED_FEATURES
        .iter()
        .chain(BARE_FEATURES)
        .find_map(|marker| text.find(marker).map(|position| (position, marker.len())))
}

fn cut_bare_feature(text: &str) -> &str {
    find_bare_feature(text).map_or(text, |(position, _)| &text[..position])
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Le contenu d'une mention entre parenthèses, sans son mot d'annonce : « feat. X » donne « X ».
fn after_feature_word(inner: &str) -> &str {
    let rest = inner.trim_start_matches(|c: char| !c.is_alphanumeric());
    let rest = rest.trim_start_matches(char::is_alphanumeric);
    rest.trim_start_matches('.').trim()
}

/// Retire les mentions d'invité entre parenthèses ou crochets ; leur contenu s'ajoute à `guests`.
fn take_feature_groups(text: &str, guests: &mut Vec<String>) -> String {
    remove_groups(text, |inner| {
        let feature = is_feature(&inner.to_lowercase());
        if feature {
            guests.push(collapse(after_feature_word(inner)));
        }
        feature
    })
}

/// Les noms d'un texte d'artistes sans parenthèses, coupé à chaque séparateur et mention d'invité, sauf
/// à l'intérieur d'un nom de `UNSPLIT_ARTISTS` ; les noms vides sont écartés.
fn split_names(text: &str) -> Vec<&str> {
    // Même longueur en octets que `text` : les positions valent pour les deux.
    let lower = text.to_ascii_lowercase();
    let mut separators: Vec<&str> =
        [LIST_SEPARATORS, MARKED_FEATURES, ARTIST_ONLY_FEATURES].concat();
    if !MARKED_FEATURES.iter().any(|marker| lower.contains(marker)) {
        separators.extend(BARE_FEATURES);
    }
    let mut spans = Vec::new();
    let (mut start, mut at) = (0, 0);
    while let Some(c) = lower[at..].chars().next() {
        match separators.iter().find(|s| lower[at..].starts_with(*s)) {
            Some(separator) => {
                spans.push((start, at));
                at += separator.len();
                start = at;
            }
            None => at += c.len_utf8(),
        }
    }
    spans.push((start, text.len()));
    // Le plus long nom protégé qui commence à chaque morceau les recouvre tous.
    let mut names = Vec::new();
    let mut i = 0;
    while i < spans.len() {
        let begin = spans[i].0;
        let last = (i + 1..spans.len())
            .rev()
            .find(|&j| UNSPLIT_ARTISTS.contains(&fold(&text[begin..spans[j].1]).as_str()))
            .unwrap_or(i);
        names.push(text[begin..spans[last].1].trim());
        i = last + 1;
    }
    names.retain(|name| !name.is_empty());
    names
}

/// Les invités cités dans un titre : « T (feat. A) », « T feat. A & B », « T ft. A, B (Live) ».
fn title_guests(title: &str) -> Vec<String> {
    let mut guests = Vec::new();
    let rest = take_feature_groups(title, &mut guests);
    if let Some((position, length)) = find_bare_feature(&rest.to_ascii_lowercase()) {
        let after = &rest[position + length..];
        let end = [after.find(['(', '[']), after.find(" - ")]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(after.len());
        guests.push(after[..end].to_owned());
    }
    guests
}

/// Les artistes crédités d'une écoute, dans l'ordre : ceux du nom d'artiste, puis les invités cités
/// dans le titre. Le premier est l'artiste principal, qui donne les clés d'album et de titre. Une clé
/// n'apparaît qu'une fois. Un nom d'artiste qui ne donne aucun nom reste entier, comme avant.
pub fn credited_artists(artist: &str, title: &str) -> Vec<Credit> {
    fn credit(credits: &mut Vec<Credit>, name: &str) {
        let key = fold(name);
        if !key.is_empty() && !credits.iter().any(|c| c.key == key) {
            credits.push(Credit {
                key,
                name: name.to_owned(),
            });
        }
    }
    let mut guests = Vec::new();
    let main = take_feature_groups(artist, &mut guests);
    let mut credits = Vec::new();
    for text in std::iter::once(&main).chain(&guests) {
        for name in split_names(text) {
            credit(&mut credits, name);
        }
    }
    if credits.is_empty() {
        credit(&mut credits, &collapse(artist));
    }
    for text in title_guests(title) {
        for name in split_names(&text) {
            credit(&mut credits, name);
        }
    }
    credits
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

/// La clé de l'artiste principal d'un crédit : « A & B feat. C » donne « a ».
pub fn artist_key(artist: &str) -> String {
    credited_artists(artist, "")
        .into_iter()
        .next()
        .map_or_else(|| fold(artist), |credit| credit.key)
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

/// Artiste principal et titre : deux versions d'un même titre ont la même clé, et « A & B – T » est
/// le même titre que « A – T (feat. B) ».
pub fn track_key(artist: &str, title: &str) -> String {
    format!("{}{KEY_SEPARATOR}{}", artist_key(artist), title_key(title))
}

/// Artiste principal et album, sinon tous les « Greatest Hits » seraient confondus : « A & B » et
/// « A & C » partagent l'album de A. Le nom de l'album reste tel quel : « Album » et « Album - Single »
/// restent deux albums, et une compilation se coupe par artiste principal (limites connues, sans catalogue).
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
            // Un crédit à plusieurs artistes donne son artiste principal.
            ("A2H & Dau", "a2h"),
            ("Zek, Jungle Jack & Caballero", "zek"),
            (
                "Daft Punk, Pharrell Williams (feat. Nile Rodgers)",
                "daft punk",
            ),
            // Sauf les vrais groupes, seuls ou dans un crédit.
            ("Simon & Garfunkel", "simon & garfunkel"),
            ("SIMON  &  GARFUNKEL feat. Someone", "simon & garfunkel"),
            ("Earth, Wind & Fire", "earth, wind & fire"),
            ("Someone & Earth, Wind & Fire", "someone"),
            ("AC/DC", "ac/dc"),
            ("10,000 Maniacs", "10,000 maniacs"),
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

    fn credits(artist: &str, title: &str) -> Vec<(String, String)> {
        credited_artists(artist, title)
            .into_iter()
            .map(|c| (c.key, c.name))
            .collect()
    }

    fn names(artist: &str, title: &str) -> Vec<String> {
        credits(artist, title).into_iter().map(|c| c.1).collect()
    }

    #[test]
    fn every_credited_artist_is_named_once_the_lead_first() {
        for (artist, title, expected) in [
            (
                "Zek, Jungle Jack & Caballero",
                "T",
                &["Zek", "Jungle Jack", "Caballero"][..],
            ),
            ("A2H & Dau", "T", &["A2H", "Dau"]),
            ("A x B", "T", &["A", "B"]),
            ("A X B et C and D", "T", &["A", "B", "C", "D"]),
            ("A / B + C", "T", &["A", "B", "C"]),
            ("A feat. B", "T", &["A", "B"]),
            ("A ft. B & C", "T", &["A", "B", "C"]),
            ("A featuring B", "T", &["A", "B"]),
            ("A with B", "T", &["A", "B"]),
            ("A avec B", "T", &["A", "B"]),
            ("A feat B", "T", &["A", "B"]),
            ("A (feat. B, C)", "T", &["A", "B", "C"]),
            ("A [Feat B]", "T", &["A", "B"]),
            // Les invités du titre suivent ceux du nom d'artiste.
            ("BenPLG", "X (feat. Orelsan)", &["BenPLG", "Orelsan"]),
            (
                "BenPLG",
                "X feat. Orelsan & Nekfeu",
                &["BenPLG", "Orelsan", "Nekfeu"],
            ),
            ("BenPLG", "X ft. A, B", &["BenPLG", "A", "B"]),
            ("BenPLG", "X FT A (Live)", &["BenPLG", "A"]),
            ("BenPLG", "X feat. A - Remastered", &["BenPLG", "A"]),
            ("BenPLG", "X (with A) [Live]", &["BenPLG", "A"]),
            (
                "BenPLG & Orelsan",
                "X (feat. Orelsan)",
                &["BenPLG", "Orelsan"],
            ),
            // Les doublons, par clé : le premier nom gagne.
            ("A & B feat. B", "T", &["A", "B"]),
            ("A & b", "T (feat. B)", &["A", "b"]),
            // Les noms vides sont écartés.
            ("A & ", "T", &["A"]),
            (", A", "T", &["A"]),
            // Les vrais groupes ne se coupent pas, seuls ou dans un crédit.
            ("Simon & Garfunkel", "T", &["Simon & Garfunkel"]),
            ("Bigflo & Oli", "T", &["Bigflo & Oli"]),
            ("Bigflo & Oli feat. X", "T", &["Bigflo & Oli", "X"]),
            (
                "Tyler, The Creator",
                "T (feat. A)",
                &["Tyler, The Creator", "A"],
            ),
            ("Kool & The Gang & A", "T", &["Kool & The Gang", "A"]),
            (
                "Earth, Wind & Fire & A",
                "T (feat. Simon & Garfunkel)",
                &["Earth, Wind & Fire", "A", "Simon & Garfunkel"],
            ),
            // Un mot n'est pas un séparateur.
            ("Little Feat", "T", &["Little Feat"]),
            ("Little Feat feat. A", "T", &["Little Feat", "A"]),
            ("Andrea Bocelli", "T", &["Andrea Bocelli"]),
            ("Max & Co", "Dance with Me", &["Max", "Co"]),
            ("Bonobo (Live)", "Feat", &["Bonobo (Live)"]),
            ("Malcolm X", "T", &["Malcolm X"]),
            // Un nom réduit à rien reste entier.
            ("&", "T", &["&"]),
        ] {
            assert_eq!(names(artist, title), expected, "{artist} – {title}");
        }
        assert_eq!(
            credits("Zek,  Jungle JACK & Cabállero", "T"),
            [
                ("zek".to_owned(), "Zek".to_owned()),
                ("jungle jack".to_owned(), "Jungle JACK".to_owned()),
                ("cabállero".to_owned(), "Cabállero".to_owned()),
            ]
        );
    }

    #[test]
    fn the_unsplit_artists_are_folded_keys_that_stay_whole() {
        for name in UNSPLIT_ARTISTS {
            assert_eq!(fold(name), *name, "clé non repliée : {name}");
            assert_eq!(credits(name, ""), [(name.to_string(), name.to_string())]);
        }
    }

    #[test]
    fn a_guest_does_not_change_the_track_and_a_co_artist_does_not_change_the_album() {
        assert_eq!(
            track_key("BenPLG & Orelsan", "X"),
            track_key("BenPLG", "X (feat. Orelsan)")
        );
        assert_eq!(track_key("BenPLG", "X"), "benplg\tx");
        assert_eq!(
            album_key("A2H & Dau", "Les gosses effrayés"),
            album_key("A2H & zek", "Les gosses effrayés")
        );
        assert_ne!(track_key("A & B", "T"), track_key("B & A", "T"));
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
