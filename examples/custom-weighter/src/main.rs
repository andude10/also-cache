use also_cache::{AlsoCache, Weighter};
use std::collections::hash_map::RandomState;

#[derive(Clone, Default)]
struct StringWeighter;

impl Weighter<u64> for StringWeighter {
    fn weight(&self, _key: &u64, val: &Vec<u8>) -> u64 {
        val.len() as u64
    }
}

fn main() {
    let cache = AlsoCache::with(100, StringWeighter, RandomState::default());
    cache.insert(1, &"1".to_string()).unwrap();
    cache.insert(54, &"54".to_string()).unwrap();
    cache.insert(1000, &"1000".to_string()).unwrap();
    assert_eq!(cache.get::<String>(&1000).unwrap(), "1000");
    println!("Weighter example completed successfully!");
}
