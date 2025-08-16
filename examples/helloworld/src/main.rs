use also_cache::cache::Cache;

fn main() {
    // let mut cache = Cache::new(64);
    //pub fn new(size: usize, weighter: We, hash_builder: B) -> Self {
    let cache = Cache::new(100);

    println!("Hello, world!");
    // let key = "hello";
    // let value = b"world";
    // cache.insert(key, value);

    // match cache.get(&key) {
    //     Some(val) => match std::str::from_utf8(val) {
    //         Ok(s) => println!("Value for key '{}': {}", key, s),
    //         Err(_) => println!("Value for key '{}': <binary data>", key),
    //     },
    //     None => println!("Key '{}' not found in cache.", key),
    // }
}
