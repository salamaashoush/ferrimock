//! Generating a value that satisfies a declared `pattern`.
//!
//! A spec that writes `pattern: "^[A-Z]{3}-[0-9]{4}$"` has stated the answer
//! more precisely than any name-based guess can, and a client validating the
//! response will hold it to exactly that. So the pattern is honoured — but only
//! as a fallback: a field that already produces something realistic *and*
//! matching keeps what it had, because `^.+$` is a pattern too and answering it
//! with a random letter would be a worse mock than a sentence.
//!
//! The generator walks the parsed regex rather than sampling and testing:
//! rejection sampling never terminates on anything specific.

use dashmap::DashMap;
use lean_string::LeanString;
use regex::Regex;
use regex_syntax::hir::{Class, Hir, HirKind};
use std::sync::{Arc, OnceLock};

use crate::fake_data::rng;

/// How many extra repetitions an unbounded quantifier is allowed on the first
/// attempt.
///
/// `a+` has to stop somewhere, and a mock is more useful short than long — but
/// a `minLength` the short walk cannot reach raises this, which is what later
/// attempts are for.
const MAX_EXTRA_REPEATS: u32 = 4;

/// How many walks are tried before giving up.
///
/// Each one draws differently, so an alternation that happened to pick a branch
/// the length bounds rule out gets another go rather than the field falling
/// back to a value the pattern rejects.
const ATTEMPTS: usize = 6;

/// The longest value the walk will build before it gives up.
///
/// A nested quantifier multiplies out fast, and a spec asking for a megabyte of
/// `a` is a spec error rather than a value anyone wants generated.
const MAX_LEN: usize = 4096;

/// A pattern is compiled and parsed once per process; a document reuses the
/// same handful across every instance of every entity.
struct Compiled {
    regex: Option<Regex>,
    hir: Option<Hir>,
}

fn cache() -> &'static DashMap<LeanString, Arc<Compiled>> {
    static CACHE: OnceLock<DashMap<LeanString, Arc<Compiled>>> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
}

fn compiled(pattern: &str) -> Arc<Compiled> {
    if let Some(found) = cache().get(pattern) {
        return Arc::clone(&found);
    }
    let entry = Arc::new(Compiled {
        regex: Regex::new(pattern).ok(),
        hir: regex_syntax::parse(pattern).ok(),
    });
    cache().insert(LeanString::from(pattern), Arc::clone(&entry));
    entry
}

/// Whether a value already satisfies a pattern.
///
/// A pattern a regex engine cannot compile is treated as satisfied: refusing
/// every value because the spec's dialect is not Rust's would replace a
/// cosmetic mismatch with an empty field.
#[must_use]
pub fn matches(pattern: &str, value: &str) -> bool {
    compiled(pattern)
        .regex
        .as_ref()
        .is_none_or(|regex| regex.is_match(value))
}

/// A value the pattern accepts, within whatever length the spec also asked for.
///
/// The two can disagree — `^[a-z]+$` with `minLength: 40` needs a long walk, and
/// `^[a-z]{2,64}$` with `maxLength: 8` needs a short one — so the walk is tried
/// several times with a widening repetition budget. A value that satisfies the
/// pattern but misses the length is still returned in preference to nothing:
/// the pattern is the stricter statement about what the field holds.
#[must_use]
pub fn generate(pattern: &str, min_len: Option<usize>, max_len: Option<usize>) -> Option<String> {
    let entry = compiled(pattern);
    let hir = entry.hir.as_ref()?;

    let mut unbounded: Option<String> = None;
    for attempt in 0..ATTEMPTS {
        let mut out = String::new();
        write_hir(hir, &mut out, extra_repeats(attempt, min_len));
        // The walk is exact for everything it handles, so a value it produced
        // that the engine then rejects means the pattern used a construct the
        // walk approximated. Try again rather than answering with it.
        if out.len() > MAX_LEN || !matches(pattern, &out) {
            continue;
        }
        let length = out.chars().count();
        if min_len.is_some_and(|min| length < min) || max_len.is_some_and(|max| length > max) {
            unbounded.get_or_insert(out);
            continue;
        }
        return Some(out);
    }
    unbounded
}

/// How much slack a quantifier gets on one attempt.
///
/// Widening rather than jumping: most patterns are satisfied by the shortest
/// walk, and a `minLength` is the only reason to grow one.
fn extra_repeats(attempt: usize, min_len: Option<usize>) -> u32 {
    let wanted = u32::try_from(min_len.unwrap_or(0)).unwrap_or(u32::MAX);
    let widened = MAX_EXTRA_REPEATS.saturating_mul(u32::try_from(attempt).unwrap_or(1) + 1);
    widened.max(if attempt == 0 { 0 } else { wanted })
}

fn write_hir(hir: &Hir, out: &mut String, extra: u32) {
    if out.len() > MAX_LEN {
        return;
    }
    match hir.kind() {
        // A look-around asserts a position rather than contributing a
        // character; `^`, `$` and `\b` all leave the value alone.
        HirKind::Empty | HirKind::Look(_) => {}
        HirKind::Literal(literal) => {
            out.push_str(&String::from_utf8_lossy(&literal.0));
        }
        HirKind::Class(class) => {
            if let Some(ch) = pick_from_class(class) {
                out.push(ch);
            }
        }
        HirKind::Repetition(repetition) => {
            let count = repeat_count(repetition.min, repetition.max, extra);
            for _ in 0..count {
                write_hir(&repetition.sub, out, extra);
                if out.len() > MAX_LEN {
                    return;
                }
            }
        }
        HirKind::Capture(capture) => write_hir(&capture.sub, out, extra),
        HirKind::Concat(parts) => {
            for part in parts {
                write_hir(part, out, extra);
                if out.len() > MAX_LEN {
                    return;
                }
            }
        }
        HirKind::Alternation(branches) => {
            if let Some(branch) = pick(branches) {
                write_hir(branch, out, extra);
            }
        }
    }
}

fn repeat_count(min: u32, max: Option<u32>, extra: u32) -> u32 {
    let ceiling = max.unwrap_or_else(|| min.saturating_add(extra));
    if ceiling <= min {
        return min;
    }
    // Bounded so `{2,255}` does not answer with 255 characters when 2 says
    // everything the client needs to see.
    let ceiling = ceiling.min(min.saturating_add(extra));
    if ceiling <= min {
        return min;
    }
    min.saturating_add(u32::try_from(next_index(u64::from(ceiling - min) + 1)).unwrap_or(0))
}

fn pick<T>(options: &[T]) -> Option<&T> {
    options.get(next_index(options.len() as u64))
}

/// A character the class admits, preferring one a person would recognise.
///
/// `\w` spans every Unicode word character; answering it with a Han ideograph
/// is correct and useless, so the printable ASCII part of the class is tried
/// first and the full range is the fallback for a class that has none.
fn pick_from_class(class: &Class) -> Option<char> {
    let ranges: Vec<(u32, u32)> = match class {
        Class::Unicode(unicode) => unicode
            .ranges()
            .iter()
            .map(|range| (range.start() as u32, range.end() as u32))
            .collect(),
        Class::Bytes(bytes) => bytes
            .ranges()
            .iter()
            .map(|range| (u32::from(range.start()), u32::from(range.end())))
            .collect(),
    };
    if ranges.is_empty() {
        return None;
    }

    let readable: Vec<(u32, u32)> = ranges
        .iter()
        .filter_map(|&(start, end)| {
            let start = start.max(0x21);
            let end = end.min(0x7e);
            (start <= end).then_some((start, end))
        })
        .collect();

    let candidates = if readable.is_empty() {
        &ranges
    } else {
        &readable
    };
    let total: u64 = candidates
        .iter()
        .map(|&(start, end)| u64::from(end - start) + 1)
        .sum();
    let mut offset = next_index(total) as u64;
    for &(start, end) in candidates {
        let width = u64::from(end - start) + 1;
        if offset < width {
            return char::from_u32(start + u32::try_from(offset).ok()?);
        }
        offset -= width;
    }
    None
}

/// An index below `len`, drawn from whatever stream is installed — so a value
/// generated from a pattern is as reproducible as every other generated value.
fn next_index(len: u64) -> usize {
    use rand::RngExt as _;
    if len == 0 {
        return 0;
    }
    usize::try_from(rng::rng().random_range(0..len)).unwrap_or(0)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::*;

    fn generated(pattern: &str) -> String {
        let _scope = rng::scope_seeded(7);
        generate(pattern, None, None).unwrap_or_else(|| panic!("`{pattern}` should be generatable"))
    }

    fn generated_within(pattern: &str, min: Option<usize>, max: Option<usize>) -> String {
        let _scope = rng::scope_seeded(7);
        generate(pattern, min, max).unwrap_or_else(|| panic!("`{pattern}` should be generatable"))
    }

    #[test]
    fn a_literal_pattern_is_reproduced() {
        assert_eq!(generated("^ADD_METADATA$"), "ADD_METADATA");
    }

    #[test]
    fn a_counted_class_gets_exactly_that_many() {
        let value = generated("^[A-Z]{3}-[0-9]{4}$");
        assert!(
            Regex::new("^[A-Z]{3}-[0-9]{4}$").unwrap().is_match(&value),
            "{value} does not satisfy the pattern"
        );
    }

    #[test]
    fn an_alternation_picks_a_branch() {
        let value = generated("^(red|green|blue)$");
        assert!(
            ["red", "green", "blue"].contains(&value.as_str()),
            "{value}"
        );
    }

    #[test]
    fn an_unbounded_quantifier_terminates() {
        let value = generated("^a+$");
        assert!(!value.is_empty() && value.len() <= 8, "{value}");
        assert!(value.chars().all(|c| c == 'a'));
    }

    #[test]
    fn optional_and_nested_groups_are_handled() {
        let pattern = r"^\d{3}(-\d{2})?$";
        let value = generated(pattern);
        assert!(Regex::new(pattern).unwrap().is_match(&value), "{value}");
    }

    #[test]
    fn a_word_class_answers_with_something_readable() {
        let value = generated(r"^\w{6}$");
        assert_eq!(value.chars().count(), 6);
        assert!(value.is_ascii(), "{value} should be readable ASCII");
    }

    #[test]
    fn generation_is_reproducible_for_a_seed() {
        let once = generated("^[a-z]{10}$");
        let twice = generated("^[a-z]{10}$");
        assert_eq!(once, twice);
    }

    #[test]
    fn a_length_bound_is_met_alongside_the_pattern() {
        let value = generated_within("^[a-z]+$", Some(24), Some(40));
        assert!(
            (24..=40).contains(&value.chars().count()),
            "length {} outside the declared bounds: {value}",
            value.chars().count()
        );
        assert!(value.chars().all(|c| c.is_ascii_lowercase()));
    }

    #[test]
    fn a_short_bound_keeps_the_walk_short() {
        let value = generated_within("^[a-z]{2,64}$", None, Some(8));
        assert!(value.chars().count() <= 8, "{value}");
    }

    #[test]
    fn an_uncompilable_pattern_is_refused_rather_than_guessed() {
        assert!(generate("(?<broken", None, None).is_none());
    }

    #[test]
    fn matching_treats_an_uncompilable_pattern_as_satisfied() {
        assert!(matches("(?<broken", "anything"));
    }
}
