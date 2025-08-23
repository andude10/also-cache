use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::Mutex;

use bincode::{
    config::standard,
    error::{DecodeError, EncodeError},
};
use serde::{Serialize, de::DeserializeOwned};

use crate::cache_shard::CacheShard;

pub const SMALL_THRESHOLD_RATIO: f64 = 0.1;
pub const MAIN_THRESHOLD_RATIO: f64 = 0.9;
pub const GHOST_THRESHOLD_RATIO: f64 = 0.5;
pub const MIN_SHARD_SIZE: usize = 8192;

pub struct AlsoCache<Key, We, B> {
    shards: Vec<Mutex<CacheShard<Key, B>>>,
    shard_mask: usize,
    weighter: We,
    hasher: B,
}

pub trait Weighter<Key>: Default + Clone {
    fn weight(&self, key: &Key, val: &Vec<u8>) -> u64;
}

#[derive(Debug, Clone, Default)]
pub struct DefaultWeighter;

impl<Key> Weighter<Key> for DefaultWeighter {
    fn weight(&self, _key: &Key, val: &Vec<u8>) -> u64 {
        val.len() as u64
    }
}

#[derive(Debug)]
pub enum CacheError {
    Decode(DecodeError),
    Encode(EncodeError),
    KeyNotFound,
}

impl<Key: Eq + Hash + Clone, We: Weighter<Key>, B: BuildHasher + Clone> AlsoCache<Key, We, B> {
    #[inline(always)]
    fn get_shard_index(&self, key: &Key) -> usize {
        let mut hasher = self.hasher.build_hasher();
        key.hash(&mut hasher);
        (hasher.finish() as usize) & self.shard_mask
    }

    pub fn with_estimated_count(
        estimated_items_count: usize,
        size: usize,
        weighter: We,
        hasher: B,
    ) -> Self {
        let shard_count = calculate_shard_count(size);
        let shard_mask = shard_count - 1;
        let per_shard_size = size / shard_count;
        let per_shard_items = estimated_items_count / shard_count;

        let shards = (0..shard_count)
            .map(|_| {
                Mutex::new(CacheShard::with_estimated_count(
                    per_shard_items,
                    ((per_shard_size as f64 * SMALL_THRESHOLD_RATIO) as u64).max(1),
                    ((per_shard_size as f64 * MAIN_THRESHOLD_RATIO) as u64).max(1),
                    ((per_shard_size as f64 * GHOST_THRESHOLD_RATIO) as u64).max(1),
                    hasher.clone(),
                ))
            })
            .collect();

        AlsoCache {
            shards,
            shard_mask,
            weighter,
            hasher,
        }
    }

    pub fn with(size: usize, weighter: We, hasher: B) -> Self {
        let shard_count = calculate_shard_count(size);
        let shard_mask = shard_count - 1;
        let per_shard_size = size / shard_count;

        let shards = (0..shard_count)
            .map(|_| {
                Mutex::new(CacheShard::new(
                    ((per_shard_size as f64 * SMALL_THRESHOLD_RATIO) as u64).max(1),
                    ((per_shard_size as f64 * MAIN_THRESHOLD_RATIO) as u64).max(1),
                    ((per_shard_size as f64 * GHOST_THRESHOLD_RATIO) as u64).max(1),
                    hasher.clone(),
                ))
            })
            .collect();

        AlsoCache {
            shards,
            shard_mask,
            weighter,
            hasher,
        }
    }

    #[inline(always)]
    pub fn get<V: DeserializeOwned>(&self, key: &Key) -> Result<V, CacheError> {
        let shard_idx = self.get_shard_index(key);
        let mut shard = self.shards[shard_idx].lock().unwrap();
        let bytes = shard.get_bytes(key).ok_or(CacheError::KeyNotFound)?;
        deserialize(bytes).map_err(CacheError::Decode)
    }

    #[inline(always)]
    pub fn insert<V: Serialize>(&self, key: Key, val: &V) -> Result<(), CacheError> {
        let bytes = serialize(val).map_err(CacheError::Encode)?;
        let weight = self.weighter.weight(&key, &bytes);
        let shard_idx = self.get_shard_index(&key);
        let mut shard = self.shards[shard_idx].lock().unwrap();
        shard.insert_bytes(key, weight, bytes);
        Ok(())
    }

    #[inline(always)]
    pub fn delete(&self, key: &Key) -> bool {
        let shard_idx = self.get_shard_index(key);
        let mut shard = self.shards[shard_idx].lock().unwrap();
        shard.delete(key)
    }

    pub fn print_queues(&self, limit: usize) {
        for (i, shard) in self.shards.iter().enumerate() {
            println!("Shard {}:", i);
            let shard = shard.lock().unwrap();
            shard.print_queues(limit);
        }
    }

    pub fn print_shard_utilization(&self) {
        let mut total_small = 0;
        let mut total_main = 0;
        let mut total_ghost = 0;
        let mut non_empty_shards = 0;

        println!("=== Shard Utilization Analysis ===");
        for (i, shard) in self.shards.iter().enumerate() {
            let shard = shard.lock().unwrap();
            let small_count = shard.get_small_size();
            let main_count = shard.get_main_size();
            let ghost_count = shard.get_ghost_size();

            if small_count + main_count + ghost_count > 0 {
                println!(
                    "Shard {}: Small={}, Main={}, Ghost={}",
                    i, small_count, main_count, ghost_count
                );
                non_empty_shards += 1;
            }

            total_small += small_count;
            total_main += main_count;
            total_ghost += ghost_count;
        }

        println!(
            "Total across {} shards: Small={}, Main={}, Ghost={}",
            self.shards.len(),
            total_small,
            total_main,
            total_ghost
        );
        println!(
            "Non-empty shards: {}/{}",
            non_empty_shards,
            self.shards.len()
        );
        println!("=== End Utilization Analysis ===");
    }

    pub fn get_utilization_stats(&self) -> (u64, u64, u64, usize) {
        let mut total_small = 0;
        let mut total_main = 0;
        let mut total_ghost = 0;
        let mut non_empty_shards = 0;

        for shard in &self.shards {
            let shard = shard.lock().unwrap();
            total_small += shard.get_small_size();
            total_main += shard.get_main_size();
            total_ghost += shard.get_ghost_size();

            if shard.get_small_size() + shard.get_main_size() + shard.get_ghost_size() > 0 {
                non_empty_shards += 1;
            }
        }

        (total_small, total_main, total_ghost, non_empty_shards)
    }
}

impl<Key: Eq + Hash + Clone> AlsoCache<Key, DefaultWeighter, ahash::RandomState> {
    pub fn default(size: usize) -> Self {
        AlsoCache::with(size, Default::default(), Default::default())
    }

    pub fn default_with_estimated_count(estimated_items_count: usize, size: usize) -> Self {
        AlsoCache::with_estimated_count(
            estimated_items_count,
            size,
            Default::default(),
            Default::default(),
        )
    }
}

#[inline(always)]
pub fn serialize<T: Serialize>(value: &T) -> Result<Vec<u8>, EncodeError> {
    bincode::serde::encode_to_vec(value, standard())
}

#[inline(always)]
pub fn deserialize<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
    bincode::serde::decode_from_slice::<T, _>(bytes, standard()).map(|(res, _)| res)
}

fn calculate_shard_count(total_size: usize) -> usize {
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    // don't over-shard small caches - ensure each shard has meaningful capacity
    let max_shards_by_size = (total_size / MIN_SHARD_SIZE).max(1);
    let max_shards_by_cpu = (cpu_count * 2).next_power_of_two().min(64);

    max_shards_by_size
        .min(max_shards_by_cpu)
        .max(1)
        .next_power_of_two()
}
