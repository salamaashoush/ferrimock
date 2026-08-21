//! Text that reads like a product wrote it, rather than like Cicero did.
//!
//! Lorem is the right answer for "some text of about this length" and the wrong
//! answer for a field a person will read. A schema saying `title: String` gives
//! nothing to go on, but the *field* is still a title, and every real title is
//! a short noun phrase drawn from the vocabulary of whatever the API is about.
//! Composing one from that vocabulary costs the same as drawing eight lorem
//! words and produces something a designer can screenshot.
//!
//! The vocabulary is deliberately generic — reports, invoices, releases,
//! onboarding — because it has to read plausibly for an API this engine has
//! never seen. A profile that knows the domain can answer ahead of it.

use super::rng::rng;
use rand::seq::IndexedRandom;

/// Things an API tends to be about.
const SUBJECTS: &[&str] = &[
    "account",
    "invoice",
    "report",
    "release",
    "campaign",
    "workspace",
    "policy",
    "contract",
    "shipment",
    "dashboard",
    "integration",
    "migration",
    "budget",
    "roadmap",
    "inventory",
    "order",
    "audit",
    "backup",
    "template",
    "notification",
    "permission",
    "subscription",
    "deployment",
    "incident",
    "onboarding",
    "handoff",
    "review",
    "proposal",
    "renewal",
    "settlement",
    "forecast",
    "allocation",
    "reconciliation",
    "credential",
    "endpoint",
    "ledger",
    "statement",
    "remittance",
    "payout",
    "refund",
    "chargeback",
    "entitlement",
    "quota",
    "tenant",
    "directory",
    "roster",
    "assignment",
    "escalation",
    "retention",
    "disclosure",
    "attestation",
    "pipeline",
    "artifact",
    "manifest",
    "changelog",
    "runbook",
    "rotation",
    "failover",
    "snapshot",
    "segment",
    "cohort",
    "funnel",
    "touchpoint",
    "impression",
    "conversion",
    "attribution",
    "requisition",
    "purchase",
    "vendor",
    "supplier",
    "catalogue",
    "listing",
    "fulfilment",
    "carrier",
    "waybill",
    "customs",
    "tariff",
    "duty",
    "levy",
    "charter",
    "milestone",
    "deliverable",
    "dependency",
    "blocker",
    "retrospective",
    "questionnaire",
    "submission",
    "adjudication",
    "appeal",
    "waiver",
    "exemption",
    "enrolment",
    "eligibility",
    "coverage",
    "claim",
    "adjustment",
    "premium",
    "lease",
    "amendment",
    "addendum",
    "termination",
    "novation",
    "survey",
    "benchmark",
    "scorecard",
    "rubric",
    "calibration",
    "inspection",
    "certification",
    "licence",
    "permit",
    "variance",
    "dispatch",
    "itinerary",
    "allowance",
    "reimbursement",
    "expense",
    "formulary",
    "indication",
    "contraindication",
    "dosage",
    "titration",
    "consent",
    "authorisation",
    "revocation",
    "delegation",
    "custodian",
];

/// What gets done to them.
const ACTIONS: &[&str] = &[
    "review",
    "rollout",
    "migration",
    "cleanup",
    "refresh",
    "sync",
    "audit",
    "handover",
    "planning",
    "approval",
    "renewal",
    "rollback",
    "import",
    "export",
    "archive",
    "upgrade",
    "reconciliation",
    "verification",
    "escalation",
    "remediation",
    "onboarding",
    "offboarding",
    "provisioning",
    "decommission",
    "failover",
    "cutover",
    "rehearsal",
    "triage",
    "grooming",
    "sequencing",
    "batching",
    "throttling",
    "attestation",
    "certification",
    "revalidation",
    "recertification",
    "settlement",
    "clearing",
    "netting",
    "posting",
    "accrual",
    "ingestion",
    "normalisation",
    "enrichment",
    "deduplication",
    "retention",
    "purge",
    "redaction",
    "anonymisation",
    "handshake",
    "negotiation",
    "arbitration",
    "adjudication",
];

/// How they are qualified.
const QUALIFIERS: &[&str] = &[
    "quarterly",
    "annual",
    "monthly",
    "internal",
    "external",
    "regional",
    "global",
    "draft",
    "final",
    "legacy",
    "pending",
    "archived",
    "shared",
    "private",
    "priority",
    "scheduled",
    "automated",
    "manual",
    "consolidated",
    "provisional",
    "interim",
    "retroactive",
    "supplementary",
    "conditional",
    "expedited",
    "deferred",
    "recurring",
    "one-off",
    "ad-hoc",
    "restricted",
    "confidential",
    "redacted",
    "anonymised",
    "aggregated",
    "upstream",
    "downstream",
    "inbound",
    "outbound",
    "bilateral",
    "nightly",
    "weekly",
    "rolling",
    "incremental",
    "cumulative",
    "mandatory",
    "optional",
    "discretionary",
    "statutory",
    "contractual",
    "preliminary",
    "revised",
    "amended",
    "superseded",
    "reinstated",
];

/// Periods a title hangs off.
///
/// Capitalised, because a period leads a title and a title starts with a
/// capital.
const PERIODS: &[&str] = &[
    "Q1",
    "Q2",
    "Q3",
    "Q4",
    "H1",
    "H2",
    "2023",
    "2024",
    "2025",
    "2026",
    "FY23",
    "FY24",
    "FY25",
    "FY26",
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "May",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
    "Week-1",
    "Week-2",
    "Sprint-14",
    "Sprint-15",
    "Cycle-3",
    "Cycle-4",
];

/// Clauses a description is built from.
///
/// The braces are placeholders this module substitutes, not format arguments.
/// Enough of them that a description of a few sentences does not repeat one
/// verbatim -- ten of these read as ten, however varied the words inside them.
#[allow(clippy::literal_string_with_formatting_args)]
const CLAUSES: &[&str] = &[
    "Tracks every {subject} raised against this account",
    "Generated automatically when {a subject} changes state",
    "Kept for audit purposes and never edited in place",
    "Owned by the team that requested the {action}",
    "Superseded by the {qualifier} version once it is approved",
    "Includes the attachments uploaded during {action}",
    "Visible to collaborators with at least read access",
    "Refreshed nightly from the upstream system of record",
    "Excludes anything already archived",
    "Applies from the start of the billing period",
    "Raised whenever {a subject} misses its {qualifier} threshold",
    "Reconciled against the {qualifier} ledger before it is published",
    "Held until the {action} it depends on has cleared",
    "Carries the reference the {subject} was originally filed under",
    "Rebuilt from source whenever the {action} runs",
    "Retained for seven years, then purged without notice",
    "Signed off by whoever owns the {subject} it belongs to",
    "Split by region so each team sees only its own {subject}",
    "Locked once the {action} completes, and reopened only by request",
    "Derived from the {qualifier} figures rather than the raw feed",
    "Queued behind any {qualifier} {subject} already in flight",
    "Emitted once per {subject}, and never retried",
    "Scoped to the workspace the {action} was started in",
    "Populated on first read, and cached until the {subject} changes",
    "Rejected if the {subject} it references has already closed",
    "Mirrored to the reporting store within a few minutes",
    "Annotated with whatever the {action} produced",
    "Counted toward the {qualifier} quota for this account",
    "Ignored by downstream systems until the {subject} is approved",
    "Replaced wholesale on every {action}, never merged",
    "Numbered in the order the {subject} arrived, not the order it was filed",
    "Kept in step with the {qualifier} {subject} it was copied from",
    "Expires at the end of the period unless the {action} renews it",
    "Attributed to the requester rather than to whoever approved it",
];

fn pick(options: &[&str]) -> String {
    let mut rng = rng();
    (*options.choose(&mut rng).unwrap_or(&"item")).to_string()
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// A short noun phrase, the way a title, a name or a subject line reads.
///
/// Named for what it is rather than `fake_title`, which already means an
/// honorific and is a different kind of answer entirely.
///
/// `Quarterly Budget Review`, `Invoice Export`, `Q3 Shipment Audit`.
#[must_use]
pub fn fake_headline() -> String {
    use rand::RngExt as _;
    let shape = rng().random_range(0..5_u8);
    let words: Vec<String> = match shape {
        0 => vec![title_case(&pick(QUALIFIERS)), title_case(&pick(SUBJECTS))],
        1 => vec![title_case(&pick(SUBJECTS)), title_case(&pick(ACTIONS))],
        2 => vec![
            pick(PERIODS),
            title_case(&pick(SUBJECTS)),
            title_case(&pick(ACTIONS)),
        ],
        3 => vec![
            title_case(&pick(QUALIFIERS)),
            title_case(&pick(SUBJECTS)),
            title_case(&pick(ACTIONS)),
        ],
        _ => vec![
            title_case(&pick(SUBJECTS)),
            "for".to_string(),
            title_case(&pick(SUBJECTS)),
        ],
    };
    words.join(" ")
}

/// A label: one or two lowercase-ish words, the way a tag or a short name reads.
#[must_use]
pub fn fake_label() -> String {
    use rand::RngExt as _;
    if rng().random_range(0..2_u8) == 0 {
        format!("{} {}", pick(QUALIFIERS), pick(SUBJECTS))
    } else {
        pick(SUBJECTS)
    }
}

/// One sentence about something, ending in a full stop.
#[must_use]
#[allow(clippy::literal_string_with_formatting_args)]
pub fn fake_prose_sentence() -> String {
    let subject = pick(SUBJECTS);
    let clause = pick(CLAUSES)
        .replace("{a subject}", &format!("{} {subject}", article(&subject)))
        .replace("{subject}", &subject)
        .replace("{action}", &pick(ACTIONS))
        .replace("{qualifier}", &pick(QUALIFIERS));
    format!("{clause}.")
}

/// `a` or `an`, so a generated sentence does not read as a generated sentence.
fn article(word: &str) -> &'static str {
    match word.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u' | 'A' | 'E' | 'I' | 'O' | 'U') => "an",
        _ => "a",
    }
}

/// Several sentences, the way a description or a summary reads.
#[must_use]
pub fn fake_prose(sentences: usize) -> String {
    let count = sentences.max(1);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(fake_prose_sentence());
    }
    out.join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::fake_data::rng::scope_seeded;

    #[test]
    fn a_title_reads_as_a_noun_phrase() {
        let _scope = scope_seeded(3);
        for _ in 0..50 {
            let title = fake_headline();
            assert!(!title.is_empty());
            assert!(!title.ends_with('.'), "a title is not a sentence: {title}");
            let first = title.chars().next().unwrap();
            assert!(
                first.is_uppercase() || first.is_numeric(),
                "a title starts capitalised: {title}"
            );
            assert!(title.split(' ').count() <= 4, "and stays short: {title}");
        }
    }

    #[test]
    fn an_article_agrees_with_the_word_after_it() {
        let _scope = scope_seeded(2);
        for _ in 0..200 {
            let text = fake_prose_sentence();
            let mut found: Vec<(&str, char)> = Vec::new();
            for article in ["a ", "an "] {
                for (at, _) in text.match_indices(article) {
                    // `an ` occurs inside `than `, and `a ` inside `via `. An
                    // article is a word, so the character before it has to end
                    // one.
                    let preceded_by_word = text
                        .get(..at)
                        .and_then(|before| before.chars().next_back())
                        .is_some_and(char::is_alphanumeric);
                    if preceded_by_word {
                        continue;
                    }
                    let after = at + article.len();
                    if let Some(next) = text.get(after..).and_then(|rest| rest.chars().next()) {
                        found.push((article.trim(), next));
                    }
                }
            }
            for (article, next) in found {
                // `an ` also matches inside `a `, so only the longer reading of
                // a position is judged.
                let vowel = "aeiou".contains(next.to_ascii_lowercase());
                if article == "a" && vowel {
                    assert!(
                        text.contains(&format!("an {next}")),
                        "`a {next}` should be `an {next}` in: {text}"
                    );
                }
                if article == "an" {
                    assert!(vowel, "`an {next}` should be `a {next}` in: {text}");
                }
            }
        }
    }

    #[test]
    fn prose_is_sentences_rather_than_word_salad() {
        let _scope = scope_seeded(5);
        let text = fake_prose(3);
        assert_eq!(text.matches('.').count(), 3, "{text}");
        assert!(text.split(' ').count() >= 12, "{text}");
    }

    #[test]
    fn generation_is_reproducible_for_a_seed() {
        let once = {
            let _scope = scope_seeded(11);
            (fake_headline(), fake_prose(2), fake_label())
        };
        let twice = {
            let _scope = scope_seeded(11);
            (fake_headline(), fake_prose(2), fake_label())
        };
        assert_eq!(once, twice);
    }

    #[test]
    fn a_label_stays_short_and_lowercase() {
        let _scope = scope_seeded(7);
        for _ in 0..25 {
            let label = fake_label();
            assert!(label.split(' ').count() <= 2, "{label}");
            assert_eq!(label, label.to_lowercase(), "{label}");
        }
    }
}
