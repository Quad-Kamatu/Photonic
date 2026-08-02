use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use photonic_core::raster::filter::gaussian_blur_gray;
use rayon::ThreadPoolBuilder;

const WIDTH: u32 = 4_000;
const HEIGHT: u32 = 3_000;
const RADIUS: f32 = 4.0;

fn gaussian_blur_gray_speedup(c: &mut Criterion) {
    let image: Vec<u8> = (0..WIDTH as usize * HEIGHT as usize)
        .map(|i| ((i * 31 + i / WIDTH as usize * 17) % 256) as u8)
        .collect();
    let serial_pool = ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .expect("one-worker Rayon pool");
    let mut group = c.benchmark_group("gaussian_blur_gray_4000x3000");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    group.bench_function("serial_one_rayon_worker", |b| {
        b.iter(|| {
            serial_pool.install(|| {
                black_box(gaussian_blur_gray(
                    black_box(&image),
                    black_box(WIDTH),
                    black_box(HEIGHT),
                    black_box(RADIUS),
                ))
            })
        })
    });
    group.bench_function("parallel_default_rayon_pool", |b| {
        b.iter(|| {
            black_box(gaussian_blur_gray(
                black_box(&image),
                black_box(WIDTH),
                black_box(HEIGHT),
                black_box(RADIUS),
            ))
        })
    });
    group.finish();
}

criterion_group!(benches, gaussian_blur_gray_speedup);
criterion_main!(benches);
