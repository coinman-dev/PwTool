//! Entropy accounting (`upstream/src/random/EntropyManager.*`).
//!
//! Keeps the estimate of entropy bits in the pool that is shown in the main
//! window, and feeds user input events and user-provided data into the pool.
//!
//! Deliberate fix: in the original, `AddEvent()` writes
//! `auto randPool = RandomPool::GetInstance();`, which *copies* the pool, so
//! the event data ends up in a temporary copy and never reaches the real pool
//! (only the side effect of the copy constructor — a flush of the real pool —
//! remains). Here the data is added to the real pool, as intended.

use std::sync::{LazyLock, Mutex, MutexGuard};

use super::hrtimer::{TimerKind, high_res_timer, timer_kind};
use super::pool::{MAX_ENTROPY, RandomPool};

/// Kind of a user input event (`EntropyEventType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntropyEventType {
    Keyboard,
    MouseClick,
    MouseMove,
    MouseWheel,
    Other,
}

pub const MAX_ENTROPYBITS: u32 = 10000;
pub const MAX_TOTALENTBITS: u32 = 1_000_000_000;
const TIMER_ENTROPYBITS_DERATING: u32 = 4;
/// 100 ns => 1 ms (`HRT_NONE_ACCURACY_DERATING`).
const HRT_NONE_ACCURACY_DERATING: u32 = 10000;

pub struct EntropyManager {
    entropy_bits: u32,
    total_ent_bits: u32,
    max_timer_ent_bits: u32,
    system_ent_bits: u32,
    last_keys: [u64; 2],
    last_pos: [u64; 2],
    last_deltas: [u32; 2],
    last_timer: u64,
}

static GLOBAL: LazyLock<Mutex<EntropyManager>> = LazyLock::new(|| Mutex::new(EntropyManager::new(4, 16)));

fn log_base2(v: u32) -> u32 {
    if v == 0 { 0 } else { 31 - v.leading_zeros() }
}

impl EntropyManager {
    pub fn new(max_timer_ent_bits: u32, system_ent_bits: u32) -> Self {
        EntropyManager {
            entropy_bits: 0,
            total_ent_bits: 0,
            max_timer_ent_bits,
            system_ent_bits,
            last_keys: [0; 2],
            last_pos: [0; 2],
            last_deltas: [0; 2],
            last_timer: 0,
        }
    }

    /// The program-wide instance (`EntropyManager::GetInstance()`).
    pub fn global() -> MutexGuard<'static, EntropyManager> {
        GLOBAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn increase(&mut self, bits: u32) {
        self.entropy_bits = (self.entropy_bits.saturating_add(bits)).min(MAX_ENTROPYBITS);
        self.total_ent_bits = (self.total_ent_bits.saturating_add(bits)).min(MAX_TOTALENTBITS);
    }

    /// Add a user input event (`AddEvent()`). `key` is the key code for
    /// keyboard events, `pos` the packed pointer position for mouse clicks;
    /// `data` is any further raw event data. Returns the entropy bits credited.
    pub fn add_event(
        &mut self,
        pool: &mut RandomPool,
        event: EntropyEventType,
        key: u64,
        pos: u64,
        data: &[u8],
        mut ent_bits: u32,
    ) -> u32 {
        let timer = high_res_timer();
        pool.add_data(&timer.to_le_bytes());
        let mut delta = timer.wrapping_sub(self.last_timer).min(0xffff_ffff) as u32;
        self.last_timer = timer;

        match event {
            EntropyEventType::Keyboard => {
                if key == self.last_keys[0] && key == self.last_keys[1] {
                    return 0;
                }
                self.last_keys[1] = self.last_keys[0];
                self.last_keys[0] = key;
            }
            EntropyEventType::MouseClick => {
                if pos == self.last_pos[0] && pos == self.last_pos[1] {
                    return 0;
                }
                self.last_pos[1] = self.last_pos[0];
                self.last_pos[0] = pos;
            }
            _ => {}
        }

        pool.add_data(&key.to_le_bytes());
        pool.add_data(&pos.to_le_bytes());
        pool.add_data(data);

        if timer_kind() == TimerKind::None {
            delta /= HRT_NONE_ACCURACY_DERATING;
        }
        if delta >= (1 << (TIMER_ENTROPYBITS_DERATING + 1))
            && !(delta == self.last_deltas[0] && delta == self.last_deltas[1])
        {
            self.last_deltas[1] = self.last_deltas[0];
            self.last_deltas[0] = delta;
            let timer_bits = log_base2(delta) - TIMER_ENTROPYBITS_DERATING;
            ent_bits += timer_bits.min(self.max_timer_ent_bits);
        }
        self.increase(ent_bits);
        ent_bits
    }

    /// Add user-provided data (`AddData()`): the LZO-compressed data goes into
    /// the pool; entropy is estimated from the compressed size.
    pub fn add_data(
        &mut self,
        pool: &mut RandomPool,
        data: &[u8],
        ent_bits_incompr: f32,
        ent_bits_compr: f32,
    ) -> u32 {
        if data.is_empty() {
            return 0;
        }
        let compr = crate::lzo::compress(data);
        pool.add_data(&compr);
        let compr_len = if compr.len() > 4 { compr.len() - 4 } else { 1 };
        let bits = if compr_len >= data.len() {
            (data.len() as f64 * ent_bits_incompr as f64).min(MAX_TOTALENTBITS as f64)
        } else {
            (compr_len as f64 * ent_bits_compr as f64).min(MAX_TOTALENTBITS as f64)
        } as u32;
        self.increase(bits);
        bits
    }

    /// Add system entropy (`AddSystemEntropy()`).
    pub fn add_system_entropy(&mut self, pool: &mut RandomPool) {
        pool.randomize_system();
        self.increase(self.system_ent_bits);
    }

    /// Account for `bits` consumed by generated passwords
    /// (`ConsumeEntropyBits()`); returns the remaining estimate.
    pub fn consume_entropy_bits(&mut self, bits: u32) -> u32 {
        let max = self.entropy_bits_max();
        self.entropy_bits = max.saturating_sub(bits);
        self.entropy_bits
    }

    /// Entropy available, capped at the pool size (`EntropyBitsMax`).
    pub fn entropy_bits_max(&self) -> u32 {
        self.entropy_bits.min(MAX_ENTROPY)
    }

    pub fn entropy_bits(&self) -> u32 {
        self.entropy_bits
    }

    pub fn total_entropy_bits(&self) -> u32 {
        self.total_ent_bits
    }

    pub fn reset_counters(&mut self) {
        self.entropy_bits = 0;
        self.total_ent_bits = 0;
    }

    pub fn max_timer_entropy_bits(&self) -> u32 {
        self.max_timer_ent_bits
    }

    pub fn set_max_timer_entropy_bits(&mut self, v: u32) {
        self.max_timer_ent_bits = v;
    }

    pub fn system_entropy_bits(&self) -> u32 {
        self.system_ent_bits
    }

    pub fn set_system_entropy_bits(&mut self, v: u32) {
        self.system_ent_bits = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::{Jsf32, PoolCipherType};

    #[test]
    fn counters() {
        let mut pool = RandomPool::new(PoolCipherType::ChaCha20, Some(Jsf32::new()), false);
        let mut em = EntropyManager::new(4, 16);
        em.add_system_entropy(&mut pool);
        assert_eq!(em.entropy_bits(), 16);
        let bits = em.add_data(&mut pool, &[0u8; 1000], 6.0, 1.0);
        assert!(bits > 0 && bits < 1000, "highly compressible data gives little entropy: {bits}");
        assert_eq!(em.consume_entropy_bits(10), em.entropy_bits());
        // Repeated identical key presses give no entropy.
        em.add_event(&mut pool, EntropyEventType::Keyboard, 65, 0, &[], 1);
        em.add_event(&mut pool, EntropyEventType::Keyboard, 65, 0, &[], 1);
        assert_eq!(em.add_event(&mut pool, EntropyEventType::Keyboard, 65, 0, &[], 1), 0);
    }
}
