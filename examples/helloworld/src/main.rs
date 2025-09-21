use also_cache::sync::AlsoCache;

fn main() {
    // create a new cache with 1KB capacity
    let cache = AlsoCache::default(1024);

    let key = "hello".to_string();
    let value = "world";

    // Note: insert and get may return CacheError
    match cache.insert(key.clone(), &value) {
        Ok(()) => println!("Successfully inserted key '{}' with value '{}'", key, value),

        // errors for insert
        Err(e) => match e {
            also_cache::InsertCacheError::Encode(err) => {
                println!("Failed to encode value '{}': {:?}", value, err)
            }
            also_cache::InsertCacheError::Decode(err) => {
                println!("Failed to decode value for key '{}': {:?}", key, err)
            }
        },
    }

    // retrieve the value
    match cache.get::<String>(&key) {
        Ok(retrieved_value) => println!("Retrieved value for key '{}': {}", key, retrieved_value),

        // errors for get
        Err(e) => match e {
            also_cache::GetCacheError::Encode(err) => {
                println!("Failed to encode value '{}': {:?}", value, err)
            }
            also_cache::GetCacheError::Decode(err) => {
                println!("Failed to decode value for key '{}': {:?}", key, err)
            }
            also_cache::GetCacheError::KeyNotFound => {
                println!("Key '{}' not found in cache", key)
            }
        },
    }

    println!("\nHello world example completed!");
}
