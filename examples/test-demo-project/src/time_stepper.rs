use std::time::Duration;

use time::{OffsetDateTime, SignedDuration};

/// 60 TPS (ish)
pub const GAME_TIME_STEP: Duration = Duration::from_micros(16_667);
#[expect(unused)]
pub const GAME_TIME_DELTA: f32 = GAME_TIME_STEP.as_secs_f32();
// max time accumulated between frames should be 0.25 seconds
pub const MAX_TIME_BETWEEN_STEPS: SignedDuration = SignedDuration::milliseconds(250);

pub struct FixedTimeStepper {
    last_checked_time: OffsetDateTime,
    game_time_step: Duration,
    // Wall-clock time observed since the last tick was consumed.
    accumulator: SignedDuration,
    last_frame_time: SignedDuration,
    // has to be bigger than game_time_step
    max_time_between_steps: SignedDuration,
}

impl FixedTimeStepper {
    pub fn new(game_time_step: Duration, max_time_between_steps: SignedDuration) -> Self {
        Self {
            last_checked_time: OffsetDateTime::now_utc(),
            game_time_step,
            accumulator: SignedDuration::ZERO,
            last_frame_time: SignedDuration::ZERO,
            max_time_between_steps,
        }
    }

    /// Returns true once per pending tick, so callers drain it with `while timestepper.step() { .. }`.
    pub fn step(&mut self) -> bool {
        // This might result in a runaway effect if the amount of time we're getting isn't enough to drain
        // which is why we have max_time_between_steps
        let current_time = OffsetDateTime::now_utc();
        let frame_time = current_time - self.last_checked_time;
        self.last_checked_time = current_time;

        self.accumulator += if frame_time > self.max_time_between_steps {
            self.max_time_between_steps
        } else {
            frame_time
        };

        self.last_frame_time = frame_time;

        if self.accumulator >= self.game_time_step {
            self.accumulator -= self.game_time_step;
            true
        } else {
            false
        }
    }

    #[expect(unused)]
    pub fn get_time_since_last_step(&self) -> SignedDuration {
        self.last_frame_time
    }
}

impl Default for FixedTimeStepper {
    fn default() -> Self {
        Self::new(GAME_TIME_STEP, MAX_TIME_BETWEEN_STEPS)
    }
}
