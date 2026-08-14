#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]
//! Replay a real HAR corpus in-process. Ignored by default; point
//! FERRIMOCK_CORPUS at a directory of `*/har.har` and run with --ignored.
//!
//! Each recording is converted to mocks, loaded, and then every request it
//! recorded is replayed through the matcher. Two numbers come out of that:
//! how much of the recording the mocks serve, and whether what they serve is
//! the response that was actually recorded for that request. The second is
//! the one that matters — a mock that answers with a neighbour's body is a
//! worse outcome than one that does not answer at all.

use ferrimock::consolidator::pattern::PatternDetector;
use ferrimock::engine::{MockMatcher, MockRegistry, suggest};
use ferrimock::types::{BodySource, UrlPattern};
use http::{HeaderMap, Method};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Why a replayed request found no mock.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MissKind {
    /// The recording holds no response for this request, so no mock was ever
    /// made for it. Nothing to fix in the engine.
    NotRecorded,
    /// A mock covers this exact method and path, but its query rejected the
    /// request.
    QueryOnly,
    /// A mock covers the same endpoint with different ids in the path.
    PathIds,
    /// Mocks exist at this path but none for this GraphQL operation.
    GraphQlOperation,
    /// Nothing in the recording addresses this endpoint.
    Absent,
}

impl MissKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotRecorded => "no response recorded",
            Self::QueryOnly => "query rejected",
            Self::PathIds => "path ids differ",
            Self::GraphQlOperation => "graphql operation absent",
            Self::Absent => "endpoint absent",
        }
    }
}

#[derive(Default)]
struct Stats {
    hars: usize,
    convert_failed: usize,
    load_failed: usize,
    entries: usize,
    recorded: usize,
    served: usize,
    missed: usize,
    wrong_body: usize,
    wrong_status: usize,
    wrong_body_own_live: usize,
    wrong_body_own_spent: usize,
    wrong_body_own_missing: usize,
    served_from_recorded: usize,
    missed_recorded: usize,
    suggestions: usize,
    covered_by_suggestion: usize,
    miss_kinds: BTreeMap<MissKind, usize>,
    /// Query parameter names implicated in a `QueryOnly` miss, and how often.
    query_culprits: HashMap<String, usize>,
    samples: Vec<String>,
}

/// How many worked examples the report prints when asked for them.
const SAMPLE_LIMIT: usize = 12;

impl Stats {
    /// Keep a worked example of something going wrong, for the report.
    ///
    /// Counts say how much is wrong, never what. Set FERRIMOCK_CORPUS_SAMPLES
    /// to see the request lines behind them; the closure means the formatting
    /// costs nothing on the runs that do not.
    fn sample(&mut self, describe: impl FnOnce() -> String) {
        if self.samples.len() < SAMPLE_LIMIT
            && std::env::var_os("FERRIMOCK_CORPUS_SAMPLES").is_some()
        {
            self.samples.push(describe());
        }
    }
}

/// What a mock declares, reduced to what the replay needs to reason about.
struct MockFacts {
    method: String,
    path: String,
    normalized_path: String,
    graphql: bool,
    query: BTreeMap<String, String>,
}

#[tokio::test]
#[ignore = "needs FERRIMOCK_CORPUS"]
async fn replay_corpus() {
    let root = std::env::var("FERRIMOCK_CORPUS").expect("FERRIMOCK_CORPUS");
    let mut s = Stats::default();

    let mut dirs: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    dirs.sort_by_key(std::fs::DirEntry::file_name);

    for entry in dirs {
        let har_path = entry.path().join("har.har");
        if !har_path.exists() {
            continue;
        }
        s.hars += 1;

        let content = std::fs::read_to_string(&har_path).unwrap();
        let Ok(har) = ferrimock::config::parse_har(&content) else {
            s.convert_failed += 1;
            continue;
        };

        let entries = match &har.log {
            har::Spec::V1_2(log) => log.entries.clone(),
            har::Spec::V1_3(_) => {
                s.convert_failed += 1;
                continue;
            }
        };

        let Ok(mocks) = ferrimock::config::HarLoader::new()
            .convert_har_to_mocks(har)
            .await
        else {
            s.convert_failed += 1;
            continue;
        };

        // The loader names each mock after the entry it came from, which is
        // how the replay tells a request the loader deliberately dropped
        // (a preflight, a static asset, a request that never completed) from
        // one it meant to cover.
        let recorded: HashSet<usize> = mocks
            .iter()
            .filter_map(|m| m.id.strip_prefix("har-entry-"))
            .filter_map(|n| n.parse::<usize>().ok())
            .map(|n| n - 1)
            .collect();

        let registry = MockRegistry::new();
        let mut load_failed = false;
        for m in mocks {
            let Ok(definition) = m.into_mock_definition().await else {
                load_failed = true;
                break;
            };
            registry.add_mock(definition);
        }
        if load_failed {
            s.load_failed += 1;
            continue;
        }

        let facts: Vec<MockFacts> = registry
            .get_all_mocks()
            .iter()
            .map(|m| MockFacts::of(m))
            .collect();

        let matcher = MockMatcher::new(registry.clone());
        matcher.set_track_unmatched(true);

        for (idx, e) in entries.iter().enumerate() {
            s.entries += 1;
            let is_recorded = recorded.contains(&idx);
            if is_recorded {
                s.recorded += 1;
            }

            let url = url::Url::parse(&e.request.url).ok();
            let (path, query) = url.as_ref().map_or(("/".to_string(), None), |u| {
                (u.path().to_string(), u.query().map(str::to_string))
            });
            let method = e.request.method.parse::<Method>().unwrap_or(Method::GET);
            let body = e.request.post_data.as_ref().and_then(|p| p.text.clone());

            let found = matcher.find_match(
                &method,
                &path,
                query.as_deref(),
                &HeaderMap::new(),
                body.as_deref().map(str::as_bytes),
            );

            if let Some(m) = found {
                s.served += 1;
                if is_recorded {
                    s.served_from_recorded += 1;
                }
                if let Some(status) = u16::try_from(e.response.status)
                    .ok()
                    .filter(|st| (100..=599).contains(st))
                    && m.mock.response.status != status
                {
                    s.wrong_status += 1;
                }
                // Only a recorded entry has a body to be right or wrong
                // about; one the loader dropped was answered by some
                // other entry's mock, which the miss taxonomy accounts
                // for rather than the correctness count.
                if is_recorded && let Some(served) = inline_body(&m.mock.response.body) {
                    let expected = e.response.content.text.as_deref().unwrap_or("");
                    if served != expected.as_bytes() {
                        s.wrong_body += 1;
                        // The mock built from this very entry is the one
                        // that should have answered. Whether it is still
                        // enabled says which way the sequence went wrong:
                        // it was skipped over, or it had already been used.
                        let own = format!("har-entry-{}", idx + 1);
                        match registry.get_mock(&own) {
                            Some(mock) if mock.enabled => s.wrong_body_own_live += 1,
                            Some(_) => s.wrong_body_own_spent += 1,
                            None => s.wrong_body_own_missing += 1,
                        }
                        s.sample(|| {
                                format!(
                                    "WRONG {method} {path}?{}\n  answered by {} {:?}\n  recorded as  {own} {:?}",
                                    query.as_deref().unwrap_or(""),
                                    m.mock.id,
                                    m.mock.request.url_patterns,
                                    registry.get_mock(&own).map(|k| k.request.url_patterns.clone()),
                                )
                            });
                    }
                }
            } else {
                s.missed += 1;
                if is_recorded {
                    s.missed_recorded += 1;
                }
                let kind = classify(
                    is_recorded,
                    &facts,
                    method.as_str(),
                    &path,
                    query.as_deref(),
                    &mut s.query_culprits,
                );
                *s.miss_kinds.entry(kind).or_default() += 1;
                if kind == MissKind::QueryOnly {
                    s.sample(|| {
                        let nearest: Vec<String> = facts
                            .iter()
                            .filter(|f| f.method == method.as_str() && f.path == path)
                            .take(3)
                            .map(|f| {
                                f.query
                                    .iter()
                                    .map(|(k, v)| format!("{k}={v}"))
                                    .collect::<Vec<_>>()
                                    .join("&")
                            })
                            .collect();
                        format!(
                            "MISS {method} {path}?{}\n  nearest mocks {}",
                            query.as_deref().unwrap_or(""),
                            nearest.join("  |  ")
                        )
                    });
                }
            }
        }

        let unmatched = registry.unmatched_requests();
        let sug = suggest(&matcher, &unmatched.requests);
        s.suggestions += sug.len();
        s.covered_by_suggestion += sug
            .iter()
            .filter_map(|g| usize::try_from(g.request_count).ok())
            .sum::<usize>();
    }

    report(&s);

    assert!(s.hars > 0, "no `*/har.har` found under {root}");
    assert_eq!(s.convert_failed, 0, "every recording must convert");
    assert_eq!(s.load_failed, 0, "every converted collection must load");
    // A mock made from a request must answer that request. This holds whatever
    // the corpus is, and it is the property every loader change here restored:
    // a query rewritten on the way in, or a credential taken out of the middle
    // of one, leaves a mock that cannot match what it was recorded from.
    assert_eq!(
        s.missed_recorded, 0,
        "{} recorded requests were not served by the mocks made from them",
        s.missed_recorded
    );
}

/// Whether a query parameter decides the response, measured rather than assumed.
///
/// For every group of recorded requests that share a method and path, this asks
/// of each parameter: when only that parameter moves, does the body move with
/// it? A parameter that changes the answer is identity-bearing and must keep
/// matching; one that never does is incidental and pinning it only costs
/// coverage. The counts also separate both from the case no query can explain —
/// the same request recorded twice with two different bodies.
#[tokio::test]
#[ignore = "needs FERRIMOCK_CORPUS"]
async fn corpus_query_variation() {
    let root = std::env::var("FERRIMOCK_CORPUS").expect("FERRIMOCK_CORPUS");

    let mut decides: HashMap<String, usize> = HashMap::new();
    let mut incidental: HashMap<String, usize> = HashMap::new();
    let mut identical_request_different_body = 0usize;
    let mut identical_request_repeated = 0usize;
    let mut groups = 0usize;

    let mut dirs: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    dirs.sort_by_key(std::fs::DirEntry::file_name);

    for dir in dirs {
        let har_path = dir.path().join("har.har");
        if !har_path.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&har_path).unwrap();
        let Ok(har) = ferrimock::config::parse_har(&content) else {
            continue;
        };
        let har::Spec::V1_2(log) = &har.log else {
            continue;
        };

        let mut by_endpoint: BTreeMap<(String, String), Vec<Observation>> = BTreeMap::new();
        for e in &log.entries {
            if !(100..=599).contains(&e.response.status) {
                continue;
            }
            let Ok(url) = url::Url::parse(&e.request.url) else {
                continue;
            };
            by_endpoint
                .entry((e.request.method.clone(), url.path().to_string()))
                .or_default()
                .push(Observation {
                    query: parse_query(url.query().unwrap_or("")),
                    body: e.response.content.text.clone().unwrap_or_default(),
                });
        }

        for observations in by_endpoint.values() {
            if observations.len() < 2 {
                continue;
            }
            groups += 1;

            for (i, a) in observations.iter().enumerate() {
                for b in observations.iter().skip(i + 1) {
                    let moved = differing_params(&a.query, &b.query);
                    let same_body = a.body == b.body;
                    match (moved.as_slice(), same_body) {
                        ([], true) => identical_request_repeated += 1,
                        // No parameter can explain the difference: the same
                        // request was recorded twice and answered differently.
                        // Only ordering separates these two responses.
                        ([], false) => identical_request_different_body += 1,
                        // Attribute only when exactly one parameter moved;
                        // with several, which one decided is not observable.
                        ([single], _) => {
                            let counter = if same_body {
                                &mut incidental
                            } else {
                                &mut decides
                            };
                            *counter.entry(single.clone()).or_default() += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    println!("\n===== QUERY PARAMETER VARIATION =====");
    println!("endpoint groups with >1 recording : {groups}");
    println!("same request, same body           : {identical_request_repeated}");
    println!("same request, different body      : {identical_request_different_body}");

    let mut names: Vec<&String> = decides.keys().chain(incidental.keys()).collect();
    names.sort_unstable();
    names.dedup();
    names.sort_by_key(|n| {
        std::cmp::Reverse(decides.get(*n).unwrap_or(&0) + incidental.get(*n).unwrap_or(&0))
    });

    println!(
        "\n{:<28} {:>10} {:>10}",
        "parameter", "decides", "incidental"
    );
    for name in names.iter().take(30) {
        println!(
            "{:<28} {:>10} {:>10}",
            name,
            decides.get(*name).unwrap_or(&0),
            incidental.get(*name).unwrap_or(&0)
        );
    }
}

struct Observation {
    query: BTreeMap<String, String>,
    body: String,
}

impl MockFacts {
    fn of(mock: &ferrimock::types::MockDefinition) -> Self {
        let literal = mock
            .request
            .url_patterns
            .iter()
            .find_map(|p| match p {
                UrlPattern::Exact(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("");
        let (path, raw_query) = literal
            .split_once('?')
            .map_or((literal, ""), |(p, q)| (p, q));

        Self {
            method: mock
                .request
                .methods
                .first()
                .map_or_else(|| "GET".to_string(), ToString::to_string),
            path: path.to_string(),
            normalized_path: PatternDetector::new().normalize_path_for_grouping(path),
            graphql: mock.request.graphql_matcher.is_some(),
            query: parse_query(raw_query),
        }
    }
}

fn parse_query(raw: &str) -> BTreeMap<String, String> {
    raw.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (k.to_string(), v.to_string())
        })
        .collect()
}

fn inline_body(source: &BodySource) -> Option<&[u8]> {
    match source {
        BodySource::Inline(bytes) | BodySource::FileCached(bytes) => Some(bytes.as_ref()),
        BodySource::File(_) | BodySource::Template { .. } | BodySource::Handler(_) => None,
    }
}

fn classify(
    is_recorded: bool,
    facts: &[MockFacts],
    method: &str,
    path: &str,
    query: Option<&str>,
    culprits: &mut HashMap<String, usize>,
) -> MissKind {
    if !is_recorded {
        return MissKind::NotRecorded;
    }

    let same_path: Vec<&MockFacts> = facts
        .iter()
        .filter(|f| f.method == method && f.path == path)
        .collect();

    if !same_path.is_empty() {
        if same_path.iter().all(|f| f.graphql) {
            return MissKind::GraphQlOperation;
        }
        let request_query = parse_query(query.unwrap_or(""));
        // Attribute the miss to the parameters that would have to move for the
        // closest same-path mock to accept it.
        if let Some(best) = same_path
            .iter()
            .min_by_key(|f| differing_params(&f.query, &request_query).len())
        {
            for name in differing_params(&best.query, &request_query) {
                *culprits.entry(name).or_default() += 1;
            }
        }
        return MissKind::QueryOnly;
    }

    let normalized = PatternDetector::new().normalize_path_for_grouping(path);
    if normalized != path
        && facts
            .iter()
            .any(|f| f.method == method && f.normalized_path == normalized)
    {
        return MissKind::PathIds;
    }

    MissKind::Absent
}

fn differing_params(
    mock: &BTreeMap<String, String>,
    request: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut names: Vec<String> = mock
        .iter()
        .filter(|(k, v)| request.get(*k) != Some(*v))
        .map(|(k, _)| k.clone())
        .collect();
    names.extend(
        request
            .keys()
            .filter(|k| !mock.contains_key(*k))
            .map(Clone::clone),
    );
    names.sort();
    names.dedup();
    names
}

fn report(s: &Stats) {
    // Counts here are request tallies, far below the point where f64 loses an
    // integer, and the result is only ever printed to one decimal place.
    #[allow(clippy::cast_precision_loss)]
    let pct = |n: usize, d: usize| {
        if d == 0 {
            0.0
        } else {
            100.0 * n as f64 / d as f64
        }
    };

    println!("\n===== CORPUS REPLAY =====");
    println!("HARs                   : {}", s.hars);
    println!("  convert failed       : {}", s.convert_failed);
    println!("  collection unloadable: {}", s.load_failed);
    println!("entries replayed       : {}", s.entries);
    println!(
        "  the loader mocked    : {} ({:.1}%)",
        s.recorded,
        pct(s.recorded, s.entries)
    );
    println!("\n-- all replayed entries --");
    println!(
        "served                 : {} ({:.1}%)",
        s.served,
        pct(s.served, s.entries)
    );
    println!(
        "missed                 : {} ({:.1}%)",
        s.missed,
        pct(s.missed, s.entries)
    );
    println!("\n-- entries the loader mocked --");
    println!(
        "served                 : {} ({:.1}%)",
        s.served_from_recorded,
        pct(s.served_from_recorded, s.recorded)
    );
    println!(
        "missed                 : {} ({:.1}%)",
        s.missed_recorded,
        pct(s.missed_recorded, s.recorded)
    );
    println!("\n-- correctness --");
    println!(
        "wrong body             : {} ({:.2}% of served)",
        s.wrong_body,
        pct(s.wrong_body, s.served_from_recorded)
    );
    println!(
        "wrong status           : {} ({:.2}% of served)",
        s.wrong_status,
        pct(s.wrong_status, s.served)
    );
    println!("  own mock still live  : {}", s.wrong_body_own_live);
    println!("  own mock already used: {}", s.wrong_body_own_spent);
    println!("  own mock absent      : {}", s.wrong_body_own_missing);
    println!("\n-- why requests missed --");
    for (kind, count) in &s.miss_kinds {
        println!(
            "  {:<24}: {} ({:.1}%)",
            kind.as_str(),
            count,
            pct(*count, s.missed)
        );
    }
    println!("\n-- query parameters implicated in a rejected query --");
    let mut culprits: Vec<_> = s.query_culprits.iter().collect();
    culprits.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    for (name, count) in culprits.iter().take(25) {
        println!("  {name:<28}: {count}");
    }
    for sample in &s.samples {
        println!("\n{sample}");
    }
    println!("\n-- suggestions --");
    println!("suggestions            : {}", s.suggestions);
    println!(
        "  requests they cover  : {} ({:.1}% of misses)",
        s.covered_by_suggestion,
        pct(s.covered_by_suggestion, s.missed)
    );
}
