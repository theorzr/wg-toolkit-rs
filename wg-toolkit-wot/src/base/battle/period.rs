//! Where a battle is in its periods: waiting for its clients to be ready, counting down,
//! fighting, then over for a while before its clients are sent back, as its clients'
//! arenas are told, see `ARENA_PERIOD`.
//!
//! ```text
//!   Waiting ── all ready, or READY_WAIT ──▶ Countdown ── PREBATTLE_LENGTH ──▶ Battle
//!   Battle ── a team exterminated ──▶ Exterminated ── EXTERMINATION_DELAY ──▶ AfterBattle
//!                                       (that team's enemy wins, or a draw)
//!   Battle ── BATTLE_LENGTH ────────▶ AfterBattle (a draw, timed out)
//!   AfterBattle ── AFTER_BATTLE_LENGTH ──▶ done: its clients go back to their garage
//! ```

use std::time::{Duration, Instant};
use std::io;

use flate2::write::ZlibEncoder;
use flate2::Compression;

use wgtk::pickle::{self, Value as PickleValue};
use wgtk::app::entity::Bytes;


/// The team that wins a draw: none.
pub const NO_WINNER: u8 = 0;

/// The arena periods, `ARENA_PERIOD`.
const ARENA_PERIOD_PREBATTLE: u8 = 2;
const ARENA_PERIOD_BATTLE: u8 = 3;
const ARENA_PERIOD_AFTERBATTLE: u8 = 4;
/// `ARENA_UPDATE.PERIOD`.
const ARENA_UPDATE_PERIOD: u8 = 3;
/// Why a battle is over, `FINISH_REASON`: a team has no vehicle left alive, or its time
/// is up.
const FINISH_REASON_EXTERMINATION: u8 = 1;
const FINISH_REASON_TIMEOUT: u8 = 3;
/// The countdown before the battle, and the battle's length.
const PREBATTLE_LENGTH: Duration = Duration::from_secs(10);
const BATTLE_LENGTH: Duration = Duration::from_secs(900);
/// How long after a team is exterminated the battle is over, its clients told who won.
/// The user's choice.
const EXTERMINATION_DELAY: Duration = Duration::from_secs(5);
/// How long a battle stays over before its clients are sent back to their garage. Our
/// choice: enough to read who won.
const AFTER_BATTLE_LENGTH: Duration = Duration::from_secs(10);
/// How long a battle waits for its clients to be ready, done loading, before counting
/// down anyway. Our choice: a slow machine loading a map.
const READY_WAIT: Duration = Duration::from_secs(60);

/// See the module's documentation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArenaPeriod {
    /// Waiting for its clients to be ready, since when.
    Waiting(Instant),
    /// Counting down, until when.
    Countdown(Instant),
    /// Fighting, until when.
    Battle(Instant),
    /// Still fighting, a team exterminated: over at `at`, won by `winner`, or a draw
    /// ([`NO_WINNER`]).
    Exterminated {
        /// The team left alive, or [`NO_WINNER`].
        winner: u8,
        /// When the battle is over, [`EXTERMINATION_DELAY`] after.
        at: Instant,
    },
    /// Over: won by a team, or a draw ([`NO_WINNER`]), why, as `FINISH_REASON`, and
    /// until when its clients stay before they are sent back.
    AfterBattle {
        /// The team that won, or [`NO_WINNER`].
        winner: u8,
        /// Why it is over, as `FINISH_REASON`: a team exterminated, or its time up.
        reason: u8,
        /// When its clients are sent back to their garage.
        until: Instant,
    },
}

impl ArenaPeriod {

    /// Waiting from now.
    pub fn new() -> Self {
        Self::Waiting(Instant::now())
    }

    /// The period it goes to now, if due: the countdown once its clients are all
    /// `ready`, or after [`READY_WAIT`] whoever is; the battle once counted down; over
    /// [`EXTERMINATION_DELAY`] after a team is `exterminated` while fighting, its enemy
    /// (or nobody) the winner as given, or once its time is up, a draw.
    pub fn next(self, ready: bool, exterminated: Option<u8>) -> Option<Self> {
        let now = Instant::now();
        match (self, exterminated) {
            (Self::Waiting(since), _) if ready || now >= since + READY_WAIT => Some(Self::Countdown(now + PREBATTLE_LENGTH)),
            (Self::Countdown(until), _) if now >= until => Some(Self::Battle(now + BATTLE_LENGTH)),
            (Self::Battle(_), Some(winner)) => Some(Self::Exterminated {
                winner,
                at: now + EXTERMINATION_DELAY,
            }),
            (Self::Battle(until), None) if now >= until => Some(Self::after_battle(NO_WINNER, FINISH_REASON_TIMEOUT, now)),
            (Self::Exterminated { winner, at }, _) if now >= at => Some(Self::after_battle(winner, FINISH_REASON_EXTERMINATION, now)),
            _ => None,
        }
    }

    /// Over from `now`, won by this team for this reason.
    fn after_battle(winner: u8, reason: u8, now: Instant) -> Self {
        Self::AfterBattle {
            winner,
            reason,
            until: now + AFTER_BATTLE_LENGTH,
        }
    }

    /// Whether the battle is on: bots drive and fire, and players fire.
    pub fn fighting(self) -> bool {
        matches!(self, Self::Battle(_) | Self::Exterminated { .. })
    }

    /// Whether it has been over for long enough: its clients go back to their garage.
    pub fn done(self) -> bool {
        matches!(self, Self::AfterBattle { until, .. } if Instant::now() >= until)
    }

    /// Its `updateArena` of the `PERIOD` kind for a client of this game time, if any is
    /// told while waiting, nor once exterminated, still the battle for the clients:
    /// `(period, end time, length, extra)`, pickled then compressed, its end as long
    /// from now as is left of it. Its extra is nothing, but once over `(winner, reason)`.
    pub fn arena_update(self, game_time: f64) -> io::Result<Option<(u8, Bytes)>> {

        let (period, until, length, extra) = match self {
            Self::Waiting(_) | Self::Exterminated { .. } => return Ok(None),
            Self::Countdown(until) => (ARENA_PERIOD_PREBATTLE, until, PREBATTLE_LENGTH, PickleValue::List(Vec::new())),
            Self::Battle(until) => (ARENA_PERIOD_BATTLE, until, BATTLE_LENGTH, PickleValue::List(Vec::new())),
            Self::AfterBattle { winner, reason, until } => {
                let extra = PickleValue::Tuple(vec![
                    PickleValue::I64(winner.into()),
                    PickleValue::I64(reason.into()),
                ]);
                (ARENA_PERIOD_AFTERBATTLE, until, AFTER_BATTLE_LENGTH, extra)
            }
        };

        let end = game_time + until.saturating_duration_since(Instant::now()).as_secs_f64();
        let info = PickleValue::Tuple(vec![
            PickleValue::I64(period.into()),
            PickleValue::F64(end),
            PickleValue::I64(length.as_secs() as i64),
            extra,
        ]);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        pickle::to_writer(&mut encoder, &info)?;
        Ok(Some((ARENA_UPDATE_PERIOD, Bytes(encoder.finish()?))))

    }

}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn ends_on_extermination_or_timeout() {

        let ago = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        let later = Instant::now() + Duration::from_secs(60);

        // Fighting on, until a team is exterminated: still a while, then its enemy wins,
        // or nobody.
        assert_eq!(ArenaPeriod::Battle(later).next(true, None), None);
        let Some(exterminated @ ArenaPeriod::Exterminated { winner: 2, .. }) = ArenaPeriod::Battle(later).next(true, Some(2)) else {
            panic!("not exterminated");
        };
        assert!(exterminated.fighting());
        assert_eq!(exterminated.next(true, Some(2)), None);
        let due = ArenaPeriod::Exterminated {
            winner: 2,
            at: ago,
        };
        let Some(ArenaPeriod::AfterBattle { winner, reason, .. }) = due.next(true, Some(2)) else {
            panic!("not over");
        };
        assert_eq!((winner, reason), (2, FINISH_REASON_EXTERMINATION));

        // Out of time, a draw.
        let Some(ArenaPeriod::AfterBattle { winner, reason, .. }) = ArenaPeriod::Battle(ago).next(true, None) else {
            panic!("not over");
        };
        assert_eq!((winner, reason), (NO_WINNER, FINISH_REASON_TIMEOUT));

        // Over, it stays so, done once its time is up.
        let over = ArenaPeriod::after_battle(1, FINISH_REASON_EXTERMINATION, Instant::now());
        assert_eq!(over.next(true, Some(1)), None);
        assert!(!over.done() && !over.fighting());
        let done = ArenaPeriod::after_battle(1, FINISH_REASON_EXTERMINATION, ago.checked_sub(AFTER_BATTLE_LENGTH).unwrap());
        assert!(done.done());

    }

}
