//! Ask the built-in detector what it makes of one field.
//!
//! The companion to `ferrimock-ml audit`. The audit says a class is wrong and
//! how often; this says why, for one field, in one run -- which is what closing
//! a defect actually needs.
//!
//! ```text
//! cargo run -p ferrimock-ml --example detector-probe -- <name> <value>...
//! cargo run -p ferrimock-ml --example detector-probe -- --number total 42 7
//! ```
//!
//! `--number` and `--boolean` say the values were recorded as that JSON kind
//! rather than as text. It changes the answer, and it is meant to: a count and a
//! numeric string id are the same digits.

use ferrimock::type_detector::TypeDetector;
use ferrimock_ml::{Field, ValueKind, detector};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    let mut kind = ValueKind::String;
    let mut rest: Vec<String> = Vec::new();
    for argument in arguments {
        match argument.as_str() {
            "--number" => kind = ValueKind::Number,
            "--boolean" => kind = ValueKind::Boolean,
            _ => rest.push(argument),
        }
    }

    let Some((name, values)) = rest.split_first() else {
        eprintln!(
            "usage: detector-probe [--number|--boolean] <field-name> <value>...\n\
             \n\
             example: detector-probe reference 01ARZ3NDEKTSV4RRFFQ69G5FAV 01BX5ZZKBKACTAV9WEVGEMMVRZ"
        );
        return;
    };
    if values.is_empty() {
        eprintln!("give at least one value to look at");
        return;
    }

    let borrowed: Vec<&str> = values.iter().map(String::as_str).collect();
    let field = Field::new(name, &borrowed).of_kind(kind);

    let (field_type, confidence) = detector::detect(&TypeDetector::new(), &field);
    println!("field    {name}  (recorded as {kind:?})");
    for value in &borrowed {
        println!("  value  {value}");
    }
    println!(
        "detector {}  ({confidence:.2})",
        detector::kind_of(&field_type)
    );
    println!(
        "template {}",
        ferrimock::codegen::field_type_to_tera_expr(name, &field_type)
    );
}
