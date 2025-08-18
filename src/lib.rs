pub mod cache;

#[cfg(test)]
mod tests {
    use serde_derive::{Deserialize, Serialize};

    use crate::cache::{AlsoCache, CacheError};

    #[test]
    fn test_insert_and_get() {
        let mut cache = AlsoCache::new(2000); // size in bytes

        let key1 = "test_key".to_string();
        let val_str = "some value of type String".to_string();
        cache
            .insert(key1, &val_str)
            .expect("first insert should succeed");

        let retrieved_str: String = cache
            .get(&"test_key".to_string())
            .expect("get after insertion should succeed");

        assert_eq!(
            retrieved_str, val_str,
            "Retrieved value should match inserted value"
        );

        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct ExampleStruct(i32, Vec<String>, bool);

        let key2 = "test_key_struct".to_string();
        let val_struct = ExampleStruct(42, vec!["example".to_string(), "test".to_string()], true);
        cache
            .insert(key2, &val_struct)
            .expect("insert struct should succeed");

        let retrieved_struct: ExampleStruct = cache
            .get(&"test_key_struct".to_string())
            .expect("get struct after insertion should succeed");

        assert_eq!(
            retrieved_struct, val_struct,
            "Retrieved struct should match inserted struct"
        );

        cache.print_queues(5);
    }

    #[test]
    fn test_many_inserts_and_gets() {
        let mut cache = AlsoCache::new(2000); // size in bytes

        for i in 0..10000 {
            let key = format!("key_{}", i);
            let value = format!("value_{}", i);
            cache.insert(key, &value).expect("insert should succeed");

            for j in 0..50 {
                let key = format!("key_{}", j);
                let _: Result<String, CacheError> = cache.get(&key);
            }
        }

        cache.print_queues(10);

        let mut found_count = 0;
        for i in 0..10000 {
            let key = format!("key_{}", i);
            if let Ok(_) = cache.get::<String>(&key) {
                found_count += 1;
                //println!("Found key: {}", key);
            } else {
                let value = format!("value_{}", i);
                let _ = cache.insert(key.clone(), &value);
                let _ = cache.get::<String>(&key);
            }
        }

        assert!(found_count > 0, "At least some keys should be found");
        println!("Total keys found: {}", found_count);
        assert!(found_count == 50, "Expected 50 keys to be found"); //TODO: remove this assert / make more reasonable
    }
}
