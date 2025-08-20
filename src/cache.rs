use std::hash::{BuildHasher, Hash};

use ahash::RandomState;
use bincode::{
    config::standard,
    error::{DecodeError, EncodeError},
};
use serde::{Serialize, de::DeserializeOwned};

use crate::cache_nodes_arena::NodeArena;

#[derive(Debug)]
pub struct AlsoCache<Key, We, B> {
    arena: NodeArena<Key, B>,
    weighter: We,
}

pub trait Weighter {
    fn weight(&self, val: &Vec<u8>) -> usize;
}

#[derive(Debug, Clone)]
pub struct DefaultWeighter;

impl Weighter for DefaultWeighter {
    fn weight(&self, val: &Vec<u8>) -> usize {
        val.len()
    }
}

#[derive(Debug)]
pub enum CacheError {
    Decode(DecodeError),
    Encode(EncodeError),
    KeyNotFound,
}

impl<Key: Eq + Hash, We: Weighter, B: BuildHasher> AlsoCache<Key, We, B> {
    pub fn with(size: usize, weighter: We, hasher: B) -> Self {
        AlsoCache {
            arena: NodeArena::new(
                (size as f64 * 0.1) as usize,
                (size as f64 * 0.9) as usize,
                (size as f64 * 0.6) as usize,
                hasher,
            ),
            weighter,
        }
    }

    #[inline(always)]
    pub fn get<V: DeserializeOwned>(&mut self, key: &Key) -> Result<V, CacheError> {
        let bytes = self.arena.get_bytes(key).ok_or(CacheError::KeyNotFound)?;
        deserialize(bytes).map_err(CacheError::Decode)
    }

    #[inline(always)]
    pub fn insert<V: Serialize>(&mut self, key: Key, val: &V) -> Result<(), CacheError> {
        let bytes = serialize(val).map_err(CacheError::Encode)?;
        self.arena
            .insert_bytes(key, self.weighter.weight(&bytes), bytes);
        Ok(())
    }

    #[inline(always)]
    pub fn delete(&mut self, key: &Key) -> bool {
        self.arena.delete(key)
    }

    pub fn print_queues(&self, limit: usize) {
        self.arena.print_queues(limit);
    }
}

impl<Key: Eq + Hash> AlsoCache<Key, DefaultWeighter, RandomState> {
    pub fn new(size: usize) -> Self {
        let weighter = DefaultWeighter;
        let hasher = RandomState::new();
        AlsoCache::with(size, weighter, hasher)
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
