//! How a value is spread over its support.
//!
//! Every draw here is a pure map from the bytes a field already derived, so
//! nothing about laziness, replay or determinism changes — only what comes
//! out. That is the whole point: the engine's loudest remaining signature is
//! statistical rather than structural, and "uniform, everywhere, for
//! everything" is not a shape any real API has.

/// How a magnitude is spread over its support.
#[derive(Debug, Clone, PartialEq)]
pub enum Spread {
    /// Every value equally likely. Correct where a field's declared range is
    /// narrow — a rating, a percentage, a page size — and wrong everywhere a
    /// real value spans orders of magnitude.
    Uniform { low: f64, high: f64 },
    /// Equal mass per decade.
    ///
    /// This is what makes a leading-digit profile read like a real one.
    /// Log-*normal* is the more commonly reached-for answer and is worse here:
    /// truncated to a range narrower than a decade it is uniform again, and a
    /// support that includes zero breaks the logarithm outright.
    LogUniform { low: f64, high: f64 },
    /// A multiplicative bell: most values near the median, a long right tail.
    LogNormal { median: f64, sigma: f64 },
    /// A point mass at zero laid over another spread. Counts are mostly zero,
    /// and a count that never is announces itself.
    ZeroInflated { zero: f64, inner: Box<Spread> },
    /// Waiting times: mostly short, occasionally long. What a collection
    /// length actually looks like.
    Geometric { mean: f64 },
}

impl Spread {
    /// One draw, from one derived word.
    #[must_use]
    pub fn draw(&self, derived: u64) -> f64 {
        match self {
            Self::Uniform { low, high } => between(*low, *high, unit(derived)),
            Self::LogUniform { low, high } => {
                let (low, high) = positive_span(*low, *high);
                between(low.ln(), high.ln(), unit(derived)).exp()
            }
            Self::LogNormal { median, sigma } => {
                let median = if *median > 0.0 { *median } else { 1.0 };
                (sigma.mul_add(normal(derived), median.ln())).exp()
            }
            // The inflation reads one end of the word and the inner spread the
            // other, so a value that missed the point mass is not also biased
            // toward the bottom of the range it fell back to.
            Self::ZeroInflated { zero, inner } => {
                if unit(derived.rotate_left(29)) < *zero {
                    0.0
                } else {
                    inner.draw(derived)
                }
            }
            Self::Geometric { mean } => {
                let mean = mean.max(f64::EPSILON);
                let chance = 1.0 / (1.0 + mean);
                let drawn = unit(derived).max(f64::MIN_POSITIVE);
                drawn.log((1.0 - chance).max(f64::MIN_POSITIVE)).floor()
            }
        }
    }

    /// The same draw, rounded to a whole number inside `[low, high]`.
    #[must_use]
    pub fn whole(&self, derived: u64, low: i64, high: i64) -> i64 {
        let drawn = self.draw(derived);
        if !drawn.is_finite() {
            return low;
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "clamped to the declared bounds either side of the cast"
        )]
        let rounded = drawn.round().clamp(-9.2e18, 9.2e18) as i64;
        rounded.clamp(low.min(high), high.max(low))
    }
}

/// How the members of a closed set share the mass.
///
/// Skewed, but with nothing asserted about *which* member is common. Keying
/// the prior on declaration order is the tempting shortcut and it is wrong:
/// lifecycle enums are ordered by lifecycle, so the terminal state carrying
/// most of the mass is listed last; machine-emitted schemas are frequently
/// alphabetical; and protobuf mandates the zero value first, conventionally
/// `UNSPECIFIED`, which should be near-absent in real data. So the ranks are
/// permuted by a word drawn from the field itself. Which member is modal is
/// something only a recording can say, and that is where it should come from.
#[derive(Debug, Clone, Copy)]
pub struct Ranking {
    members: usize,
    exponent: f64,
    order: u64,
}

/// How skewed a set of members is, drawn per field so two enums of a schema
/// do not share one shape.
const LEAST_SKEW: f64 = 0.7;
const MOST_SKEW: f64 = 1.6;

impl Ranking {
    /// A ranking over `members`, shaped and permuted by one derived word.
    #[must_use]
    pub fn of(members: usize, derived: u64) -> Self {
        Self {
            members,
            exponent: between(LEAST_SKEW, MOST_SKEW, unit(derived)),
            order: derived.rotate_left(17),
        }
    }

    /// Which member one draw lands on.
    #[must_use]
    pub fn pick(&self, derived: u64) -> usize {
        if self.members <= 1 {
            return 0;
        }
        let weight = |rank: usize| {
            #[allow(clippy::cast_precision_loss, reason = "a rank inside a schema's enum")]
            let position = (rank + 1) as f64;
            position.powf(-self.exponent)
        };
        let total: f64 = (0..self.members).map(weight).sum();
        let target = unit(derived) * total;
        let mut carried = 0.0;
        for rank in 0..self.members {
            carried += weight(rank);
            if carried >= target {
                return permuted(rank, self.members, self.order);
            }
        }
        permuted(self.members - 1, self.members, self.order)
    }
}

/// How often a flag is set.
///
/// Never a fair coin. A real boolean is lopsided in one direction or the
/// other — half of an API's users are not administrators — and which
/// direction is a fact about the field, so the chance is drawn from the
/// field's own word and pushed away from the middle.
#[must_use]
pub fn lopsided_chance(derived: u64) -> f64 {
    const NEAREST: f64 = 0.02;
    const FURTHEST: f64 = 0.42;

    let drawn = unit(derived);
    let toward_zero = drawn < 0.5;
    let distance = if toward_zero { drawn } else { 1.0 - drawn } * 2.0;
    let magnitude = (FURTHEST - NEAREST).mul_add(distance * distance, NEAREST);
    if toward_zero {
        magnitude
    } else {
        1.0 - magnitude
    }
}

/// Whether one draw lands inside `chance`.
#[must_use]
pub fn falls_within(chance: f64, derived: u64) -> bool {
    unit(derived) < chance
}

/// A derived word as a uniform draw on `[0, 1)`.
#[must_use]
pub fn unit(derived: u64) -> f64 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "53 bits, which is exactly the f64 mantissa"
    )]
    let scaled = (derived >> 11) as f64 / (1_u64 << 53) as f64;
    scaled
}

fn between(low: f64, high: f64, at: f64) -> f64 {
    if high <= low {
        low
    } else {
        (high - low).mul_add(at, low)
    }
}

/// A support a logarithm can be taken over.
fn positive_span(low: f64, high: f64) -> (f64, f64) {
    let low = if low > 0.0 { low } else { 1.0 };
    let high = if high > low { high } else { low * 10.0 };
    (low, high)
}

/// A standard normal from one word, by Box-Muller on its two halves.
fn normal(derived: u64) -> f64 {
    let first = unit(derived).max(f64::MIN_POSITIVE);
    let second = unit(derived.rotate_left(32));
    (-2.0 * first.ln()).sqrt() * (std::f64::consts::TAU * second).cos()
}

/// A permutation of `0..members`, from one derived word, without allocating.
///
/// Multiplying by a step coprime to the size and adding an offset is a
/// bijection. The domain here is a schema's enum — a handful of members — so a
/// full shuffle would be an allocation per field per record for no more
/// mixing than this.
fn permuted(index: usize, members: usize, derived: u64) -> usize {
    if members <= 1 {
        return 0;
    }
    let offset = usize::try_from(derived % members as u64).unwrap_or(0);
    let start = usize::try_from(derived >> 32).unwrap_or(0) % members;
    let mut step = start + 1;
    for _ in 0..members {
        if gcd(step, members) == 1 {
            break;
        }
        step = step % members + 1;
    }
    index.wrapping_mul(step).wrapping_add(offset) % members
}

const fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let held = b;
        b = a % b;
        a = held;
    }
    a
}

#[cfg(test)]
mod tests;
