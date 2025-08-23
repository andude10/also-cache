// This benchmark borrowed from `quick_cache` crate
// https://github.com/arthurprs/quick-cache/blob/master/benches/benchmarks.rs

use std::mem;

use criterion::{Criterion, criterion_group, criterion_main};
use rand::prelude::*;

use rand_distr::Zipf;

// Import our cache implementation
use also_cache::cache::AlsoCache;

pub fn r_benchmark(c: &mut Criterion) {
    const N_SAMPLES: usize = 1_000;
    for max_cache_size in [10_000, 1_000_000] {
        for s in [0.5, 0.75] {
            let mut g = c.benchmark_group(format!("Reads N={} S={}", max_cache_size, s));
            g.throughput(criterion::Throughput::Elements(N_SAMPLES as u64));
            g.bench_function(format!("qc {}", max_cache_size), |b| {
                let mut cache = AlsoCache::default(max_cache_size * mem::size_of::<usize>()); // original benchmark passed total number of elements, but we pass total size in bytes.
                let mut samples = (0usize..max_cache_size).collect::<Vec<_>>();
                let mut rng = SmallRng::seed_from_u64(1);
                samples.shuffle(&mut rng);
                samples.truncate((max_cache_size as f64 * s) as usize);
                for p in samples {
                    let _ = cache.insert(p, &p);
                }
                b.iter(|| {
                    let mut count = 0usize;
                    for i in 0..N_SAMPLES {
                        count += cache.get::<usize>(&i).is_ok() as usize;
                    }
                    count
                });
            });
        }
    }
}

pub fn rw_benchmark(c: &mut Criterion) {
    const N_SAMPLES: usize = 1_000;
    let mut print_times = 0;
    for population in [10_000.0, 1_000_000.0] {
        for s in [0.5, 0.75] {
            let mut g = c.benchmark_group(format!("Zipf N={} S={}", population, s));
            g.throughput(criterion::Throughput::Elements(N_SAMPLES as u64));
            for capacity_ratio in [0.05, 0.1, 0.15] {
                let capacity = (population * capacity_ratio) as usize;
                g.bench_function(format!("qc {}", capacity), |b| {
                    let mut hits = 0u64;
                    let mut misses = 0u64;
                    b.iter_batched_ref(
                        || {
                            let mut rng = SmallRng::seed_from_u64(1);
                            let dist = Zipf::new(population, s).unwrap();
                            let mut cache = AlsoCache::default(capacity * mem::size_of::<usize>());
                            for _ in 0..population as usize * 3 {
                                let sample = dist.sample(&mut rng) as usize;
                                let _ = cache.insert(sample, &sample);
                            }
                            (rng, dist, cache)
                        },
                        |(rng, dist, cache)| {
                            for _ in 0..N_SAMPLES {
                                let sample = dist.sample(rng) as usize;
                                if cache.get::<usize>(&sample).is_ok() {
                                    hits += 1;
                                } else {
                                    let _ = cache.insert(sample, &sample);
                                    misses += 1;
                                }
                            }
                            (hits, misses)
                        },
                        criterion::BatchSize::LargeInput,
                    );
                    print_times += 1;
                    if print_times % 10 == 0 {
                        eprintln!("Hit rate {:?}", hits as f64 / (hits + misses) as f64);
                    }
                });
            }
        }
    }
}

criterion_group!(benches, rw_benchmark);
criterion_main!(benches);
