use criterion::{criterion_group, criterion_main, Criterion};
use kzg_bench::benches::poly::bench_new_poly_div;
use rust_kzg_zkcrypto::kzg_types::ZFr;
use rust_kzg_zkcrypto::poly::PolyData;

fn bench_new_poly_div_(c: &mut Criterion) {
    bench_new_poly_div::<ZFr, PolyData>(c);
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_new_poly_div_
}

criterion_main!(benches);
