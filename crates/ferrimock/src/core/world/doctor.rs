//! A lint over the world a schema builds, naming its own tells.
//!
//! Realism is not one number. A discriminator score needs a corpus of real
//! responses, which is exactly what a mock stands in for, and it reads 1.0
//! while any single feature separates perfectly — so it cannot rank work.
//! Each check here fails independently, runs with no corpus, and reports the
//! measurement that made it fail, so a change either moves a check or it does
//! not.
//!
//! Two severities, because they are not the same claim. A [`Severity::Broken`]
//! finding is a behaviour no real API has; a [`Severity::Tell`] is a behaviour
//! a real API could have but a client can distinguish.

use std::collections::{BTreeMap, BTreeSet};

use chrono::Datelike;
use serde_json::Value as JsonValue;

use super::algebra::{DEFAULT_PAGE_SIZE, Page, Selection};
use super::model::{Cardinality, EntityType, FieldDef, Relation, ScalarKind, TextShape, ValueSpec};
use super::store::{EntityStore, Record, counted_relation, is_membership};
use crate::fake_data;
use crate::type_detector::FieldType;

/// How many instances of one entity the value checks read before they have
/// enough.
///
/// The doctor is offline, so the cap is about not walking a million-record
/// world rather than about the request path. Every check below settles well
/// under this: the hungriest wants ninety draws.
const SAMPLE_CAP: usize = 600;

/// How many *parents* a relation check walks.
///
/// Its own cap, and a much smaller one, because each parent costs a whole
/// collection: reading one parent's children materialises them. Walking every
/// record of every entity made the doctor quadratic in the size of the world,
/// which on a real schema — three hundred entities, fifty thousand records —
/// is the difference between a lint someone runs and one they do not. These
/// checks report a rate over what they walked, and they say so.
const RELATION_SAMPLE: usize = 40;

/// How many records of a collection a contiguity check reads.
///
/// A page, because a page is the unit the tell lives in: the number of runs of
/// the parent key down *one unsorted response* equals the number of distinct
/// parents on it. Reading the whole entity to see that is answering a
/// different question more slowly.
const CONTIGUITY_PAGE: usize = 400;

/// The bound `kind_value` draws every unconstrained number inside.
const DEFAULT_NUMBER_BOUND: f64 = 1000.0;

/// The last day of the shortest month, which is where a naive day draw stops.
const SHORTEST_MONTH: u32 = 28;

/// What a check claims when it fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// No real API does this.
    Broken,
    /// A real API could, but this one always does.
    Tell,
}

/// One lint, identified by what it measures rather than by what fixes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Check {
    RelationDisagreement,
    CountDisagreement,
    SelfParent,
    WorldSize,
    ContiguousChildren,
    DayOfMonth,
    SmallVocabulary,
    NumberSupport,
    ConstantListLength,
    NeverAbsent,
    FairCoin,
    UniformEnum,
    MembershipDegree,
    StaleClock,
    IdTimeOrder,
}

impl Check {
    /// The stable name a report is diffed on.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RelationDisagreement => "relation-disagreement",
            Self::CountDisagreement => "count-disagreement",
            Self::SelfParent => "self-parent",
            Self::WorldSize => "world-size",
            Self::ContiguousChildren => "contiguous-children",
            Self::DayOfMonth => "day-of-month",
            Self::SmallVocabulary => "small-vocabulary",
            Self::NumberSupport => "number-support",
            Self::ConstantListLength => "constant-list-length",
            Self::NeverAbsent => "never-absent",
            Self::FairCoin => "fair-coin",
            Self::UniformEnum => "uniform-enum",
            Self::MembershipDegree => "membership-degree",
            Self::StaleClock => "stale-clock",
            Self::IdTimeOrder => "id-time-order",
        }
    }

    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::RelationDisagreement | Self::CountDisagreement | Self::SelfParent => {
                Severity::Broken
            }
            _ => Severity::Tell,
        }
    }

    /// What a client sees, in one line.
    #[must_use]
    pub const fn tell(self) -> &'static str {
        match self {
            Self::RelationDisagreement => "the two directions of a relation answer differently",
            Self::CountDisagreement => "a count field disagrees with the collection it names",
            Self::SelfParent => "a record is its own parent",
            Self::WorldSize => "one unpaginated request returns the entire population",
            Self::ContiguousChildren => "each parent's children are one run of the default order",
            Self::DayOfMonth => "no date ever falls after the 28th",
            Self::SmallVocabulary => "the text is drawn from a closed set of words",
            Self::NumberSupport => "every value sits inside the generator's default bound",
            Self::ConstantListLength => "every array has the same length",
            Self::NeverAbsent => "an optional field is never null and never omitted",
            Self::FairCoin => "a boolean is an even split",
            Self::UniformEnum => "an enum is indistinguishable from uniform",
            Self::MembershipDegree => "a many-to-many has only two degrees",
            Self::StaleClock => "the newest timestamp is older than today",
            Self::IdTimeOrder => "sorting by id does not order by time",
        }
    }
}

/// A check that failed, and the number that failed it.
#[derive(Debug, Clone)]
pub struct Finding {
    pub check: Check,
    pub subject: String,
    pub measured: String,
}

/// A check that could not run, and what it would take.
///
/// This is its own outcome rather than a pass: a world of twelve records
/// cannot express a five-member enum, and reporting that as "uniform enums:
/// none" would say the opposite of what is true.
#[derive(Debug, Clone)]
pub struct Unmeasured {
    pub check: Check,
    pub subject: String,
    pub needs: String,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub unmeasured: Vec<Unmeasured>,
    /// Records read, across every entity.
    pub sampled: usize,
}

impl Report {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    #[must_use]
    pub fn broken(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.check.severity() == Severity::Broken)
            .count()
    }

    /// How many subjects failed each check, which is the number every change
    /// is ranked against.
    #[must_use]
    pub fn by_check(&self) -> BTreeMap<Check, usize> {
        let mut counts = BTreeMap::new();
        for finding in &self.findings {
            *counts.entry(finding.check).or_insert(0) += 1;
        }
        counts
    }

    fn fail(&mut self, check: Check, subject: String, measured: String) {
        self.findings.push(Finding {
            check,
            subject,
            measured,
        });
    }

    fn skip(&mut self, check: Check, subject: String, needs: String) {
        self.unmeasured.push(Unmeasured {
            check,
            subject,
            needs,
        });
    }
}

/// Read the world and report what a client could tell about it.
#[must_use]
pub fn examine(store: &EntityStore) -> Report {
    let mut report = Report::default();
    let graph = store.graph();

    for entity in graph.entities() {
        let records = sample(store, entity);
        report.sampled += records.len();

        check_world_size(store, entity, &mut report);
        check_values(entity, &records, &mut report);
        check_relations(store, entity, &records, &mut report);
        check_id_time_order(entity, &records, &mut report);
    }

    report.findings.sort_by(|a, b| {
        a.check
            .cmp(&b.check)
            .then_with(|| a.subject.cmp(&b.subject))
    });
    report
}

fn sample(store: &EntityStore, entity: &EntityType) -> Vec<Record> {
    store
        .keys(entity.name.as_str())
        .into_iter()
        .take(SAMPLE_CAP)
        .filter_map(|key| store.get(entity.name.as_str(), &key))
        .collect()
}

fn check_world_size(store: &EntityStore, entity: &EntityType, report: &mut Report) {
    let count = store.count(entity.name.as_str());
    if count > 0 && count <= DEFAULT_PAGE_SIZE {
        report.fail(
            Check::WorldSize,
            entity.name.to_string(),
            format!("{count} instance(s); a default page holds {DEFAULT_PAGE_SIZE}"),
        );
    }
}

/// Everything one field of one entity was observed to hold.
#[derive(Debug, Default)]
struct FieldStats {
    present: usize,
    absent: usize,
    nulls: usize,
    numbers: Vec<f64>,
    strings: Vec<String>,
    trues: usize,
    falses: usize,
    list_lengths: Vec<usize>,
    enum_counts: BTreeMap<String, usize>,
}

impl FieldStats {
    fn observe(&mut self, value: Option<&JsonValue>) {
        let Some(value) = value else {
            self.absent += 1;
            return;
        };
        self.present += 1;
        match value {
            JsonValue::Null => self.nulls += 1,
            JsonValue::Bool(true) => self.trues += 1,
            JsonValue::Bool(false) => self.falses += 1,
            JsonValue::Number(number) => {
                if let Some(as_f64) = number.as_f64() {
                    self.numbers.push(as_f64);
                }
            }
            JsonValue::String(text) => {
                *self.enum_counts.entry(text.clone()).or_insert(0) += 1;
                self.strings.push(text.clone());
            }
            JsonValue::Array(items) => self.list_lengths.push(items.len()),
            JsonValue::Object(_) => {}
        }
    }

    fn seen(&self) -> usize {
        self.present + self.absent
    }
}

fn check_values(entity: &EntityType, records: &[Record], report: &mut Report) {
    for field in entity.value_fields() {
        // A `*_count` naming a to-many holds the real width of a partition,
        // not a draw, so the value checks have nothing to say about it.
        if counted_relation(entity, field.name.as_str()).is_some() {
            continue;
        }
        let mut stats = FieldStats::default();
        for record in records {
            stats.observe(record.get(field.name.as_str()));
        }
        let subject = format!("{}.{}", entity.name, field.name);
        check_nullability(entity, field, &stats, &subject, report);
        check_list_length(field, &stats, &subject, report);
        check_enum(field, &stats, &subject, report);
        check_boolean(&stats, &subject, report);
        check_numbers(field, &stats, &subject, report);
        check_text(field, &stats, &subject, report);
    }
}

/// A field the schema said may be absent, that never is.
fn check_nullability(
    entity: &EntityType,
    field: &FieldDef,
    stats: &FieldStats,
    subject: &str,
    report: &mut Report,
) {
    const ENOUGH: usize = 10;

    if !field.may_be_missing() {
        return;
    }
    // A key is written on every record whatever the schema said about it: a
    // record that does not carry the value it is addressed by cannot be
    // fetched, listed or linked to. Reporting that as a field which is never
    // absent is reporting the store for keeping its own promise.
    if entity.key.iter().any(|part| part.field == field.name) {
        return;
    }
    if stats.seen() < ENOUGH {
        report.skip(
            Check::NeverAbsent,
            subject.to_string(),
            format!("{ENOUGH} records, world has {}", stats.seen()),
        );
        return;
    }
    if stats.nulls == 0 && stats.absent == 0 {
        report.fail(
            Check::NeverAbsent,
            subject.to_string(),
            format!("present and non-null in all {} record(s)", stats.present),
        );
    }
}

/// An array whose length never varies.
///
/// Relation arrays are excluded: their length is a real child count, which is
/// already heavy-tailed.
fn check_list_length(field: &FieldDef, stats: &FieldStats, subject: &str, report: &mut Report) {
    const ENOUGH: usize = 5;

    if !matches!(&field.value, ValueSpec::List(inner) if inner.relation().is_none()) {
        return;
    }
    if stats.list_lengths.len() < ENOUGH {
        report.skip(
            Check::ConstantListLength,
            subject.to_string(),
            format!("{ENOUGH} arrays, world has {}", stats.list_lengths.len()),
        );
        return;
    }
    let mut lengths: Vec<usize> = stats.list_lengths.clone();
    lengths.sort_unstable();
    lengths.dedup();
    if let [only] = lengths.as_slice() {
        report.fail(
            Check::ConstantListLength,
            subject.to_string(),
            format!(
                "every one of {} array(s) holds {only}",
                stats.list_lengths.len()
            ),
        );
    }
}

fn check_enum(field: &FieldDef, stats: &FieldStats, subject: &str, report: &mut Report) {
    let ValueSpec::Enum(members) = &field.value else {
        return;
    };
    if members.len() < 2 {
        return;
    }
    let counts: Vec<usize> = members
        .iter()
        .map(|member| stats.enum_counts.get(member.as_str()).copied().unwrap_or(0))
        .collect();
    let observed: usize = counts.iter().sum();
    // Below an expected count of five per member the chi-square approximation
    // is not one, and the world simply cannot express the difference.
    let enough = members.len() * 5;
    if observed < enough {
        report.skip(
            Check::UniformEnum,
            subject.to_string(),
            format!(
                "{enough} records for a {}-member enum, world has {observed}",
                members.len()
            ),
        );
        return;
    }
    let statistic = chi_square_uniform(&counts);
    let critical = chi_square_critical(members.len().saturating_sub(1));
    if statistic <= critical {
        report.fail(
            Check::UniformEnum,
            subject.to_string(),
            format!("chi-square {statistic:.2} against {critical:.2} over {observed} draw(s)"),
        );
    }
}

/// Whether a boolean reads as a fair coin.
///
/// The sample size is set by the power the test needs, not by what is
/// convenient. At thirty draws a genuinely lopsided field — two in three —
/// fails to reject fairness about half the time, so the check would fire on
/// exactly the worlds it was supposed to pass. Ninety is where a coin that far
/// off the middle is caught reliably, and a smaller world is reported as
/// unmeasurable rather than as broken.
fn check_boolean(stats: &FieldStats, subject: &str, report: &mut Report) {
    const ENOUGH: usize = 90;

    let n = stats.trues + stats.falses;
    if n == 0 {
        return;
    }
    if n < ENOUGH {
        report.skip(
            Check::FairCoin,
            subject.to_string(),
            format!("{ENOUGH} records, world has {n}"),
        );
        return;
    }
    // Two-sided normal approximation to the binomial at p = 1/2.
    let z = z_against_fair_coin(stats.trues, n);
    if z.abs() <= 1.96 {
        report.fail(
            Check::FairCoin,
            subject.to_string(),
            format!("{} true of {n}, z = {z:.2}", stats.trues),
        );
    }
}

fn check_numbers(field: &FieldDef, stats: &FieldStats, subject: &str, report: &mut Report) {
    const ENOUGH: usize = 10;

    let ValueSpec::Scalar(scalar) = &field.value else {
        return;
    };
    if !matches!(scalar.kind, ScalarKind::Int | ScalarKind::Float) {
        return;
    }
    // A declared bound is the spec's answer, not the generator's default.
    if scalar.constraints.max.is_some() {
        return;
    }
    if stats.numbers.len() < ENOUGH {
        report.skip(
            Check::NumberSupport,
            subject.to_string(),
            format!("{ENOUGH} values, world has {}", stats.numbers.len()),
        );
        return;
    }
    let largest = stats.numbers.iter().copied().fold(f64::MIN, f64::max);
    if largest <= DEFAULT_NUMBER_BOUND {
        report.fail(
            Check::NumberSupport,
            subject.to_string(),
            format!(
                "largest of {} value(s) is {largest:.0}, inside the default {DEFAULT_NUMBER_BOUND:.0}",
                stats.numbers.len()
            ),
        );
    }
}

/// The three things a string field gives away: its calendar, its clock and its
/// vocabulary.
fn check_text(field: &FieldDef, stats: &FieldStats, subject: &str, report: &mut Report) {
    check_day_of_month(stats, subject, report);
    check_clock(stats, subject, report);
    // A declared enum is a closed set by definition and a written date is a
    // calendar rather than a vocabulary; both are already measured above.
    if reads_as_prose(field) && !reads_as_dates(stats) {
        check_vocabulary(stats, subject, report);
    }
}

/// Whether a field holds prose, which is the only thing a vocabulary measures.
///
/// A closed set of words is the *right* answer for most string fields. A
/// `status` holds one of a handful of tokens, a slug is assembled from stems,
/// a MIME type comes from a registry — reporting those as a small vocabulary
/// is reporting the generator for doing its job, and it buries the fields
/// where the tell is real under fields where it is not.
fn reads_as_prose(field: &FieldDef) -> bool {
    let ValueSpec::Scalar(scalar) = &field.value else {
        return false;
    };
    if scalar.shape != TextShape::Prose {
        return false;
    }
    match &scalar.semantic {
        // Free text, whether the detector named it or declined to.
        None
        | Some(
            FieldType::Sentence | FieldType::Paragraph | FieldType::Name | FieldType::RandomString,
        ) => true,
        // Everything else is drawn from a format, a registry or an alphabet,
        // and its vocabulary is a property of that rather than of the writing.
        Some(_) => false,
    }
}

/// Whether a field's text is mostly written dates.
fn reads_as_dates(stats: &FieldStats) -> bool {
    if stats.strings.is_empty() {
        return false;
    }
    let dated = stats
        .strings
        .iter()
        .filter(|text| year_of(text).is_some())
        .count();
    ratio_of(dated, stats.strings.len()) > 0.5
}

fn check_day_of_month(stats: &FieldStats, subject: &str, report: &mut Report) {
    const ENOUGH: usize = 20;

    let days: Vec<u32> = stats
        .strings
        .iter()
        .filter_map(|text| day_of_month(text))
        .collect();
    if days.is_empty() {
        return;
    }
    if days.len() < ENOUGH {
        report.skip(
            Check::DayOfMonth,
            subject.to_string(),
            format!("{ENOUGH} dates, world has {}", days.len()),
        );
        return;
    }
    let latest = days.iter().copied().max().unwrap_or(0);
    if latest <= SHORTEST_MONTH {
        report.fail(
            Check::DayOfMonth,
            subject.to_string(),
            format!("latest of {} date(s) is the {latest}th", days.len()),
        );
    }
}

fn check_clock(stats: &FieldStats, subject: &str, report: &mut Report) {
    let mut years: Vec<i32> = stats.strings.iter().filter_map(|s| year_of(s)).collect();
    years.extend(stats.numbers.iter().filter_map(|n| epoch_year(*n)));
    if years.is_empty() {
        return;
    }
    let newest = years.iter().copied().max().unwrap_or(0);
    let today = chrono::Utc::now().year();
    if newest < today {
        report.fail(
            Check::StaleClock,
            subject.to_string(),
            format!(
                "newest of {} value(s) is {newest}; the year is {today}",
                years.len()
            ),
        );
    }
}

/// How much of a field's text is the same words over again.
///
/// A closed vocabulary collides fast — the birthday bound puts an even chance
/// of a repeat at seven draws from thirty-five — so the rate of new words
/// separates a generator from prose on a single page.
///
/// Measured in fixed-size segments and averaged, not over the whole corpus at
/// once. A plain type-token ratio falls as the sample grows, because the words
/// available run out while the tokens keep coming — Heaps' law puts the
/// distinct count near the square root of the total, so *real* English scores
/// about 0.15 over six thousand words. Comparing that against a constant
/// measures how much text an entity has rather than how varied it is, and a
/// field flags for being popular.
fn check_vocabulary(stats: &FieldStats, subject: &str, report: &mut Report) {
    /// Tokens per segment. Every segment is scored over the same number of
    /// draws, so the threshold means one thing at any corpus size.
    const SEGMENT: usize = 100;
    /// One full segment, so the number reported was actually measured.
    const ENOUGH: usize = SEGMENT;
    /// Prose over a hundred words repeats function words and little else.
    /// A generator drawing from a closed set lands far below this.
    const CLOSED_BELOW: f64 = 0.55;

    let tokens: Vec<String> = stats.strings.iter().flat_map(|text| words(text)).collect();
    if tokens.is_empty() {
        return;
    }
    if tokens.len() < ENOUGH {
        report.skip(
            Check::SmallVocabulary,
            subject.to_string(),
            format!("{ENOUGH} words, world has {}", tokens.len()),
        );
        return;
    }

    // A trailing partial segment is dropped rather than scored: fewer draws
    // score higher, and averaging that in would reward a short tail.
    let segments: Vec<f64> = tokens
        .chunks(SEGMENT)
        .filter(|segment| segment.len() == SEGMENT)
        .map(|segment| {
            let distinct: BTreeSet<&String> = segment.iter().collect();
            ratio_of(distinct.len(), segment.len())
        })
        .collect();
    let Some(count) = u32::try_from(segments.len()).ok().filter(|n| *n > 0) else {
        return;
    };
    let mean = segments.iter().sum::<f64>() / f64::from(count);

    if mean < CLOSED_BELOW {
        report.fail(
            Check::SmallVocabulary,
            subject.to_string(),
            format!(
                "{mean:.2} new words per {SEGMENT}, over {count} segment(s) of {} word(s)",
                tokens.len()
            ),
        );
    }
}

/// Whether an entity's keys, sorted as a client would sort them, put its
/// records in the order they were created.
fn check_id_time_order(entity: &EntityType, records: &[Record], report: &mut Report) {
    const ENOUGH: usize = 20;
    const AGREES_ABOVE: f64 = 0.5;

    let Some(field) = newest_timestamp_field(entity, records) else {
        return;
    };
    let mut pairs: Vec<(String, i64)> = records
        .iter()
        .filter_map(|record| {
            let stamp = record
                .get(&field)
                .and_then(JsonValue::as_str)
                .and_then(fake_data::instant_of)?;
            Some((record.key.to_string(), stamp))
        })
        .collect();
    let subject = format!("{}.{field}", entity.name);
    if pairs.len() < ENOUGH {
        report.skip(
            Check::IdTimeOrder,
            subject,
            format!("{ENOUGH} records, world has {}", pairs.len()),
        );
        return;
    }
    // The way a client would sort them: a numeric id sorts by its value, and
    // an opaque one by its text.
    let numeric = pairs.iter().all(|(key, _)| key.parse::<i128>().is_ok());
    if numeric {
        pairs.sort_by_key(|(key, _)| key.parse::<i128>().unwrap_or(0));
    } else {
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
    }

    let stamps: Vec<i64> = pairs.into_iter().map(|(_, stamp)| stamp).collect();
    // Signed, not absolute: an id that counts *backwards* through time is as
    // wrong as one that carries no time at all, and a client comparing two of
    // them would infer the opposite of the truth.
    let agreement = concordance(&stamps);
    if agreement < AGREES_ABOVE {
        report.fail(
            Check::IdTimeOrder,
            subject,
            format!(
                "id order and time agree {:.0}% of the time over {} record(s)",
                agreement * 100.0,
                stamps.len()
            ),
        );
    }
}

/// The first field of an entity that reads as an instant.
///
/// Probed over a handful of records rather than parsed over all of them for
/// every field: whether a field holds a timestamp is a fact about the field,
/// and the first few records settle it.
fn newest_timestamp_field(entity: &EntityType, records: &[Record]) -> Option<String> {
    const PROBE: usize = 4;

    // Emptiness is a fact about the records, not about each field: `all` over
    // nothing is true, so without this every field would read as an instant.
    if records.is_empty() {
        return None;
    }
    entity
        .value_fields()
        .map(|field| field.name.to_string())
        .find(|name| {
            records.iter().take(PROBE).all(|record| {
                record
                    .get(name)
                    .and_then(JsonValue::as_str)
                    .and_then(fake_data::instant_of)
                    .is_some()
            })
        })
}

fn check_relations(
    store: &EntityStore,
    entity: &EntityType,
    records: &[Record],
    report: &mut Report,
) {
    let graph = store.graph();
    for (field, relation) in entity.relations() {
        let subject = format!("{}.{}", entity.name, field.name);
        match relation.cardinality {
            Cardinality::One => {
                check_self_parent(store, entity, field, relation, records, &subject, report);
            }
            Cardinality::Many => {
                check_agreement(store, entity, field, records, &subject, report);
                for member in relation.concrete_targets() {
                    let Some(target) = graph.get(member.as_str()) else {
                        continue;
                    };
                    if is_membership(target, entity.name.as_str()) {
                        check_degree(store, entity, field, records, &subject, report);
                    } else {
                        check_contiguity(store, entity, member.as_str(), &subject, report);
                    }
                }
            }
        }
    }
    check_counts(store, entity, records, report);
}

/// A record whose own link resolves back to itself.
///
/// A self-relation partitions an entity's census against itself with nothing
/// excluding the child's own position, and the owning map is monotone over a
/// rising boundary vector, so a fixed point is not merely possible — it is
/// guaranteed.
fn check_self_parent(
    store: &EntityStore,
    entity: &EntityType,
    field: &FieldDef,
    relation: &Relation,
    records: &[Record],
    subject: &str,
    report: &mut Report,
) {
    if !relation
        .concrete_targets()
        .iter()
        .any(|target| target == &entity.name)
    {
        return;
    }
    let mut fixed = 0usize;
    let walked = records.len().min(RELATION_SAMPLE);
    for record in records.iter().take(RELATION_SAMPLE) {
        let resolved = store.relation_target(
            entity.name.as_str(),
            &record.key,
            field.name.as_str(),
            relation,
        );
        if resolved.is_some_and(|parent| parent.key == record.key) {
            fixed += 1;
        }
    }
    if fixed > 0 {
        report.fail(
            Check::SelfParent,
            subject.to_string(),
            format!(
                "{fixed} of {walked} record(s) are their own parent ({:.0}%)",
                ratio_of(fixed, walked) * 100.0
            ),
        );
    }
}

/// Whether following a link out and back lands where it started.
fn check_agreement(
    store: &EntityStore,
    entity: &EntityType,
    field: &FieldDef,
    records: &[Record],
    subject: &str,
    report: &mut Report,
) {
    let graph = store.graph();
    let mut disagreed = 0usize;
    let mut walked = 0usize;

    for record in records.iter().take(RELATION_SAMPLE) {
        let Ok(children) = store.related(
            entity.name.as_str(),
            &record.key,
            field.name.as_str(),
            &Selection::new(),
        ) else {
            continue;
        };
        for child in &children.records {
            let Some(target) = graph.get(child.entity.as_str()) else {
                continue;
            };
            let Some((back_field, back_relation)) = inverse_of(entity, target) else {
                continue;
            };
            walked += 1;
            let agrees = match back_relation.cardinality {
                Cardinality::One => store
                    .relation_target(
                        child.entity.as_str(),
                        &child.key,
                        back_field.name.as_str(),
                        back_relation,
                    )
                    .is_some_and(|parent| parent.key == record.key),
                Cardinality::Many => store
                    .related(
                        child.entity.as_str(),
                        &child.key,
                        back_field.name.as_str(),
                        &Selection::new(),
                    )
                    .is_ok_and(|back| back.records.iter().any(|other| other.key == record.key)),
            };
            if !agrees {
                disagreed += 1;
            }
        }
    }

    if disagreed > 0 {
        report.fail(
            Check::RelationDisagreement,
            subject.to_string(),
            format!("{disagreed} of {walked} link(s) do not resolve back"),
        );
    }
}

/// The field on `target` that reads the other end of the same link.
///
/// Routed the way the store routes it rather than by shape: an ownership is
/// answered by the functional carrier pointing back, and a membership by a
/// to-many that its own side also reads as a membership. A to-many whose
/// declared inverse does not exist — `Post.liked_by` beside a `User.posts`
/// that is really the inverse of `Post.author` — has no other end to compare
/// against, and pairing the two would report the schema as broken.
fn inverse_of<'a>(
    entity: &'a EntityType,
    target: &'a EntityType,
) -> Option<(&'a FieldDef, &'a Relation)> {
    if is_membership(target, entity.name.as_str()) {
        if !is_membership(entity, target.name.as_str()) {
            return None;
        }
        return target.relations().find(|(_, back)| {
            back.cardinality == Cardinality::Many
                && back
                    .concrete_targets()
                    .iter()
                    .any(|name| name == &entity.name)
        });
    }
    target
        .relations()
        .find(|(_, back)| back.cardinality == Cardinality::One && back.target == entity.name)
}

/// Whether a `*_count` field reports what its collection actually holds.
fn check_counts(store: &EntityStore, entity: &EntityType, records: &[Record], report: &mut Report) {
    for field in entity.value_fields() {
        let Some((counted, _)) = counted_relation(entity, field.name.as_str()) else {
            continue;
        };
        let mut disagreed = 0usize;
        let walked = records.len().min(RELATION_SAMPLE);
        for record in records.iter().take(RELATION_SAMPLE) {
            let Some(stated) = record.get(field.name.as_str()).and_then(JsonValue::as_u64) else {
                continue;
            };
            let Ok(held) = store.related(
                entity.name.as_str(),
                &record.key,
                counted.name.as_str(),
                &Selection::new(),
            ) else {
                continue;
            };
            if usize::try_from(stated).unwrap_or(usize::MAX) != held.total {
                disagreed += 1;
            }
        }
        if disagreed > 0 {
            report.fail(
                Check::CountDisagreement,
                format!("{}.{}", entity.name, field.name),
                format!(
                    "{disagreed} of {walked} record(s) disagree with `{}`",
                    counted.name
                ),
            );
        }
    }
}

/// Whether each parent's children form exactly one run of the default order.
///
/// The partition that makes counting arithmetic also files every child of one
/// parent side by side, so the number of runs of the parent key down an
/// unsorted page equals the number of distinct parents on it — a deterministic
/// identity rather than a statistic, and visible in one response.
fn check_contiguity(
    store: &EntityStore,
    entity: &EntityType,
    member: &str,
    subject: &str,
    report: &mut Report,
) {
    const ENOUGH: usize = 3;
    let Some(target) = store.graph().get(member) else {
        return;
    };
    let Some((back_field, back_relation)) = target
        .relations()
        .find(|(_, back)| back.cardinality == Cardinality::One && back.target == entity.name)
    else {
        return;
    };

    let Ok(page) = store.list(
        member,
        &Selection::new().paged(Page::Offset {
            skip: 0,
            take: CONTIGUITY_PAGE,
        }),
    ) else {
        return;
    };
    let owners: Vec<String> = page
        .records
        .iter()
        .filter_map(|child| {
            store
                .relation_target(member, &child.key, back_field.name.as_str(), back_relation)
                .map(|parent| parent.key.to_string())
        })
        .collect();

    let mut runs = 0usize;
    let mut previous: Option<&String> = None;
    for owner in &owners {
        if previous != Some(owner) {
            runs += 1;
        }
        previous = Some(owner);
    }
    let mut distinct: Vec<&String> = owners.iter().collect();
    distinct.sort();
    distinct.dedup();

    if distinct.len() < ENOUGH {
        report.skip(
            Check::ContiguousChildren,
            subject.to_string(),
            format!(
                "{ENOUGH} distinct parents on a page, world has {}",
                distinct.len()
            ),
        );
        return;
    }
    if runs == distinct.len() {
        report.fail(
            Check::ContiguousChildren,
            subject.to_string(),
            format!("{runs} run(s) for {} distinct parent(s)", distinct.len()),
        );
    }
}

/// Whether a many-to-many's degree histogram has more than two bars.
fn check_degree(
    store: &EntityStore,
    entity: &EntityType,
    field: &FieldDef,
    records: &[Record],
    subject: &str,
    report: &mut Report,
) {
    const ENOUGH: usize = 10;

    if records.len() < ENOUGH {
        report.skip(
            Check::MembershipDegree,
            subject.to_string(),
            format!("{ENOUGH} records, world has {}", records.len()),
        );
        return;
    }
    let mut degrees: Vec<usize> = records
        .iter()
        .take(RELATION_SAMPLE)
        .filter_map(|record| {
            store
                .related(
                    entity.name.as_str(),
                    &record.key,
                    field.name.as_str(),
                    &Selection::new(),
                )
                .ok()
                .map(|page| page.total)
        })
        .collect();
    let sampled = degrees.len();
    degrees.sort_unstable();
    degrees.dedup();
    if degrees.len() <= 2 {
        let bars: Vec<String> = degrees.iter().map(usize::to_string).collect();
        report.fail(
            Check::MembershipDegree,
            subject.to_string(),
            format!(
                "degree is only ever {} over {sampled} record(s)",
                bars.join(" or ")
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// Measurements
// ---------------------------------------------------------------------------

#[allow(
    clippy::cast_precision_loss,
    reason = "counts of sampled records, far below the f64 mantissa"
)]
fn ratio_of(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    part as f64 / whole as f64
}

#[allow(
    clippy::cast_precision_loss,
    reason = "counts of sampled records, far below the f64 mantissa"
)]
fn chi_square_uniform(counts: &[usize]) -> f64 {
    let total: usize = counts.iter().sum();
    if total == 0 || counts.is_empty() {
        return 0.0;
    }
    let expected = total as f64 / counts.len() as f64;
    counts
        .iter()
        .map(|observed| {
            let deviation = *observed as f64 - expected;
            deviation * deviation / expected
        })
        .sum()
}

/// The 95th percentile of a chi-square with `df` degrees of freedom.
///
/// Tabulated where an enum can actually reach and Wilson-Hilferty beyond it.
/// The closed form alone is 2% low at one degree of freedom, which is exactly
/// where a two-member enum lands, and a cutoff that is too low reads a skewed
/// draw as uniform.
#[allow(
    clippy::cast_precision_loss,
    reason = "degrees of freedom are a small enum length"
)]
fn chi_square_critical(df: usize) -> f64 {
    const TABLE: [f64; 30] = [
        3.841, 5.991, 7.815, 9.488, 11.070, 12.592, 14.067, 15.507, 16.919, 18.307, 19.675, 21.026,
        22.362, 23.685, 24.996, 26.296, 27.587, 28.869, 30.144, 31.410, 32.671, 33.924, 35.172,
        36.415, 37.652, 38.885, 40.113, 41.337, 42.557, 43.773,
    ];

    if df == 0 {
        return 0.0;
    }
    if let Some(exact) = df.checked_sub(1).and_then(|at| TABLE.get(at)) {
        return *exact;
    }
    let df = df as f64;
    let term = 2.0 / (9.0 * df);
    df * 1.644_853_6_f64.mul_add(term.sqrt(), 1.0 - term).powi(3)
}

/// How far a run of booleans is from a fair coin, in standard deviations.
#[allow(
    clippy::cast_precision_loss,
    reason = "counts of sampled records, far below the f64 mantissa"
)]
fn z_against_fair_coin(trues: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let n = total as f64;
    (trues as f64 - n / 2.0) / (n.sqrt() / 2.0)
}

/// How often a sequence rises rather than falls, on a scale where 1 is sorted
/// and 0 is a coin flip.
///
/// Kendall's tau over adjacent pairs only: the full statistic is quadratic and
/// the question here is whether an order exists at all, not how strong it is.
#[allow(
    clippy::cast_precision_loss,
    reason = "counts of sampled records, far below the f64 mantissa"
)]
fn concordance(values: &[i64]) -> f64 {
    let mut rising = 0usize;
    let mut falling = 0usize;
    for pair in values.windows(2) {
        match pair {
            [before, after] if after > before => rising += 1,
            [before, after] if after < before => falling += 1,
            _ => {}
        }
    }
    let compared = rising + falling;
    if compared == 0 {
        return 0.0;
    }
    (rising as f64 - falling as f64) / compared as f64
}

// ---------------------------------------------------------------------------
// Reading a date out of whatever the field wrote
// ---------------------------------------------------------------------------

/// `len` digits starting at character `from`, or nothing if they are not all
/// digits.
fn digits_at(text: &str, from: usize, len: usize) -> Option<u32> {
    let mut value: u32 = 0;
    let mut seen = 0usize;
    for ch in text.chars().skip(from).take(len) {
        value = value.checked_mul(10)?.checked_add(ch.to_digit(10)?)?;
        seen += 1;
    }
    (seen == len).then_some(value)
}

fn char_at(text: &str, at: usize) -> Option<char> {
    text.chars().nth(at)
}

/// The day of the month a written date names, in any format the generators
/// emit.
fn day_of_month(text: &str) -> Option<u32> {
    let day = if char_at(text, 4) == Some('-') && char_at(text, 7) == Some('-') {
        digits_at(text, 8, 2)?
    } else if matches!(char_at(text, 2), Some('/' | '.')) {
        digits_at(text, 0, 2)?
    } else if char_at(text, 3) == Some(',') {
        // `Tue, 07 Mar 2024 …`
        digits_at(text, 5, 2)?
    } else if text.chars().count() == 8 && digits_at(text, 0, 8).is_some() {
        digits_at(text, 6, 2)?
    } else {
        return None;
    };
    (1..=31).contains(&day).then_some(day)
}

/// The year a written date names.
#[allow(
    clippy::cast_possible_wrap,
    reason = "a four-digit year is inside i32 by construction"
)]
fn year_of(text: &str) -> Option<i32> {
    let year = if char_at(text, 4) == Some('-') && char_at(text, 7) == Some('-') {
        digits_at(text, 0, 4)?
    } else if matches!(char_at(text, 2), Some('/' | '.')) {
        digits_at(text, 6, 4)?
    } else if char_at(text, 3) == Some(',') {
        digits_at(text, 12, 4)?
    } else if text.chars().count() == 8 {
        digits_at(text, 0, 4)?
    } else {
        return None;
    };
    (1900..=2200).contains(&year).then_some(year as i32)
}

/// The year a number names, when the number is plausibly epoch seconds.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the range test above keeps the value inside i64"
)]
fn epoch_year(value: f64) -> Option<i32> {
    // 1971 through 2100, which is where a value being a date stops being a
    // coincidence.
    if !(31_536_000.0..=4_102_444_800.0).contains(&value) {
        return None;
    }
    chrono::DateTime::from_timestamp(value as i64, 0).map(|moment| moment.year())
}

/// The words of one value, lowercased, with punctuation dropped.
fn words(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.chars().count() > 1)
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests;
