//! Utility functions for the ferrimock framework

/// Calculate Levenshtein distance between two strings
/// Used for fuzzy string matching and typo detection
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    // Use a single flat vector instead of Vec<Vec<>> to avoid indexing lint.
    // Layout: row-major, (a_len + 1) rows x (b_len + 1) columns.
    let cols = b_len + 1;
    let mut matrix = vec![0_usize; (a_len + 1) * cols];

    // Helper closures for safe access via .get()/.get_mut()
    let idx = |r: usize, c: usize| -> usize { r * cols + c };

    for i in 0..=a_len {
        if let Some(cell) = matrix.get_mut(idx(i, 0)) {
            *cell = i;
        }
    }
    for j in 0..=b_len {
        if let Some(cell) = matrix.get_mut(idx(0, j)) {
            *cell = j;
        }
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    for (i, a_char) in a_chars.iter().enumerate() {
        for (j, b_char) in b_chars.iter().enumerate() {
            let cost = usize::from(a_char != b_char);

            let delete = matrix.get(idx(i, j + 1)).unwrap_or(&0) + 1;
            let insert = matrix.get(idx(i + 1, j)).unwrap_or(&0) + 1;
            let substitute = matrix.get(idx(i, j)).unwrap_or(&0) + cost;

            let min_val = std::cmp::min(std::cmp::min(delete, insert), substitute);

            if let Some(cell) = matrix.get_mut(idx(i + 1, j + 1)) {
                *cell = min_val;
            }
        }
    }

    matrix.get(idx(a_len, b_len)).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("abc", "abx"), 1);
    }
}
