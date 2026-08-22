//! When a record came into being.
//!
//! A world with no history is a world where creation times are flat, show no
//! weekly or daily structure, and sit inside whatever window a constant
//! happened to name. This is the history.
//!
//! The one hard constraint is that the answer must not depend on how many
//! instances exist. Placing arrival *i* among *N* — the obvious way to put N
//! arrivals in a fixed window — makes a creation time a function of the count,
//! so bumping `world.counts` or mounting a second schema with a different
//! `seed_count` silently rewrites the creation time of every record that
//! already existed. Nothing reports that: a delta conflict is only raised when
//! an ordinal *disappears*.

use crate::fake_data::distribution::unit;
use crate::fake_data::{datetime, rng};

/// How far back the first instance of an entity sits.
///
/// Per entity, so a collection that has been filling for years and one that
/// started last spring do not share a history, and drawn from the seed alone
/// so neither depends on how big either is.
const SHORTEST_HISTORY_DAYS: f64 = 200.0;
const LONGEST_HISTORY_DAYS: f64 = 5.0 * 365.0;

const SECONDS_PER_DAY: f64 = 86_400.0;
const HOURS_PER_WEEK: usize = 168;
const SECONDS_PER_WEEK: i64 = 604_800;

/// The Unix epoch fell on a Thursday, three days after the Monday its week
/// began.
const EPOCH_INTO_WEEK: i64 = 3 * 86_400;

/// The instant the world is read from.
#[must_use]
pub fn now() -> i64 {
    datetime::anchor().timestamp()
}

/// When the `ordinal`th instance of an entity came into being.
///
/// Ordinal zero is the oldest, so a record's ordinal, its key and its age all
/// rise together — which is what lets a sequential id agree with a creation
/// time. Both ends of the window are anchored without consulting the count:
/// the first arrival sits at the start of the entity's history and the rest
/// close on the present, so the newest is recent whether the world holds
/// twelve records or six hundred.
#[must_use]
pub fn moment_of(seed: u64, entity: &str, ordinal: u64) -> i64 {
    let stream = format!("{entity}#arrival");
    let drawn = rng::derive_seed(seed, &stream, ordinal);
    // A whole step plus a fraction of one keeps the sequence ordered at the
    // scale of a day while leaving the gaps themselves irregular.
    #[allow(
        clippy::cast_precision_loss,
        reason = "an ordinal inside a census, far below the f64 mantissa"
    )]
    let step = ordinal as f64 + unit(drawn);
    // Both ends anchored without consulting the count: the first arrival at
    // the start of the history and the rest closing on the present. The
    // consequence is worth stating rather than discovering — arrivals are
    // heavily recency-weighted, the way a service whose volume grew is, rather
    // than spread evenly across its own history.
    let age_days = history_of(seed, entity) / (1.0 + step);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "an age in seconds, bounded by the constants above"
    )]
    let seconds = (age_days * SECONDS_PER_DAY).min(9e18) as i64;
    seasonal_warp(now().saturating_sub(seconds))
}

/// How many days of history this entity has.
fn history_of(seed: u64, entity: &str) -> f64 {
    let drawn = rng::derive_seed(seed, &format!("{entity}#history"), 0);
    (LONGEST_HISTORY_DAYS - SHORTEST_HISTORY_DAYS).mul_add(unit(drawn), SHORTEST_HISTORY_DAYS)
}

/// The same week, at an hour someone was working.
///
/// A flat instant has no daily or weekly structure, and both are visible in a
/// histogram of any real collection. The warp is *monotone* — an instant's
/// position within its week is pushed through a rising cumulative intensity,
/// and the week it belongs to never changes — which is the whole reason it can
/// be applied at all: an id that carries a creation time has to keep agreeing
/// with the time beside it, and any reshuffle inside a day breaks that as soon
/// as the arrivals are closer together than a day.
fn seasonal_warp(moment: i64) -> i64 {
    let into_week = (moment + EPOCH_INTO_WEEK).rem_euclid(SECONDS_PER_WEEK);
    let opened = moment - into_week;
    #[allow(
        clippy::cast_precision_loss,
        reason = "a position inside a week, far below the f64 mantissa"
    )]
    let position = into_week as f64 / SECONDS_PER_WEEK as f64;
    opened + warped_hour(position)
}

/// Where a uniform position in the week lands once the week's own intensity is
/// taken into account.
fn warped_hour(position: f64) -> i64 {
    let carried = intensity();
    let total = carried.last().copied().unwrap_or(1.0);
    let target = position.clamp(0.0, 1.0) * total;
    let at = carried.partition_point(|edge| *edge <= target);
    let Some(hour) = at.checked_sub(1).filter(|hour| *hour < HOURS_PER_WEEK) else {
        return 0;
    };
    let (Some(start), Some(end)) = (carried.get(hour), carried.get(hour + 1)) else {
        return 0;
    };
    let within = if end > start {
        (target - start) / (end - start)
    } else {
        0.0
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "an hour of the week, bounded by the array it indexes"
    )]
    let seconds = ((hour as f64 + within) * 3600.0) as i64;
    seconds.clamp(0, SECONDS_PER_WEEK - 1)
}

/// How much of a week's work happens in each of its hours, cumulatively.
///
/// Two humps on a working day, a lull over lunch, almost nothing overnight,
/// and a weekend that is not empty but is close to it.
fn intensity() -> &'static [f64; HOURS_PER_WEEK + 1] {
    static CARRIED: std::sync::OnceLock<[f64; HOURS_PER_WEEK + 1]> = std::sync::OnceLock::new();
    CARRIED.get_or_init(|| {
        const BY_HOUR: [f64; 24] = [
            0.10, 0.06, 0.05, 0.05, 0.07, 0.15, 0.40, 0.90, 1.60, 2.20, 2.40, 2.20, 1.40, 1.90,
            2.30, 2.20, 1.80, 1.30, 0.90, 0.60, 0.40, 0.30, 0.20, 0.14,
        ];
        const WEEKEND: f64 = 0.12;

        let mut carried = [0.0_f64; HOURS_PER_WEEK + 1];
        let mut running = 0.0;
        for hour in 0..HOURS_PER_WEEK {
            let weekend = hour / 24 >= 5;
            let weight = BY_HOUR.get(hour % 24).copied().unwrap_or(1.0)
                * if weekend { WEEKEND } else { 1.0 };
            running += weight;
            if let Some(slot) = carried.get_mut(hour + 1) {
                *slot = running;
            }
        }
        carried
    })
}

/// A moment a field carries, given the record's own and where the field sits
/// in the record's life.
///
/// A record's timestamps are not one instant: it was created, then touched,
/// then closed. The field that names the *opening* is the arrival itself —
/// anything else and an id built from the arrival stops agreeing with the
/// `created_at` beside it. The later ones wait, and `order_lifecycle` deals
/// the results back out in the order the names imply.
#[must_use]
pub fn field_moment(arrived: i64, derived: u64, stage: u8) -> i64 {
    const LONGEST_WAIT_DAYS: f64 = 120.0;

    if stage == 0 {
        return arrived.min(now());
    }
    let reach = LONGEST_WAIT_DAYS * f64::from(stage) / 2.0;
    let waited = unit(derived).powi(3) * reach * SECONDS_PER_DAY;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a wait in seconds, bounded by the constant above"
    )]
    let seconds = waited as i64;
    arrived.saturating_add(seconds).min(now())
}

#[cfg(test)]
mod tests;
