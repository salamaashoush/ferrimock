//! Statistical feature extraction for type detection

use rustc_hash::FxHashMap;

use super::constants::{EMAIL_REGEX, UUID_REGEX};

/// Statistical features extracted from string values
/// Simplified from Sherlock's 1,588 features to the most impactful ones
#[derive(Debug, Clone)]
pub struct TypeFeatures {
  /// Shannon entropy of character distribution
  pub char_entropy: f64,
  /// Percentage of digit characters
  pub digit_ratio: f64,
  /// Percentage of alphabetic characters
  pub alpha_ratio: f64,
  /// Percentage of special characters
  pub special_char_ratio: f64,
  /// Average string length
  pub avg_length: f64,
  /// Variance in string lengths
  pub length_variance: f64,
  /// Minimum string length
  pub min_length: usize,
  /// Maximum string length
  pub max_length: usize,
  /// How consistent the format is across samples (0.0 to 1.0)
  pub format_consistency: f64,
  /// Contains http:// or https:// protocol
  pub has_protocol: bool,
  /// Matches UUID format pattern
  pub has_uuid_format: bool,
  /// Contains @ symbol (email indicator)
  pub has_email_at: bool,
  /// Contains dots in URL-like positions
  pub has_url_dots: bool,
  /// Contains file extension pattern
  pub has_file_extension: bool,
  /// All uppercase (potential constant/enum)
  pub is_all_uppercase: bool,
  /// Contains hyphen-separated segments (UUID/date indicator)
  pub has_hyphen_segments: bool,
}

impl Default for TypeFeatures {
  fn default() -> Self {
    Self {
      char_entropy: 0.0,
      digit_ratio: 0.0,
      alpha_ratio: 0.0,
      special_char_ratio: 0.0,
      avg_length: 0.0,
      length_variance: 0.0,
      min_length: 0,
      max_length: 0,
      format_consistency: 0.0,
      has_protocol: false,
      has_uuid_format: false,
      has_email_at: false,
      has_url_dots: false,
      has_file_extension: false,
      is_all_uppercase: false,
      has_hyphen_segments: false,
    }
  }
}

/// Extract statistical features from string values
pub fn extract_features(values: &[&str]) -> TypeFeatures {
  if values.is_empty() {
    return TypeFeatures::default();
  }

  let mut total_length = 0usize;
  let mut lengths = Vec::new();
  let mut char_counts: FxHashMap<char, usize> = FxHashMap::default();
  let mut digit_count = 0usize;
  let mut alpha_count = 0usize;
  let mut special_count = 0usize;
  let mut total_chars = 0usize;

  let mut has_protocol = false;
  let mut has_email_at = false;
  let mut has_url_dots = false;
  let mut has_file_extension = false;
  let mut is_all_uppercase = true;
  let mut has_hyphen_segments = false;

  for &s in values {
    let len = s.len();
    total_length += len;
    lengths.push(len);

    // Character analysis
    for ch in s.chars() {
      *char_counts.entry(ch).or_insert(0) += 1;
      total_chars += 1;

      if ch.is_ascii_digit() {
        digit_count += 1;
      } else if ch.is_alphabetic() {
        alpha_count += 1;
        if !ch.is_uppercase() {
          is_all_uppercase = false;
        }
      } else {
        special_count += 1;
      }
    }

    // Semantic markers
    if s.starts_with("http://") || s.starts_with("https://") {
      has_protocol = true;
    }
    if s.contains('@') {
      has_email_at = true;
    }
    if s.matches('.').count() >= 2 {
      has_url_dots = true;
    }
    if s.contains('.') && s.split('.').next_back().map(|ext| ext.len() <= 5).unwrap_or(false) {
      has_file_extension = true;
    }
    if s.matches('-').count() >= 3 {
      has_hyphen_segments = true;
    }
  }

  let avg_length = total_length as f64 / values.len() as f64;

  // Calculate length variance
  let variance = if lengths.len() > 1 {
    let sum_sq_diff: f64 = lengths.iter().map(|&l| (l as f64 - avg_length).powi(2)).sum();
    sum_sq_diff / lengths.len() as f64
  } else {
    0.0
  };

  // Shannon entropy of character distribution
  let char_entropy = if total_chars > 0 {
    let mut entropy = 0.0;
    for count in char_counts.values() {
      let probability = *count as f64 / total_chars as f64;
      if probability > 0.0 {
        entropy -= probability * probability.log2();
      }
    }
    entropy
  } else {
    0.0
  };

  // Character ratios
  let digit_ratio = if total_chars > 0 {
    digit_count as f64 / total_chars as f64
  } else {
    0.0
  };
  let alpha_ratio = if total_chars > 0 {
    alpha_count as f64 / total_chars as f64
  } else {
    0.0
  };
  let special_char_ratio = if total_chars > 0 {
    special_count as f64 / total_chars as f64
  } else {
    0.0
  };

  // Format consistency - check if lengths are similar
  let format_consistency = if variance == 0.0 {
    1.0 // All same length
  } else {
    let std_dev = variance.sqrt();
    let cv = std_dev / avg_length.max(1.0); // Coefficient of variation
    (1.0 - cv.min(1.0)).max(0.0)
  };

  let has_uuid_format = values.iter().all(|s| UUID_REGEX.is_match(s));

  TypeFeatures {
    char_entropy,
    digit_ratio,
    alpha_ratio,
    special_char_ratio,
    avg_length,
    length_variance: variance,
    min_length: *lengths.iter().min().unwrap_or(&0),
    max_length: *lengths.iter().max().unwrap_or(&0),
    format_consistency,
    has_protocol,
    has_uuid_format,
    has_email_at,
    has_url_dots,
    has_file_extension,
    is_all_uppercase,
    has_hyphen_segments,
  }
}

/// Check if values represent a categorical/enum type (low cardinality)
pub fn check_categorical(values: &[&str]) -> Option<(super::types::FieldType, f64)> {
  use rustc_hash::FxHashSet;

  if values.len() < 3 {
    // Need at least 3 samples to determine cardinality
    return None;
  }

  let unique: FxHashSet<&str> = values.iter().copied().collect();
  let unique_count = unique.len();

  // If we have many samples but only a few unique values, it's likely categorical
  // Cardinality ratio: unique / total
  let cardinality_ratio = unique_count as f64 / values.len() as f64;

  // Categorical if:
  // - Low unique count (2-8 values, tightened from 15)
  // - Low cardinality ratio (< 35% to be more strict)
  // - Not all unique values look like UUIDs/URLs/emails (to avoid conflicting with other types)
  // - Values are reasonably short (not long text blobs)
  let looks_like_other_type = unique.iter().any(|s| {
    UUID_REGEX.is_match(s) || super::constants::URL_REGEX.is_match(s) || EMAIL_REGEX.is_match(s) || s.len() > 100
  });

  // Check average length - categorical values are typically short
  let avg_length: f64 = unique.iter().map(|s| s.len()).sum::<usize>() as f64 / unique_count as f64;
  let is_reasonable_length = avg_length <= 50.0;

  // Anti-pattern: Check if values look sequential (1,2,3 or a,b,c should not be categorical)
  let looks_sequential = if unique_count >= 3 {
    let mut sorted: Vec<&str> = unique.iter().copied().collect();
    sorted.sort();

    // Check for numeric sequence
    let numeric_vals: Vec<i64> = sorted.iter().filter_map(|s| s.parse::<i64>().ok()).collect();
    if numeric_vals.len() == sorted.len() && numeric_vals.len() >= 3 {
      // Check if sequential (diff of 1 between consecutive values)
      numeric_vals.windows(2).all(|w| w[1] - w[0] == 1)
    } else {
      false
    }
  } else {
    false
  };

  // More conservative thresholds for better precision
  if (2..=8).contains(&unique_count)
    && cardinality_ratio < 0.35
    && !looks_like_other_type
    && is_reasonable_length
    && !looks_sequential
  {
    let values_vec: Vec<String> = unique.into_iter().map(String::from).collect();

    // Calculate confidence based on cardinality ratio (lower ratio = higher confidence)
    // Range: 0.75 (ratio=0.35) to 0.90 (ratio=0.1)
    let confidence = (0.75 + (0.35 - cardinality_ratio) * 0.4).clamp(0.75, 0.90);

    return Some((super::types::FieldType::Categorical { values: values_vec }, confidence));
  }

  None
}
