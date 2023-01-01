//!
//! The ID is a 64-bit integer, composed of:
//! - 32 bits for the seconds since 2023-01-01 00:00:00 UTC
//! - 10 bits for a counter
//! - 22 bits for a random seed
//!
//! It is far from ideal. But it does not require any communication between peers to generate a new ID with low collision probability.
//!

use std::{
    num::NonZeroU64,
    time::{Duration, SystemTime},
};

use argosy_id::AssetId;
use parking_lot::Mutex;

const STARTING_EPOCH: u64 = 1672520400;
const MAX_COUNTER: u16 = 1 << 10;

struct State {
    last_st: SystemTime, // to prevent time to go backwards
    counter: u16,
    seed: u32,
}

pub struct IdGen {
    state: Mutex<State>,
}

impl IdGen {
    pub fn new() -> Self {
        IdGen {
            state: Mutex::new(State {
                last_st: SystemTime::UNIX_EPOCH + Duration::from_secs(STARTING_EPOCH),
                counter: 1,
                seed: rand::random(),
            }),
        }
    }

    pub fn new_id(&self) -> AssetId {
        let st = SystemTime::now();
        let mut state = self.state.lock();
        let st = st.max(state.last_st);

        loop {
            if st == state.last_st {
                debug_assert!(state.counter <= MAX_COUNTER);
                if state.counter >= MAX_COUNTER {
                    // counter overflow
                    // this even is highly unlikely to happen
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
                state.counter += 1;
            } else {
                state.last_st = st;
                state.counter = 1;
            }

            let seconds =
                st.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() - STARTING_EPOCH;

            let id = (state.seed as u64) << 42 | (state.counter as u64) << 32 | seconds;

            debug_assert_ne!(id, 0);
            return AssetId(NonZeroU64::new(id).unwrap());
        }
    }
}
