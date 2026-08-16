//! Check whether ferrimock can read a HAR, and say so plainly.
//!
//! ```text
//! cargo run -p ferrimock-ml --example har-repro -- recording.har
//! cargo run -p ferrimock-ml --release --features ferrimock/scripting \
//!     --example har-repro -- recording.har
//! ```
//!
//! The second form is the one worth remembering. Enabling `scripting` pulls in a
//! bundler that turns on serde_json's `arbitrary_precision`, and Cargo unifies
//! features across the whole graph -- so a consumer that wants scripting changes
//! how *every* crate in the build sees a JSON number. The `har` crate tags its
//! version enum internally, which buffers through a `serde_json::Value`, and
//! under that feature each `f64` timing fails with "invalid type: map, expected
//! f64".
//!
//! Running this both ways is how that was found, and running it both ways is how
//! a regression in it would be found again.

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: har-repro <recording.har>");
        return;
    };

    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => {
            eprintln!("could not read {path}: {error}");
            return;
        }
    };

    match ferrimock::config::parse_har(&content) {
        Ok(har) => match har.log {
            har::Spec::V1_2(log) => println!("parsed {} entries (HAR 1.2)", log.entries.len()),
            har::Spec::V1_3(log) => println!("parsed {} entries (HAR 1.3)", log.entries.len()),
        },
        Err(error) => println!("FAILED: {error}"),
    }
}
