//! Path normalization must terminate, keep its shape, and settle.
//!
//! It runs on whatever a recording carried, which is to say on arbitrary bytes
//! that once passed for a URL path.

#![no_main]

use ferrimock::consolidator::pattern::PatternDetector;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|path: String| {
    let detector = PatternDetector::new();

    let once = detector.normalize_path_for_grouping(&path);
    let twice = detector.normalize_path_for_grouping(&once);

    // Normalizing a normalized path must change nothing. Without this a group
    // key would depend on how many times the path had been through, and mocks
    // recorded in one session would stop matching those from another.
    assert_eq!(once, twice, "normalization did not settle for {path:?}");

    // Segment count is the skeleton every caller aligns against -- pattern
    // generation walks the original and the normalized path in step.
    assert_eq!(
        path.split('/').count(),
        once.split('/').count(),
        "normalization changed the segment count of {path:?}"
    );
});
