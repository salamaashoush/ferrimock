//! Where a record is, and everything that follows from it.
//!
//! Fields inside a record were mutually independent, because every value
//! derived from `(seed, entity, ordinal, path)` and nothing else — so a user
//! in Tokyo got a French name, a `+44` phone and an `America/Bogota`
//! timezone. None of those is individually implausible and the combination is
//! impossible.
//!
//! A place is the honest form of a per-record latent: a *discrete*
//! confounder, where conditional independence given the place is the real
//! generative structure rather than an approximation of one. A continuous
//! shared factor would trade "every correlation is zero" for "every
//! correlation is equal" — rank-one with a flat residual spectrum, which is
//! its own signature — and only loadings fitted from a recording remove that.

use super::rng::rng;
use rand::RngExt;

/// How a postal code is written where a record is.
#[derive(Debug, Clone, Copy)]
pub enum PostalShape {
    /// `94107`
    Digits(usize),
    /// `SW1A 2AA`
    UkOutwardInward,
    /// `1015 CJ`
    DigitsThenLetters,
}

/// One place, and the values that have to agree with it.
#[derive(Debug, Clone, Copy)]
pub struct Place {
    pub locale: &'static str,
    pub country: &'static str,
    pub country_code: &'static str,
    pub currency: &'static str,
    pub timezone: &'static str,
    pub calling_code: &'static str,
    pub cities: &'static [&'static str],
    pub given: &'static [&'static str],
    pub family: &'static [&'static str],
    pub postal: PostalShape,
}

impl Place {
    #[must_use]
    pub fn person(&self) -> String {
        let mut source = rng();
        let given = pick(self.given, &mut source);
        let family = pick(self.family, &mut source);
        format!("{given} {family}")
    }

    #[must_use]
    pub fn city(&self) -> String {
        pick(self.cities, &mut rng()).to_string()
    }

    #[must_use]
    pub fn phone(&self) -> String {
        let mut source = rng();
        let area = source.random_range(100..999);
        let line = source.random_range(1000..9999);
        let tail = source.random_range(100..999);
        format!("{} {area} {tail} {line}", self.calling_code)
    }

    #[must_use]
    pub fn postal_code(&self) -> String {
        const LETTERS: &[u8] = b"ABCDEFGHJKLMNPRSTUVWXY";
        let mut source = rng();
        let letter = |source: &mut _| {
            char::from(
                LETTERS
                    .get(rand::RngExt::random_range(source, 0..LETTERS.len()))
                    .copied()
                    .unwrap_or(b'A'),
            )
        };
        match self.postal {
            PostalShape::Digits(width) => (0..width)
                .map(|_| char::from(b'0' + u8::try_from(source.random_range(0..10)).unwrap_or(0)))
                .collect(),
            PostalShape::UkOutwardInward => format!(
                "{}{}{} {}{}{}",
                letter(&mut source),
                letter(&mut source),
                source.random_range(1..99),
                source.random_range(0..9),
                letter(&mut source),
                letter(&mut source)
            ),
            PostalShape::DigitsThenLetters => format!(
                "{:04} {}{}",
                source.random_range(1000..9999),
                letter(&mut source),
                letter(&mut source)
            ),
        }
    }
}

fn pick<'a>(from: &'a [&'a str], source: &mut impl rand::Rng) -> &'a str {
    if from.is_empty() {
        return "";
    }
    from.get(source.random_range(0..from.len()))
        .copied()
        .unwrap_or("")
}

/// Every place a record can be.
#[must_use]
pub fn places() -> &'static [Place] {
    const PLACES: [Place; 8] = [
        Place {
            locale: "en-US",
            country: "United States",
            country_code: "US",
            currency: "USD",
            timezone: "America/New_York",
            calling_code: "+1",
            cities: &["Austin", "Denver", "Portland", "Boston", "Atlanta"],
            given: &[
                "Ava", "Marcus", "Priya", "Devon", "Rosa", "Elliot", "Naomi", "Cole",
            ],
            family: &[
                "Whitfield",
                "Alvarez",
                "Okafor",
                "Brennan",
                "Nakamura",
                "Delgado",
                "Hollis",
                "Barnett",
            ],
            postal: PostalShape::Digits(5),
        },
        Place {
            locale: "en-GB",
            country: "United Kingdom",
            country_code: "GB",
            currency: "GBP",
            timezone: "Europe/London",
            calling_code: "+44",
            cities: &["Bristol", "Leeds", "Glasgow", "Cardiff", "Sheffield"],
            given: &[
                "Imogen", "Rhys", "Aisha", "Callum", "Freya", "Dermot", "Nia", "Oscar",
            ],
            family: &[
                "Ashworth",
                "Pemberton",
                "Kaur",
                "Lockhart",
                "Okonjo",
                "Fairweather",
                "Hollingood",
                "Mackay",
            ],
            postal: PostalShape::UkOutwardInward,
        },
        Place {
            locale: "fr-FR",
            country: "France",
            country_code: "FR",
            currency: "EUR",
            timezone: "Europe/Paris",
            calling_code: "+33",
            cities: &["Lyon", "Nantes", "Toulouse", "Rennes", "Strasbourg"],
            given: &[
                "Camille", "Thibault", "Alizee", "Mathis", "Solene", "Gaspard", "Amina", "Lucien",
            ],
            family: &[
                "Beaulieu",
                "Marchand",
                "Devaux",
                "Fontaine",
                "Perrin",
                "Lemoine",
                "Traore",
                "Chevalier",
            ],
            postal: PostalShape::Digits(5),
        },
        Place {
            locale: "de-DE",
            country: "Germany",
            country_code: "DE",
            currency: "EUR",
            timezone: "Europe/Berlin",
            calling_code: "+49",
            cities: &["Leipzig", "Freiburg", "Bremen", "Aachen", "Rostock"],
            given: &[
                "Lena",
                "Jonas",
                "Mirjam",
                "Tobias",
                "Annika",
                "Florian",
                "Katharina",
                "Sven",
            ],
            family: &[
                "Brandt",
                "Hofmann",
                "Kellner",
                "Reinhardt",
                "Vogel",
                "Ziegler",
                "Ackermann",
                "Sauer",
            ],
            postal: PostalShape::Digits(5),
        },
        Place {
            locale: "es-ES",
            country: "Spain",
            country_code: "ES",
            currency: "EUR",
            timezone: "Europe/Madrid",
            calling_code: "+34",
            cities: &["Valencia", "Sevilla", "Bilbao", "Granada", "Zaragoza"],
            given: &[
                "Lucia", "Alvaro", "Nuria", "Iker", "Carmen", "Sergio", "Elena", "Bruno",
            ],
            family: &[
                "Iglesias", "Carrasco", "Mendoza", "Bautista", "Solana", "Quintero", "Rivas",
                "Aguirre",
            ],
            postal: PostalShape::Digits(5),
        },
        Place {
            locale: "ja-JP",
            country: "Japan",
            country_code: "JP",
            currency: "JPY",
            timezone: "Asia/Tokyo",
            calling_code: "+81",
            cities: &["Sapporo", "Fukuoka", "Sendai", "Nagoya", "Kanazawa"],
            given: &[
                "Haruka", "Sota", "Yuina", "Kenji", "Mio", "Takumi", "Nanami", "Riku",
            ],
            family: &[
                "Nakamura",
                "Fujimoto",
                "Ishikawa",
                "Watanabe",
                "Kobayashi",
                "Sakamoto",
                "Uehara",
                "Morita",
            ],
            postal: PostalShape::Digits(7),
        },
        Place {
            locale: "pt-BR",
            country: "Brazil",
            country_code: "BR",
            currency: "BRL",
            timezone: "America/Sao_Paulo",
            calling_code: "+55",
            cities: &["Curitiba", "Recife", "Salvador", "Fortaleza", "Manaus"],
            given: &[
                "Beatriz", "Thiago", "Larissa", "Rafael", "Juliana", "Caio", "Renata", "Vinicius",
            ],
            family: &[
                "Cardoso",
                "Nascimento",
                "Barbosa",
                "Teixeira",
                "Moreira",
                "Fonseca",
                "Rocha",
                "Batista",
            ],
            postal: PostalShape::Digits(8),
        },
        Place {
            locale: "nl-NL",
            country: "Netherlands",
            country_code: "NL",
            currency: "EUR",
            timezone: "Europe/Amsterdam",
            calling_code: "+31",
            cities: &["Utrecht", "Eindhoven", "Groningen", "Haarlem", "Delft"],
            given: &[
                "Sanne", "Bram", "Fenna", "Joost", "Maaike", "Ruben", "Lotte", "Daan",
            ],
            family: &[
                "Vermeulen",
                "Kuiper",
                "Bakhuis",
                "Verhoeven",
                "Hendriks",
                "Dijkstra",
                "Wolters",
                "Molenaar",
            ],
            postal: PostalShape::DigitsThenLetters,
        },
    ];
    &PLACES
}

/// Which place one derived word lands in.
#[must_use]
pub fn place_of(derived: u64) -> &'static Place {
    let held = places();
    let at = usize::try_from(derived % held.len() as u64).unwrap_or(0);
    held.get(at).unwrap_or_else(|| {
        // A static table with entries always has a first one; this is the
        // borrow checker's price for saying so.
        held.first().unwrap_or(&PLACELESS)
    })
}

/// The fallback a table with no entries would need, which the table above
/// never is.
static PLACELESS: Place = Place {
    locale: "en-US",
    country: "United States",
    country_code: "US",
    currency: "USD",
    timezone: "UTC",
    calling_code: "+1",
    cities: &["Springfield"],
    given: &["Alex"],
    family: &["Doe"],
    postal: PostalShape::Digits(5),
};

#[cfg(test)]
mod tests;
