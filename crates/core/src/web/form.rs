//! Champs de formulaire : décodage de ce qu'envoie le navigateur, et lecture des nombres écrits à la française.
//!
//! Un formulaire HTML envoie ses champs au format `application/x-www-form-urlencoded` : `cote=2%2C30&mise=50`. Les
//! paramètres d'une adresse (`/performance?vue=clv`) ont la même forme. Quarante lignes suffisent à les lire : pas de
//! dépendance pour cela.

/// Les champs d'un formulaire ou d'une adresse, dans l'ordre d'envoi.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Form {
    fields: Vec<(String, String)>,
}

impl Form {
    /// Lit `nom=valeur&nom=valeur`. Un champ sans `=` vaut une chaîne vide ; un champ vide est ignoré.
    pub fn parse(encoded: &str) -> Form {
        let fields = encoded
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                (decode(name), decode(value))
            })
            .collect();
        Form { fields }
    }

    /// Valeur du premier champ `name`, ou une chaîne vide s'il est absent.
    pub fn get(&self, name: &str) -> &str {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map_or("", |(_, value)| value.as_str())
    }

    /// Le champ `name` a-t-il été envoyé (même vide) ?
    pub fn has(&self, name: &str) -> bool {
        self.fields.iter().any(|(field, _)| field == name)
    }
}

/// Décode un composant : `+` devient une espace, `%XX` l'octet XX. Les octets sont ensuite lus en UTF-8 ; une séquence
/// invalide devient `�` plutôt qu'une erreur (un champ malformé sera refusé plus loin par sa validation).
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            // `get(i + 1..i + 3)` rend `None` près de la fin : un `%` tronqué est gardé tel quel.
            b'%' => match bytes.get(i + 1..i + 3).and_then(hex_pair) {
                Some(byte) => {
                    out.push(byte);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `[b'2', b'C']` → `0x2C`.
fn hex_pair(pair: &[u8]) -> Option<u8> {
    let text = std::str::from_utf8(pair).ok()?;
    u8::from_str_radix(text, 16).ok()
}

/// Encode un texte pour une adresse ou un cookie : tout sauf `A-Z a-z 0-9 - _ . ~` devient `%XX`.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Lit un nombre tapé à la main : virgule ou point décimal ; texte vide = absent.
pub fn parse_number(text: &str, field: &str) -> Result<Option<f64>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map(Some)
        .ok_or_else(|| format!("{field} : « {trimmed} » n'est pas un nombre"))
}

/// Un nombre obligatoire.
pub fn required(text: &str, field: &str) -> Result<f64, String> {
    parse_number(text, field)?.ok_or_else(|| format!("{field} est requis"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_decoded_with_plus_percent_and_utf8() {
        let form = Form::parse("cote=2%2C30&nom=Saint+%C3%89tienne&vide=&seul&%ZZ=a%2");
        assert_eq!(form.get("cote"), "2,30");
        assert_eq!(form.get("nom"), "Saint Étienne");
        assert_eq!(form.get("vide"), "");
        assert!(form.has("seul") && form.has("vide"));
        assert!(!form.has("absent"));
        assert_eq!(form.get("absent"), "");
        // Un `%` mal formé est gardé tel quel.
        assert_eq!(form.get("%ZZ"), "a%2");
        // Octets invalides en UTF-8 : remplacés, pas d'erreur.
        assert_eq!(Form::parse("x=%FF").get("x"), "\u{FFFD}");
    }

    #[test]
    fn encoding_then_decoding_gives_back_the_text() {
        let text = "Pari n° 3 enregistré : Falkirk – Kilmarnock, 2,30 & 50 €";
        let encoded = encode(text);
        assert!(
            encoded
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.~%".contains(c))
        );
        assert_eq!(Form::parse(&format!("m={encoded}")).get("m"), text);
    }

    #[test]
    fn numbers_accept_a_decimal_comma_and_an_empty_text_means_absent() {
        assert_eq!(parse_number("2,30", "cote"), Ok(Some(2.3)));
        assert_eq!(parse_number(" 2.3 ", "cote"), Ok(Some(2.3)));
        assert_eq!(parse_number("   ", "cote"), Ok(None));
        assert!(parse_number("abc", "cote").unwrap_err().contains("cote"));
        assert!(parse_number("inf", "mise").is_err());
        assert!(parse_number("NaN", "mise").is_err());
        assert!(required("", "mise").unwrap_err().contains("requis"));
    }
}
