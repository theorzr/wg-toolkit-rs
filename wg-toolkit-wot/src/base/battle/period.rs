//! Where a battle is in its periods: waiting for its clients to be ready, counting down,
//! then fighting, as its clients' arenas are told, see `ARENA_PERIOD`.

use std::time::{Duration, Instant};
use std::io;

use flate2::write::ZlibEncoder;
use flate2::Compression;

use wgtk::pickle::{self, Value as PickleValue};
use wgtk::app::entity::Bytes;


/// See the module's documentation.
#[derive(Debug, Clone, Copy)]
pub enum ArenaPeriod {
    /// Waiting for its clients to be ready, since when.
    Waiting(Instant),
    /// Counting down, until when.
    Countdown(Instant),
    /// Fighting, until when.
    Battle(Instant),
}

impl ArenaPeriod {

    /// Waiting from now.
    pub fn new() -> Self {
        Self::Waiting(Instant::now())
    }

    /// The period it goes to now, if due: the countdown once its clients are all
    /// `ready`, or after [`READY_WAIT`] whoever is; the battle once counted down.
    pub fn next(self, ready: bool) -> Option<Self> {
        let now = Instant::now();
        match self {
            Self::Waiting(since) if ready || now >= since + READY_WAIT => Some(Self::Countdown(now + PREBATTLE_LENGTH)),
            Self::Countdown(until) if now >= until => Some(Self::Battle(now + BATTLE_LENGTH)),
            _ => None,
        }
    }

    /// Whether the battle is on: bots drive and fire.
    pub fn fighting(self) -> bool {
        matches!(self, Self::Battle(_))
    }

    /// Its `updateArena` of the `PERIOD` kind for a client of this game time, if any is
    /// told while waiting: `(period, end time, length, extra)`, pickled then
    /// compressed, its end as long from now as is left of it.
    pub fn arena_update(self, game_time: f64) -> io::Result<Option<(u8, Bytes)>> {
        let (period, until, length) = match self {
            Self::Waiting(_) => return Ok(None),
            Self::Countdown(until) => (ARENA_PERIOD_PREBATTLE, until, PREBATTLE_LENGTH),
            Self::Battle(until) => (ARENA_PERIOD_BATTLE, until, BATTLE_LENGTH),
        };
        let end = game_time + until.saturating_duration_since(Instant::now()).as_secs_f64();
        let info = PickleValue::Tuple(vec![
            PickleValue::I64(period.into()),
            PickleValue::F64(end),
            PickleValue::I64(length.as_secs() as i64),
            PickleValue::List(Vec::new()),
        ]);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        pickle::to_writer(&mut encoder, &info)?;
        Ok(Some((ARENA_UPDATE_PERIOD, Bytes(encoder.finish()?))))
    }

}


/// The arena periods, `ARENA_PERIOD`.
const ARENA_PERIOD_PREBATTLE: u8 = 2;
const ARENA_PERIOD_BATTLE: u8 = 3;
/// `ARENA_UPDATE.PERIOD`.
const ARENA_UPDATE_PERIOD: u8 = 3;
/// The countdown before the battle, and the battle's length.
const PREBATTLE_LENGTH: Duration = Duration::from_secs(10);
const BATTLE_LENGTH: Duration = Duration::from_secs(900);
/// How long a battle waits for its clients to be ready, done loading, before counting
/// down anyway. Our choice: a slow machine loading a map.
const READY_WAIT: Duration = Duration::from_secs(60);
