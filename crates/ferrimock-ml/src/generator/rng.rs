//! The deterministic value source every generator draws from.
//!
//! Seeded rather than random, and seeded *per example* rather than per run: an
//! example's values are a function of its index alone. That is what makes a
//! corpus of millions streamable -- nothing has to be held in memory to be
//! reproduced, and two machines asked for row 3_141_592 produce the same row.

/// A small deterministic generator.
///
/// xorshift64* for the stream, splitmix64 for the seed. The second half matters
/// more than it looks: indices handed in are adjacent integers, and seeding
/// xorshift directly with those produces streams that stay correlated for
/// several draws -- neighbouring examples would share their first choices.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Start a stream from a seed, decorrelating adjacent seeds first.
    pub fn seeded(seed: u64) -> Self {
        let mut state = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        state = (state ^ (state >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        state = (state ^ (state >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self((state ^ (state >> 31)) | 1)
    }

    /// A stream for one example, derived from the corpus seed and the row index.
    pub fn for_index(seed: u64, index: u64) -> Self {
        Self::seeded(seed ^ index.wrapping_mul(0xD6E8_FEB8_6659_FD93))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `[0, bound)`, or 0 when the bound is empty.
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            #[allow(clippy::cast_possible_truncation)] // modulo keeps this in range
            let drawn = (self.next_u64() % bound as u64) as usize;
            drawn
        }
    }

    /// A value in `[low, high]`, inclusive at both ends.
    pub fn between(&mut self, low: usize, high: usize) -> usize {
        if high <= low {
            return low;
        }
        low + self.below(high - low + 1)
    }

    /// True with probability `numerator / denominator`.
    pub fn chance(&mut self, numerator: u32, denominator: u32) -> bool {
        if denominator == 0 {
            return false;
        }
        (self.next_u64() % u64::from(denominator)) < u64::from(numerator)
    }

    /// One of the options, or `None` for an empty table -- which is a bug in the
    /// table rather than in the draw, and is left visible instead of defaulted.
    pub fn choose<'a, T>(&mut self, options: &'a [T]) -> Option<&'a T> {
        options.get(self.below(options.len()))
    }

    /// One of the options as text. Empty tables answer with the empty string so
    /// a value synthesiser can keep composing without an unwrap.
    pub fn pick<'a>(&mut self, options: &'a [&'a str]) -> &'a str {
        self.choose(options).copied().unwrap_or("")
    }

    /// An index into `weights`, drawn in proportion to them.
    ///
    /// This is how a value mode is chosen: a label's common spelling should turn
    /// up far more often than its rare one, or a corpus teaches the model that
    /// every shape of a timestamp is equally likely.
    pub fn weighted(&mut self, weights: &[u32]) -> usize {
        let total: u32 = weights.iter().sum();
        if total == 0 {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation)] // bounded by `total`, a u32
        let mut point = (self.next_u64() % u64::from(total)) as u32;
        for (index, weight) in weights.iter().enumerate() {
            if point < *weight {
                return index;
            }
            point -= *weight;
        }
        weights.len() - 1
    }

    /// Draw from an alphabet.
    pub fn from_alphabet(&mut self, alphabet: &[u8], length: usize) -> String {
        (0..length)
            .map(|_| {
                let index = self.below(alphabet.len());
                char::from(alphabet.get(index).copied().unwrap_or(b'0'))
            })
            .collect()
    }

    pub fn hex(&mut self, length: usize) -> String {
        self.from_alphabet(b"0123456789abcdef", length)
    }

    pub fn hex_upper(&mut self, length: usize) -> String {
        self.from_alphabet(b"0123456789ABCDEF", length)
    }

    pub fn digits(&mut self, length: usize) -> String {
        self.from_alphabet(b"0123456789", length)
    }

    /// Digits that never start with zero, for anything read back as a number.
    pub fn digits_no_leading_zero(&mut self, length: usize) -> String {
        if length == 0 {
            return String::new();
        }
        let first = self.from_alphabet(b"123456789", 1);
        format!("{first}{}", self.digits(length - 1))
    }

    pub fn alnum(&mut self, length: usize) -> String {
        self.from_alphabet(
            b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            length,
        )
    }

    pub fn lower_alnum(&mut self, length: usize) -> String {
        self.from_alphabet(b"abcdefghijklmnopqrstuvwxyz0123456789", length)
    }

    /// Base62, the alphabet behind most prefixed opaque ids.
    pub fn base62(&mut self, length: usize) -> String {
        self.from_alphabet(
            b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
            length,
        )
    }

    pub fn base36(&mut self, length: usize) -> String {
        self.from_alphabet(b"0123456789abcdefghijklmnopqrstuvwxyz", length)
    }

    /// Crockford base32: ULID's alphabet, which drops I, L, O and U.
    pub fn crockford(&mut self, length: usize) -> String {
        self.from_alphabet(b"0123456789ABCDEFGHJKMNPQRSTVWXYZ", length)
    }

    /// The URL-safe base64 alphabet, without padding.
    pub fn url_safe(&mut self, length: usize) -> String {
        self.from_alphabet(
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_",
            length,
        )
    }

    /// Standard base64 text, padded to a multiple of four.
    ///
    /// A body length of `4n + 1` is grown to `4n + 2` rather than padded with
    /// three `=`: base64 encodes three bytes to four characters, so one leftover
    /// character cannot occur and three padding characters never appear in real
    /// output. Emitting them taught a model that base64 has a shape it does not.
    pub fn base64(&mut self, length: usize) -> String {
        let usable = if length % 4 == 1 { length + 1 } else { length };
        let body = self.from_alphabet(
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
            usable,
        );
        match usable % 4 {
            0 => body,
            2 => format!("{body}=="),
            _ => format!("{body}="),
        }
    }

    /// Shuffle in place, so a seed reproduces an ordering exactly.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            let swap = self.below(index + 1);
            items.swap(index, swap);
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_reproduces_its_stream() {
        let mut first = Rng::seeded(42);
        let mut second = Rng::seeded(42);
        for _ in 0..64 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
    }

    #[test]
    fn adjacent_indices_do_not_share_their_first_draws() {
        // Seeding xorshift with the raw index is the mistake this guards: rows
        // 1000 and 1001 would then open with the same choices, and a streamed
        // corpus would be full of near-duplicates.
        let opening = |index: u64| -> Vec<usize> {
            let mut rng = Rng::for_index(7, index);
            (0..6).map(|_| rng.below(16)).collect()
        };
        assert_ne!(opening(1000), opening(1001));
        assert_ne!(opening(1000), opening(1002));
        assert_eq!(opening(1000), opening(1000));
    }

    #[test]
    fn a_row_is_a_function_of_its_index_alone() {
        let mut direct = Rng::for_index(9, 3_141_592);
        let mut again = Rng::for_index(9, 3_141_592);
        assert_eq!(direct.alnum(24), again.alnum(24));
    }

    #[test]
    fn bounds_are_respected() {
        let mut rng = Rng::seeded(1);
        for _ in 0..500 {
            assert!(rng.below(7) < 7);
            let value = rng.between(3, 9);
            assert!((3..=9).contains(&value));
        }
        assert_eq!(rng.below(0), 0, "an empty range has one answer");
        assert_eq!(rng.between(5, 5), 5);
        assert_eq!(
            rng.between(9, 2),
            9,
            "an inverted range collapses to its low"
        );
    }

    #[test]
    fn a_weighted_draw_follows_its_weights() {
        let mut rng = Rng::seeded(11);
        let mut counts = [0usize; 3];
        for _ in 0..3_000 {
            counts[rng.weighted(&[90, 9, 1])] += 1;
        }
        assert!(counts[0] > counts[1], "{counts:?}");
        assert!(counts[1] > counts[2], "{counts:?}");
    }

    #[test]
    fn a_zero_weight_is_never_drawn() {
        let mut rng = Rng::seeded(5);
        for _ in 0..200 {
            assert_ne!(rng.weighted(&[1, 0, 1]), 1);
        }
    }

    #[test]
    fn alphabets_produce_the_length_they_were_asked_for() {
        let mut rng = Rng::seeded(3);
        assert_eq!(rng.hex(32).len(), 32);
        assert_eq!(rng.crockford(26).len(), 26);
        assert_eq!(rng.base62(21).len(), 21);
        assert_eq!(rng.url_safe(21).len(), 21);
        assert_eq!(rng.digits_no_leading_zero(12).len(), 12);
    }

    #[test]
    fn a_number_never_opens_with_a_zero() {
        let mut rng = Rng::seeded(17);
        for _ in 0..200 {
            assert!(!rng.digits_no_leading_zero(11).starts_with('0'));
        }
    }

    #[test]
    fn base64_is_padded_the_way_base64_really_is() {
        let mut rng = Rng::seeded(23);
        for length in 1..40 {
            let encoded = rng.base64(length);
            assert_eq!(encoded.len() % 4, 0, "{encoded} is not padded");
            let padding = encoded.chars().filter(|c| *c == '=').count();
            assert!(
                padding <= 2,
                "{encoded} carries {padding} padding characters, which base64 never does"
            );
        }
    }

    #[test]
    fn crockford_omits_the_letters_it_is_supposed_to() {
        let mut rng = Rng::seeded(29);
        let drawn = rng.crockford(2_000);
        for excluded in ['I', 'L', 'O', 'U'] {
            assert!(
                !drawn.contains(excluded),
                "ULID's alphabet has no {excluded}"
            );
        }
    }
}
