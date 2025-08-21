pub mod cache;
pub mod cache_nodes_arena;

#[cfg(test)]
mod tests {
    use serde_derive::{Deserialize, Serialize};

    use crate::cache::{AlsoCache, CacheError};

    #[test]
    fn test_insert_get_delete() {
        let mut cache = AlsoCache::default(2000); // size in bytes

        // Test inserting, retrieving and deleting a simple value
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

        let delete_res = cache.delete(&"test_key".to_string());
        assert_eq!(delete_res, true, "Delete should succeed");
        let retrieved_after_delete: Result<String, CacheError> = cache.get(&"test_key".to_string());
        assert!(
            matches!(retrieved_after_delete, Err(CacheError::KeyNotFound)),
            "Get after delete should fail for simple value"
        );

        // Test inserting, retrieving and deleting a value of a more complex type
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

        let delete_res = cache.delete(&"test_key_struct".to_string());
        assert_eq!(delete_res, true, "Delete should succeed");
        let retrieved_after_delete: Result<String, CacheError> =
            cache.get(&"test_key_struct".to_string());
        assert!(
            matches!(retrieved_after_delete, Err(CacheError::KeyNotFound)),
            "Get after delete should fail for struct value"
        );

        cache.print_queues(5);
    }

    #[test]
    fn test_many_inserts_and_gets() {
        let mut cache = AlsoCache::default(2000); // size in bytes

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
            if let Ok(value) = cache.get::<String>(&key) {
                found_count += 1;
                // assert that values in ranges 20..30 or 40..50 match expected value
                if (20..30).contains(&i) || (40..50).contains(&i) {
                    let expected_value = format!("value_{}", i);
                    assert_eq!(
                        value, expected_value,
                        "Value for key {} should match expected format",
                        key
                    );
                }
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

    #[test]
    fn test_many_deletes() {
        let mut cache = AlsoCache::default(2000); // size in bytes

        // insert many items first
        let num_items = 1000;
        for i in 0..num_items {
            let key = format!("delete_key_{}", i);
            let value = format!("delete_value_{}", i);
            cache.insert(key, &value).expect("insert should succeed");
        }

        // access some items to move them through different queues (small -> main -> ghost)
        for _ in 0..3 {
            for i in 0..100 {
                let key = format!("delete_key_{}", i);
                let _: Result<String, CacheError> = cache.get(&key);
            }
        }

        cache.print_queues(10);

        // delete every other item
        let mut deleted_count = 0;
        for i in (0..num_items).step_by(2) {
            let key = format!("delete_key_{}", i);
            if cache.delete(&key) {
                deleted_count += 1;
            }
        }

        println!("Deleted {} items", deleted_count);
        assert!(deleted_count > 0, "Should have deleted some items");

        // verify deleted items are no longer accessible
        for i in (0..num_items).step_by(2) {
            let key = format!("delete_key_{}", i);
            let result: Result<String, CacheError> = cache.get(&key);
            assert!(
                matches!(result, Err(CacheError::KeyNotFound)),
                "Deleted key {} should not be found",
                key
            );
        }

        // verify non-deleted items are still accessible
        let mut found_count = 0;
        for i in (1..num_items).step_by(2) {
            let key = format!("delete_key_{}", i);
            if let Ok(value) = cache.get::<String>(&key) {
                found_count += 1;
                assert_eq!(
                    value,
                    format!("delete_value_{}", i),
                    "Value should match for key {}",
                    key
                );
            }
        }

        println!("Found {} remaining items", found_count);
        assert!(found_count > 0, "Should have some remaining items");

        // test deleting non-existent keys
        for i in num_items..num_items + 10 {
            let key = format!("nonexistent_key_{}", i);
            let delete_result = cache.delete(&key);
            assert_eq!(
                delete_result, false,
                "Deleting non-existent key should return false"
            );
        }

        // test double deletion
        for i in (0..10).step_by(2) {
            let key = format!("delete_key_{}", i);
            let delete_result = cache.delete(&key);
            assert_eq!(delete_result, false, "Double deletion should return false");
        }

        cache.print_queues(10);
    }
}
