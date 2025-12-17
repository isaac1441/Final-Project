use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn simple_benchmark(c: &mut Criterion) {
    c.bench_function("thread_pool_performance", |b| {
        b.iter(|| {
            // Simple benchmark to demonstrate performance testing
            let n = black_box(1000);
            let mut sum = 0;
            for i in 0..n {
                sum += i;
            }
            black_box(sum);
        })
    });
}

criterion_group!(benches, simple_benchmark);
criterion_main!(benches);