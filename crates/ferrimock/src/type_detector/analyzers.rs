//! Analysis functions for numeric and structured data types

use rustc_hash::FxHashSet;
use serde_json::Value as JsonValue;

use super::types::{ArrayPattern, FieldType, ObjectAnalysis};

/// Analyze numeric values (both integers and floats)
pub(super) fn analyze_numbers(values: &[&JsonValue]) -> (FieldType, f64) {
    // Try integers first
    let integers: Vec<i64> = values.iter().filter_map(|v| v.as_i64()).collect();

    // If all values are integers, analyze as integers
    if integers.len() == values.len() {
        return analyze_integers(&integers);
    }

    // Otherwise, try floats
    let floats: Vec<f64> = values.iter().filter_map(|v| v.as_f64()).collect();

    if floats.is_empty() {
        return (
            FieldType::RandomNumber {
                min: None,
                max: None,
            },
            0.5,
        );
    }

    analyze_floats(&floats)
}

/// Analyze integer values
pub(super) fn analyze_integers(numbers: &[i64]) -> (FieldType, f64) {
    if numbers.is_empty() {
        return (
            FieldType::RandomNumber {
                min: None,
                max: None,
            },
            0.8,
        );
    }

    // Check for microsecond timestamps (16 digits, starts with 1)
    // Example: 1640000000000000 (2021-12-20 in microseconds)
    let min_micro = 1_000_000_000_000_000_i64; // 2001-09-09 in microseconds
    let max_micro = 2_000_000_000_000_000_i64; // 2033-05-18 in microseconds
    if numbers.iter().all(|&n| n >= min_micro && n < max_micro) {
        return (FieldType::MicrosecondTimestamp, 0.90);
    }

    // Check for millisecond timestamps (13 digits, starts with 1)
    // Example: 1640000000000 (2021-12-20 in milliseconds)
    let min_milli = 1_000_000_000_000_i64; // 2001-09-09 in milliseconds
    let max_milli = 2_000_000_000_000_i64; // 2033-05-18 in milliseconds
    if numbers.iter().all(|&n| n >= min_milli && n < max_milli) {
        return (FieldType::MillisecondTimestamp, 0.90);
    }

    // Check for Unix timestamps in seconds (10 digits, reasonable range: 2000-2040)
    let min_timestamp = 946_684_800_i64; // 2000-01-01
    let max_timestamp = 2_208_988_800_i64; // 2040-01-01
    if numbers
        .iter()
        .all(|&n| n >= min_timestamp && n <= max_timestamp)
    {
        return (FieldType::UnixTimestamp, 0.85);
    }

    // FileSize detection removed - too many false positives with generic numeric fields
    // Semantic detection (field name-based) handles FileSize more accurately

    // Check for sequential pattern
    if let Some((&first, rest)) = numbers.split_first()
        && let Some(&second) = rest.first()
    {
        let step = second - first;
        let is_sequential = numbers
            .windows(2)
            .all(|w| matches!((w.first(), w.get(1)), (Some(a), Some(b)) if b - a == step));

        if is_sequential && step != 0 {
            return (FieldType::SequentialNumber { start: first, step }, 0.95);
        }
    }

    // Extract min/max for random numbers
    let min = numbers.iter().min().copied();
    let max = numbers.iter().max().copied();

    (FieldType::RandomNumber { min, max }, 0.8)
}

/// Analyze floating point values
pub(super) fn analyze_floats(floats: &[f64]) -> (FieldType, f64) {
    if floats.is_empty() {
        return (
            FieldType::RandomFloat {
                min: None,
                max: None,
            },
            0.8,
        );
    }

    // Check for latitude (-90.0 to 90.0)
    if floats.iter().all(|&f| (-90.0..=90.0).contains(&f)) && floats.iter().any(|&f| f.abs() > 0.0)
    {
        // If ALL values are in latitude range and not all zero
        // This has higher confidence if field name also suggests it (via semantic boost)
        return (FieldType::Latitude, 0.70);
    }

    // Check for longitude (-180.0 to 180.0)
    if floats.iter().all(|&f| (-180.0..=180.0).contains(&f))
        && floats.iter().any(|&f| f.abs() > 0.0)
    {
        // Longitude is more ambiguous than latitude
        return (FieldType::Longitude, 0.65);
    }

    // Extract min/max for random floats
    let min = floats
        .iter()
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .copied();
    let max = floats
        .iter()
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .copied();

    (FieldType::RandomFloat { min, max }, 0.8)
}

/// Positions kept when an array's elements are not all the same shape.
/// Real listings run to hundreds; the fidelity check probes the head, and a
/// template that spelled out every element would be unreadable.
const MAX_MIXED_ELEMENTS: usize = 12;

/// Analyze array patterns to detect homogeneous structures
pub(super) fn analyze_array_pattern<F>(
    values: &[&JsonValue],
    detect_type_from_values: F,
) -> (FieldType, f64)
where
    F: Fn(&[&JsonValue]) -> (FieldType, f64),
{
    let arrays: Vec<&Vec<JsonValue>> = values.iter().filter_map(|v| v.as_array()).collect();

    if arrays.is_empty() {
        return (
            FieldType::Array(Box::new(ArrayPattern {
                element_type: FieldType::RandomString,
                is_homogeneous: false,
                sample_size_range: (0, 0),
                element_shapes: Vec::new(),
            })),
            0.5,
        );
    }

    // Calculate size range
    let sizes: Vec<usize> = arrays.iter().map(|a| a.len()).collect();
    let min_size = *sizes.iter().min().unwrap_or(&0);
    let max_size = *sizes.iter().max().unwrap_or(&0);

    // If all arrays are empty
    if max_size == 0 {
        return (
            FieldType::Array(Box::new(ArrayPattern {
                element_type: FieldType::RandomString,
                is_homogeneous: true,
                sample_size_range: (0, 0),
                element_shapes: Vec::new(),
            })),
            1.0,
        );
    }

    // Sample evenly across all arrays to avoid bias towards first array
    let samples_per_array = (20 / arrays.len().max(1)).max(1);
    let element_samples: Vec<&JsonValue> = arrays
        .iter()
        .flat_map(|a| {
            a.iter()
                .filter(|v| !v.is_null()) // Ignore nulls
                .take(samples_per_array) // Take even samples from each array
        })
        .collect();

    if element_samples.is_empty() {
        return (
            FieldType::Array(Box::new(ArrayPattern {
                element_type: FieldType::RandomString,
                is_homogeneous: false,
                sample_size_range: (min_size, max_size),
                element_shapes: Vec::new(),
            })),
            0.5,
        );
    }

    // Detect type from the larger, more representative sample
    let (element_type, confidence) = detect_type_from_values(&element_samples);

    // Whether the elements are the same shape, not merely the same kind. Two
    // objects are both `FieldType::Object` however little they have in common,
    // so comparing the detected type would call a listing of files and folders
    // homogeneous and then answer every element with the union of their fields.
    let shape_of = |value: &JsonValue| -> Option<Vec<String>> {
        value.as_object().map(|fields| {
            let mut names: Vec<String> = fields.keys().cloned().collect();
            names.sort_unstable();
            names
        })
    };

    let mut shapes = element_samples.iter().filter_map(|value| shape_of(value));
    let first_shape = shapes.next();
    let objects_agree = match first_shape {
        Some(ref first) => shapes.all(|shape| shape == *first),
        // Nothing object-shaped to disagree about.
        None => true,
    };

    let is_homogeneous = objects_agree
        && arrays.iter().all(|arr| {
            if arr.is_empty() {
                return true;
            }
            let sample: Vec<&JsonValue> = arr.iter().take(5).collect();
            let (detected, _) = detect_type_from_values(&sample);
            std::mem::discriminant(&detected) == std::mem::discriminant(&element_type)
        });

    // A mixed array is answered position by position, from the longest
    // recording -- the one that shows the most of the sequence.
    let element_shapes = if is_homogeneous {
        Vec::new()
    } else {
        arrays
            .iter()
            .max_by_key(|arr| arr.len())
            .map(|arr| {
                arr.iter()
                    .take(MAX_MIXED_ELEMENTS)
                    .map(|element| detect_type_from_values(&[element]).0)
                    .collect()
            })
            .unwrap_or_default()
    };

    (
        FieldType::Array(Box::new(ArrayPattern {
            element_type,
            is_homogeneous,
            sample_size_range: (min_size, max_size),
            element_shapes,
        })),
        confidence,
    )
}

/// Analyze nested object patterns
pub(super) fn analyze_object_pattern(
    values: &[&JsonValue],
    detector: &super::TypeDetector,
) -> (FieldType, f64) {
    let objects: Vec<&serde_json::Map<String, JsonValue>> =
        values.iter().filter_map(|v| v.as_object()).collect();

    if objects.is_empty() {
        return (
            FieldType::Object(Box::new(ObjectAnalysis {
                varying_fields: vec![],
                constant_fields: vec![],
            })),
            0.5,
        );
    }

    // Collect all field names
    let mut all_fields = FxHashSet::default();
    for obj in &objects {
        all_fields.extend(obj.keys().cloned());
    }

    let mut varying_fields = Vec::new();
    let mut constant_fields = Vec::new();

    // Analyze each field
    for field in all_fields {
        let field_values: Vec<&JsonValue> =
            objects.iter().filter_map(|obj| obj.get(&field)).collect();

        if field_values.is_empty() {
            continue;
        }

        // A field most of the samples did not carry at all is answered without
        // it, and one most of them left null is answered null. Both are the same
        // rule: reproduce the shape the recording usually had, since that is the
        // one a client will most often be checking against.
        if field_values.len() * 2 <= objects.len() {
            continue;
        }
        if field_values.iter().filter(|value| value.is_null()).count() * 2 > field_values.len() {
            constant_fields.push((field.clone(), JsonValue::Null));
            continue;
        }

        // Check if all values are the same
        let all_same = field_values
            .windows(2)
            .all(|w| w.first().zip(w.get(1)).is_some_and(|(a, b)| a == b));

        // One sample is agreement with itself, not evidence that the field is
        // fixed. Asked to generalize, the detector reads the value instead.
        let single = field_values.len() == 1;
        if all_same && !(single && detector.generalizes()) {
            if let Some(first_val) = field_values.first() {
                constant_fields.push((field.clone(), (*first_val).clone()));
            }
            continue;
        }

        if single && let Some(value) = field_values.first() {
            match detector.classify_single(&field, value) {
                Some(field_type) => varying_fields.push((field, field_type)),
                None => constant_fields.push((field.clone(), (*value).clone())),
            }
            continue;
        }

        let (field_type, _) = detector.detect_type(&field, &field_values);
        varying_fields.push((field, field_type));
    }

    (
        FieldType::Object(Box::new(ObjectAnalysis {
            varying_fields,
            constant_fields,
        })),
        0.9,
    )
}
