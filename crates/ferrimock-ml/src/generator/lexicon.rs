//! The words and formats each locale is written in.
//!
//! Half the value of this table is that most of it is not ASCII. A model whose
//! corpus was written in English learns that a person's name is two capitalised
//! Latin words, and then calls a Japanese display name opaque -- and the opaque
//! residual is the one place a model was supposed to help.
//!
//! The other half is the formats. A postal code is five digits in one country,
//! two alphanumeric groups in another, and seven digits with a dash in a third;
//! a phone number's punctuation changes with every calling code. These are the
//! shapes a field actually holds, and a corpus that knows one of them teaches a
//! model that the others are something else.

use super::dialect::Locale;
use super::rng::Rng;

/// How a country writes a postal code.
#[derive(Debug, Clone, Copy)]
pub enum PostalFormat {
    /// `94107`
    FiveDigits,
    /// `1011`
    FourDigits,
    /// `110 018`
    FiveDigitsSpaced,
    /// `SW1A 2AA`
    UkAlphanumeric,
    /// `K1A 0B1`
    CaAlphanumeric,
    /// `1234 AB`
    NlAlphanumeric,
    /// `123-4567`
    JpDashed,
    /// `12-345`
    PlDashed,
    /// `01310-100`
    BrDashed,
    /// `123 45`
    SeSpaced,
    /// `560001`
    SixDigits,
    /// `1234567`
    SevenDigits,
}

impl PostalFormat {
    pub fn render(self, rng: &mut Rng) -> String {
        match self {
            Self::FiveDigits => rng.digits(5),
            Self::FourDigits => rng.digits(4),
            Self::FiveDigitsSpaced => format!("{} {}", rng.digits(3), rng.digits(3)),
            Self::UkAlphanumeric => {
                let area_length = rng.between(1, 2);
                let area = rng.from_alphabet(b"ABCDEFGHIJKLMNOPRSTUWYZ", area_length);
                format!(
                    "{area}{} {}{}",
                    rng.digits(1),
                    rng.digits(1),
                    rng.from_alphabet(b"ABDEFGHJLNPQRSTUWXYZ", 2)
                )
            }
            Self::CaAlphanumeric => format!(
                "{}{}{} {}{}{}",
                rng.from_alphabet(b"ABCEGHJKLMNPRSTVXY", 1),
                rng.digits(1),
                rng.from_alphabet(b"ABCEGHJKLMNPRSTVWXYZ", 1),
                rng.digits(1),
                rng.from_alphabet(b"ABCEGHJKLMNPRSTVWXYZ", 1),
                rng.digits(1)
            ),
            Self::NlAlphanumeric => {
                format!(
                    "{} {}",
                    rng.digits(4),
                    rng.from_alphabet(b"ABCDEFGHJKLMNPRSTUVWXYZ", 2)
                )
            }
            Self::JpDashed => format!("{}-{}", rng.digits(3), rng.digits(4)),
            Self::PlDashed => format!("{}-{}", rng.digits(2), rng.digits(3)),
            Self::BrDashed => format!("{}-{}", rng.digits(5), rng.digits(3)),
            Self::SeSpaced => format!("{} {}", rng.digits(3), rng.digits(2)),
            Self::SixDigits => rng.digits(6),
            Self::SevenDigits => rng.digits(7),
        }
    }
}

/// How a country writes a phone number, as a calling code and a body shape.
#[derive(Debug, Clone, Copy)]
pub struct PhoneFormat {
    pub calling_code: &'static str,
    /// Lengths of the groups the national number is broken into.
    pub groups: &'static [usize],
    pub separator: char,
}

impl PhoneFormat {
    /// Render a number, sometimes in its international form and sometimes in the
    /// national one -- both turn up in the same API.
    pub fn render(self, rng: &mut Rng) -> String {
        let body: Vec<String> = self.groups.iter().map(|len| rng.digits(*len)).collect();
        let joined = body.join(&self.separator.to_string());
        match rng.weighted(&[5, 3, 2]) {
            0 => format!("+{} {joined}", self.calling_code),
            1 => format!("+{}{}", self.calling_code, body.concat()),
            _ => format!("0{joined}"),
        }
    }
}

/// Everything about a locale that shows up inside a value.
#[derive(Debug, Clone, Copy)]
pub struct LocaleData {
    pub given_names: &'static [&'static str],
    pub family_names: &'static [&'static str],
    /// Ordinary nouns, used for sentences, slugs and file names.
    pub words: &'static [&'static str],
    /// Whether words are separated by spaces when they form a sentence.
    pub spaced: bool,
    /// The character a sentence ends with.
    pub full_stop: &'static str,
    /// Hosts that show up in email addresses and URLs from this locale.
    pub domains: &'static [&'static str],
    pub timezones: &'static [&'static str],
    pub currency: &'static str,
    pub postal: PostalFormat,
    pub phone: PhoneFormat,
}

impl LocaleData {
    /// A person's name, in the order this locale writes one.
    pub fn person_name(&self, locale: Locale, rng: &mut Rng) -> String {
        let given = rng.pick(self.given_names);
        let family = rng.pick(self.family_names);
        // Japanese, Chinese, Korean and Hungarian write the family name first,
        // and the CJK three write it without a space.
        match locale {
            Locale::JaJp | Locale::ZhCn | Locale::KoKr => format!("{family}{given}"),
            _ => format!("{given} {family}"),
        }
    }

    /// A sentence, in this locale's script and punctuation.
    pub fn sentence(&self, rng: &mut Rng, words: usize) -> String {
        let drawn: Vec<&str> = (0..words.max(3)).map(|_| rng.pick(self.words)).collect();
        let body = if self.spaced {
            drawn.join(" ")
        } else {
            drawn.concat()
        };
        // Only scripts with letter case get a capital, and only the Latin ones
        // have one to give.
        let opened = if self.spaced && body.chars().next().is_some_and(char::is_alphabetic) {
            let mut chars = body.chars();
            chars.next().map_or_else(
                || body.clone(),
                |first| first.to_uppercase().collect::<String>() + chars.as_str(),
            )
        } else {
            body
        };
        format!("{opened}{}", self.full_stop)
    }

    /// A lowercase ASCII word, for the parts of a value that stay ASCII whatever
    /// the locale -- a slug, a host name, the local part of an address.
    pub fn ascii_word(&self, rng: &mut Rng) -> String {
        const FALLBACK: [&str; 16] = [
            "alpha", "bravo", "delta", "echo", "kilo", "lima", "mike", "nova", "oscar", "romeo",
            "sierra", "tango", "quebec", "victor", "whisky", "zulu",
        ];
        let word = rng.pick(self.words);
        let ascii: String = word
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .flat_map(char::to_lowercase)
            .collect();
        if ascii.len() >= 3 {
            ascii
        } else {
            rng.pick(&FALLBACK).to_string()
        }
    }
}

/// Everything known about a locale.
#[allow(clippy::too_many_lines)] // One entry per locale; splitting it only hides the table
pub fn data(locale: Locale) -> &'static LocaleData {
    match locale {
        Locale::EnUs => &LocaleData {
            given_names: &[
                "Grace",
                "Alan",
                "Ada",
                "Ken",
                "Barbara",
                "Leslie",
                "Radia",
                "Frances",
                "Marvin",
                "Katherine",
            ],
            family_names: &[
                "Hopper", "Turing", "Lovelace", "Thompson", "Liskov", "Lamport", "Perlman",
                "Allen", "Minsky", "Johnson",
            ],
            words: &[
                "report", "invoice", "folder", "project", "summary", "contract", "budget",
                "meeting", "review", "archive", "draft", "proposal",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["example.com", "acme.io", "northwind.co", "contoso.com"],
            timezones: &["America/New_York", "America/Chicago", "America/Los_Angeles"],
            currency: "USD",
            postal: PostalFormat::FiveDigits,
            phone: PhoneFormat {
                calling_code: "1",
                groups: &[3, 3, 4],
                separator: '-',
            },
        },
        Locale::EnGb => &LocaleData {
            given_names: &[
                "Oliver", "Amelia", "Harry", "Isla", "Charlie", "Freya", "Arthur", "Poppy",
            ],
            family_names: &[
                "Smith", "Jones", "Taylor", "Brown", "Wilson", "Evans", "Thomas", "Walker",
            ],
            words: &[
                "invoice",
                "enquiry",
                "licence",
                "programme",
                "cheque",
                "quarter",
                "tender",
                "minutes",
                "annexe",
                "schedule",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["example.co.uk", "britannia.uk", "thameside.org.uk"],
            timezones: &["Europe/London"],
            currency: "GBP",
            postal: PostalFormat::UkAlphanumeric,
            phone: PhoneFormat {
                calling_code: "44",
                groups: &[4, 6],
                separator: ' ',
            },
        },
        Locale::DeDe => &LocaleData {
            given_names: &[
                "Lukas", "Hannah", "Jonas", "Emilia", "Felix", "Mia", "Elias", "Lina",
            ],
            family_names: &[
                "Müller",
                "Schmidt",
                "Schneider",
                "Fischer",
                "Weber",
                "Wagner",
                "Becker",
                "Hoffmann",
            ],
            words: &[
                "Rechnung",
                "Vertrag",
                "Bericht",
                "Ordner",
                "Angebot",
                "Kunde",
                "Zahlung",
                "Vorlage",
                "Übersicht",
                "Anlage",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["beispiel.de", "musterfirma.de", "handel.at"],
            timezones: &["Europe/Berlin", "Europe/Vienna"],
            currency: "EUR",
            postal: PostalFormat::FiveDigits,
            phone: PhoneFormat {
                calling_code: "49",
                groups: &[3, 8],
                separator: ' ',
            },
        },
        Locale::FrFr => &LocaleData {
            given_names: &[
                "Camille", "Louis", "Chloé", "Hugo", "Léa", "Nathan", "Manon", "Théo",
            ],
            family_names: &[
                "Martin", "Bernard", "Dubois", "Durand", "Moreau", "Laurent", "Lefebvre", "Girard",
            ],
            words: &[
                "facture", "contrat", "dossier", "rapport", "devis", "client", "paiement",
                "modèle", "résumé", "annexe",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["exemple.fr", "societe.fr", "entreprise.be"],
            timezones: &["Europe/Paris", "Europe/Brussels"],
            currency: "EUR",
            postal: PostalFormat::FiveDigits,
            phone: PhoneFormat {
                calling_code: "33",
                groups: &[1, 2, 2, 2, 2],
                separator: ' ',
            },
        },
        Locale::EsEs => &LocaleData {
            given_names: &[
                "Lucía", "Martín", "Sofía", "Mateo", "Paula", "Diego", "Valeria", "Álvaro",
            ],
            family_names: &[
                "García",
                "Rodríguez",
                "Fernández",
                "López",
                "Martínez",
                "Sánchez",
                "Pérez",
                "Gómez",
            ],
            words: &[
                "factura",
                "contrato",
                "informe",
                "carpeta",
                "presupuesto",
                "cliente",
                "pago",
                "plantilla",
                "resumen",
                "anexo",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["ejemplo.es", "empresa.es", "comercio.mx"],
            timezones: &["Europe/Madrid", "America/Mexico_City"],
            currency: "EUR",
            postal: PostalFormat::FiveDigits,
            phone: PhoneFormat {
                calling_code: "34",
                groups: &[3, 3, 3],
                separator: ' ',
            },
        },
        Locale::PtBr => &LocaleData {
            given_names: &[
                "Ana", "João", "Maria", "Pedro", "Beatriz", "Lucas", "Carolina", "Rafael",
            ],
            family_names: &[
                "Silva", "Santos", "Oliveira", "Souza", "Pereira", "Costa", "Almeida", "Ferreira",
            ],
            words: &[
                "fatura",
                "contrato",
                "relatório",
                "pasta",
                "orçamento",
                "cliente",
                "pagamento",
                "modelo",
                "resumo",
                "anexo",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["exemplo.com.br", "empresa.br", "comercio.pt"],
            timezones: &["America/Sao_Paulo", "Europe/Lisbon"],
            currency: "BRL",
            postal: PostalFormat::BrDashed,
            phone: PhoneFormat {
                calling_code: "55",
                groups: &[2, 5, 4],
                separator: ' ',
            },
        },
        Locale::ItIt => &LocaleData {
            given_names: &[
                "Giulia",
                "Lorenzo",
                "Sofia",
                "Francesco",
                "Aurora",
                "Alessandro",
                "Chiara",
                "Matteo",
            ],
            family_names: &[
                "Rossi", "Russo", "Ferrari", "Esposito", "Bianchi", "Romano", "Colombo", "Ricci",
            ],
            words: &[
                "fattura",
                "contratto",
                "relazione",
                "cartella",
                "preventivo",
                "cliente",
                "pagamento",
                "modello",
                "riepilogo",
                "allegato",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["esempio.it", "azienda.it"],
            timezones: &["Europe/Rome"],
            currency: "EUR",
            postal: PostalFormat::FiveDigits,
            phone: PhoneFormat {
                calling_code: "39",
                groups: &[3, 7],
                separator: ' ',
            },
        },
        Locale::NlNl => &LocaleData {
            given_names: &[
                "Daan", "Sanne", "Lars", "Fenna", "Sem", "Julia", "Bram", "Eva",
            ],
            family_names: &[
                "de Jong",
                "Jansen",
                "de Vries",
                "van den Berg",
                "Bakker",
                "Visser",
                "Smit",
                "Meijer",
            ],
            words: &[
                "factuur",
                "contract",
                "rapport",
                "map",
                "offerte",
                "klant",
                "betaling",
                "sjabloon",
                "overzicht",
                "bijlage",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["voorbeeld.nl", "bedrijf.nl"],
            timezones: &["Europe/Amsterdam"],
            currency: "EUR",
            postal: PostalFormat::NlAlphanumeric,
            phone: PhoneFormat {
                calling_code: "31",
                groups: &[2, 3, 4],
                separator: '-',
            },
        },
        Locale::SvSe => &LocaleData {
            given_names: &[
                "Erik", "Anna", "Lars", "Karin", "Johan", "Sara", "Nils", "Elin",
            ],
            family_names: &[
                "Andersson",
                "Johansson",
                "Karlsson",
                "Nilsson",
                "Eriksson",
                "Larsson",
                "Olsson",
                "Persson",
            ],
            words: &[
                "faktura",
                "avtal",
                "rapport",
                "mapp",
                "offert",
                "kund",
                "betalning",
                "mall",
                "sammanfattning",
                "bilaga",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["exempel.se", "foretag.se"],
            timezones: &["Europe/Stockholm"],
            currency: "SEK",
            postal: PostalFormat::SeSpaced,
            phone: PhoneFormat {
                calling_code: "46",
                groups: &[2, 3, 2, 2],
                separator: '-',
            },
        },
        Locale::PlPl => &LocaleData {
            given_names: &[
                "Jakub", "Zofia", "Kacper", "Julia", "Antoni", "Maja", "Filip", "Lena",
            ],
            family_names: &[
                "Nowak",
                "Kowalski",
                "Wiśniewski",
                "Wójcik",
                "Kowalczyk",
                "Kamiński",
                "Lewandowski",
                "Zieliński",
            ],
            words: &[
                "faktura",
                "umowa",
                "raport",
                "folder",
                "oferta",
                "klient",
                "płatność",
                "szablon",
                "podsumowanie",
                "załącznik",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["przyklad.pl", "firma.pl"],
            timezones: &["Europe/Warsaw"],
            currency: "PLN",
            postal: PostalFormat::PlDashed,
            phone: PhoneFormat {
                calling_code: "48",
                groups: &[3, 3, 3],
                separator: ' ',
            },
        },
        Locale::TrTr => &LocaleData {
            given_names: &[
                "Yusuf", "Zeynep", "Mustafa", "Elif", "Ahmet", "Defne", "Ömer", "Azra",
            ],
            family_names: &[
                "Yılmaz", "Kaya", "Demir", "Şahin", "Çelik", "Yıldız", "Arslan", "Doğan",
            ],
            words: &[
                "fatura",
                "sözleşme",
                "rapor",
                "klasör",
                "teklif",
                "müşteri",
                "ödeme",
                "şablon",
                "özet",
                "ek",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["ornek.com.tr", "sirket.tr"],
            timezones: &["Europe/Istanbul"],
            currency: "TRY",
            postal: PostalFormat::FiveDigits,
            phone: PhoneFormat {
                calling_code: "90",
                groups: &[3, 3, 4],
                separator: ' ',
            },
        },
        Locale::RuRu => &LocaleData {
            given_names: &[
                "Александр",
                "Мария",
                "Дмитрий",
                "Анна",
                "Сергей",
                "Екатерина",
                "Иван",
                "Ольга",
            ],
            family_names: &[
                "Иванов",
                "Смирнов",
                "Кузнецов",
                "Попов",
                "Васильев",
                "Петров",
                "Соколов",
                "Михайлов",
            ],
            words: &[
                "счёт",
                "договор",
                "отчёт",
                "папка",
                "заявка",
                "клиент",
                "платёж",
                "шаблон",
                "сводка",
                "приложение",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["primer.ru", "kompaniya.ru"],
            timezones: &["Europe/Moscow", "Asia/Yekaterinburg"],
            currency: "RUB",
            postal: PostalFormat::SixDigits,
            phone: PhoneFormat {
                calling_code: "7",
                groups: &[3, 3, 2, 2],
                separator: '-',
            },
        },
        Locale::ElGr => &LocaleData {
            given_names: &[
                "Γιώργος",
                "Μαρία",
                "Δημήτρης",
                "Ελένη",
                "Νίκος",
                "Σοφία",
                "Κώστας",
                "Άννα",
            ],
            family_names: &[
                "Παπαδόπουλος",
                "Γεωργίου",
                "Νικολάου",
                "Δημητρίου",
                "Ιωάννου",
                "Βασιλείου",
            ],
            words: &[
                "τιμολόγιο",
                "σύμβαση",
                "αναφορά",
                "φάκελος",
                "προσφορά",
                "πελάτης",
                "πληρωμή",
                "πρότυπο",
                "περίληψη",
                "παράρτημα",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["paradeigma.gr", "etaireia.gr"],
            timezones: &["Europe/Athens"],
            currency: "EUR",
            postal: PostalFormat::FiveDigitsSpaced,
            phone: PhoneFormat {
                calling_code: "30",
                groups: &[3, 3, 4],
                separator: ' ',
            },
        },
        Locale::ArEg => &LocaleData {
            given_names: &[
                "محمد",
                "فاطمة",
                "أحمد",
                "مريم",
                "علي",
                "نور",
                "يوسف",
                "سارة",
            ],
            family_names: &["حسن", "إبراهيم", "عبدالله", "خليل", "منصور", "سعيد"],
            words: &[
                "فاتورة",
                "عقد",
                "تقرير",
                "مجلد",
                "عرض",
                "عميل",
                "دفع",
                "قالب",
                "ملخص",
                "مرفق",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["mithal.eg", "sharika.com"],
            timezones: &["Africa/Cairo", "Asia/Riyadh"],
            currency: "EGP",
            postal: PostalFormat::FiveDigits,
            phone: PhoneFormat {
                calling_code: "20",
                groups: &[2, 4, 4],
                separator: ' ',
            },
        },
        Locale::HeIl => &LocaleData {
            given_names: &["דוד", "שרה", "יוסף", "רחל", "משה", "מרים", "אבי", "נועה"],
            family_names: &["כהן", "לוי", "מזרחי", "פרץ", "ביטון", "דהן"],
            words: &[
                "חשבונית",
                "חוזה",
                "דוח",
                "תיקייה",
                "הצעה",
                "לקוח",
                "תשלום",
                "תבנית",
                "סיכום",
                "נספח",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["dugma.co.il", "hevra.il"],
            timezones: &["Asia/Jerusalem"],
            currency: "ILS",
            postal: PostalFormat::SevenDigits,
            phone: PhoneFormat {
                calling_code: "972",
                groups: &[2, 3, 4],
                separator: '-',
            },
        },
        Locale::HiIn => &LocaleData {
            given_names: &[
                "आर्यन",
                "प्रिया",
                "रोहित",
                "अनन्या",
                "विवेक",
                "काव्या",
                "अर्जुन",
                "दीया",
            ],
            family_names: &["शर्मा", "वर्मा", "गुप्ता", "सिंह", "पटेल", "राव"],
            words: &[
                "चालान",
                "अनुबंध",
                "रिपोर्ट",
                "फ़ोल्डर",
                "प्रस्ताव",
                "ग्राहक",
                "भुगतान",
                "टेम्पलेट",
                "सारांश",
                "संलग्नक",
            ],
            spaced: true,
            full_stop: "।",
            domains: &["udaharan.in", "company.co.in"],
            timezones: &["Asia/Kolkata"],
            currency: "INR",
            postal: PostalFormat::SixDigits,
            phone: PhoneFormat {
                calling_code: "91",
                groups: &[5, 5],
                separator: ' ',
            },
        },
        Locale::ThTh => &LocaleData {
            given_names: &["สมชาย", "สุดา", "ประยุทธ", "มาลี", "อนันต์", "ณัฐ"],
            family_names: &["จันทร์", "แสงทอง", "ศรีสุข", "บุญมี", "วงศ์ไทย"],
            words: &[
                "ใบแจ้งหนี้",
                "สัญญา",
                "รายงาน",
                "โฟลเดอร์",
                "ข้อเสนอ",
                "ลูกค้า",
                "การชำระเงิน",
                "แม่แบบ",
                "สรุป",
                "เอกสารแนบ",
            ],
            spaced: false,
            full_stop: "",
            domains: &["tuayang.co.th", "borisat.th"],
            timezones: &["Asia/Bangkok"],
            currency: "THB",
            postal: PostalFormat::FiveDigits,
            phone: PhoneFormat {
                calling_code: "66",
                groups: &[2, 3, 4],
                separator: '-',
            },
        },
        Locale::JaJp => &LocaleData {
            given_names: &["太郎", "花子", "健一", "美咲", "翔太", "由紀", "大輔", "彩"],
            family_names: &[
                "佐藤", "鈴木", "高橋", "田中", "伊藤", "渡辺", "山本", "中村",
            ],
            words: &[
                "請求書",
                "契約",
                "報告書",
                "フォルダ",
                "見積",
                "顧客",
                "支払",
                "テンプレート",
                "概要",
                "添付",
            ],
            spaced: false,
            full_stop: "。",
            domains: &["example.jp", "kaisha.co.jp"],
            timezones: &["Asia/Tokyo"],
            currency: "JPY",
            postal: PostalFormat::JpDashed,
            phone: PhoneFormat {
                calling_code: "81",
                groups: &[2, 4, 4],
                separator: '-',
            },
        },
        Locale::ZhCn => &LocaleData {
            given_names: &["伟", "芳", "娜", "敏", "静", "磊", "洋", "艳"],
            family_names: &["王", "李", "张", "刘", "陈", "杨", "黄", "赵"],
            words: &[
                "发票",
                "合同",
                "报告",
                "文件夹",
                "报价",
                "客户",
                "付款",
                "模板",
                "摘要",
                "附件",
            ],
            spaced: false,
            full_stop: "。",
            domains: &["example.cn", "gongsi.com.cn"],
            timezones: &["Asia/Shanghai"],
            currency: "CNY",
            postal: PostalFormat::SixDigits,
            phone: PhoneFormat {
                calling_code: "86",
                groups: &[3, 4, 4],
                separator: ' ',
            },
        },
        Locale::KoKr => &LocaleData {
            given_names: &[
                "민준", "서연", "지호", "하은", "도윤", "지우", "예준", "수아",
            ],
            family_names: &["김", "이", "박", "최", "정", "강", "조", "윤"],
            words: &[
                "청구서",
                "계약",
                "보고서",
                "폴더",
                "견적",
                "고객",
                "결제",
                "템플릿",
                "요약",
                "첨부",
            ],
            spaced: true,
            full_stop: ".",
            domains: &["example.kr", "hoesa.co.kr"],
            timezones: &["Asia/Seoul"],
            currency: "KRW",
            postal: PostalFormat::FiveDigits,
            phone: PhoneFormat {
                calling_code: "82",
                groups: &[2, 4, 4],
                separator: '-',
            },
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn every_locale_carries_the_words_a_value_needs() {
        for locale in Locale::ALL {
            let entry = data(locale);
            let tag = locale.tag();
            assert!(!entry.given_names.is_empty(), "{tag} has no given names");
            assert!(!entry.family_names.is_empty(), "{tag} has no family names");
            assert!(entry.words.len() >= 5, "{tag} has too few words");
            assert!(!entry.domains.is_empty(), "{tag} has no domains");
            assert!(!entry.timezones.is_empty(), "{tag} has no timezones");
            assert_eq!(entry.currency.len(), 3, "{tag} currency is not ISO 4217");
            assert!(
                entry.currency.chars().all(|c| c.is_ascii_uppercase()),
                "{tag} currency is not upper case"
            );
        }
    }

    #[test]
    fn most_locales_are_not_written_in_ascii() {
        // The whole reason this table exists. If this ever passes trivially, the
        // corpus has quietly become an English one again.
        let non_ascii = Locale::ALL
            .iter()
            .filter(|locale| data(**locale).words.iter().any(|word| !word.is_ascii()))
            .count();
        assert!(
            non_ascii >= 8,
            "only {non_ascii} locales carry non-ASCII text"
        );
    }

    #[test]
    fn a_non_latin_locale_answers_with_its_own_script() {
        let mut rng = Rng::seeded(1);
        for locale in [Locale::JaJp, Locale::ZhCn, Locale::RuRu, Locale::ArEg] {
            let name = data(locale).person_name(locale, &mut rng);
            assert!(
                !name.is_ascii(),
                "{} produced an ASCII name: {name}",
                locale.tag()
            );
        }
    }

    #[test]
    fn cjk_names_are_written_family_first_and_unspaced() {
        let mut rng = Rng::seeded(2);
        for locale in [Locale::JaJp, Locale::ZhCn, Locale::KoKr] {
            let name = data(locale).person_name(locale, &mut rng);
            assert!(
                !name.contains(' '),
                "{} spaced a name it should not: {name}",
                locale.tag()
            );
        }
        assert!(
            data(Locale::EnUs)
                .person_name(Locale::EnUs, &mut rng)
                .contains(' ')
        );
    }

    #[test]
    fn a_sentence_ends_the_way_its_locale_ends_one() {
        let mut rng = Rng::seeded(3);
        assert!(data(Locale::JaJp).sentence(&mut rng, 5).ends_with('。'));
        assert!(data(Locale::HiIn).sentence(&mut rng, 5).ends_with('।'));
        assert!(data(Locale::EnUs).sentence(&mut rng, 5).ends_with('.'));
    }

    #[test]
    fn a_latin_sentence_opens_with_a_capital() {
        let mut rng = Rng::seeded(4);
        let sentence = data(Locale::EnUs).sentence(&mut rng, 6);
        assert!(
            sentence.chars().next().is_some_and(char::is_uppercase),
            "{sentence}"
        );
    }

    #[test]
    fn postal_codes_take_the_shape_their_country_writes() {
        let mut rng = Rng::seeded(5);
        assert_eq!(PostalFormat::FiveDigits.render(&mut rng).len(), 5);
        assert!(PostalFormat::JpDashed.render(&mut rng).contains('-'));
        assert!(PostalFormat::NlAlphanumeric.render(&mut rng).contains(' '));
        assert_eq!(PostalFormat::SixDigits.render(&mut rng).len(), 6);
    }

    #[test]
    fn a_phone_number_carries_its_calling_code_or_a_national_prefix() {
        let mut rng = Rng::seeded(6);
        let format = data(Locale::DeDe).phone;
        for _ in 0..50 {
            let number = format.render(&mut rng);
            assert!(
                number.starts_with("+49") || number.starts_with('0'),
                "{number}"
            );
            assert!(number.chars().any(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn an_ascii_word_is_always_available_even_from_a_non_latin_locale() {
        // Host names and slugs stay ASCII whatever language the service speaks.
        let mut rng = Rng::seeded(7);
        for locale in [Locale::JaJp, Locale::ArEg, Locale::ThTh, Locale::EnUs] {
            for _ in 0..20 {
                let word = data(locale).ascii_word(&mut rng);
                assert!(word.is_ascii() && word.len() >= 3, "{word}");
            }
        }
    }
}
