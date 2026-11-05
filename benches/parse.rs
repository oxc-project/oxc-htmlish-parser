use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use oxc_htmlish_parser::{Allocator, Dialect, Parser};

fn bench_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    let fixtures = [
        ("html", Dialect::Html, include_str!("../benchmark/fixtures/document.html")),
        ("vue", Dialect::Vue, include_str!("../benchmark/fixtures/component.vue")),
        ("angular", Dialect::Angular, include_str!("../benchmark/fixtures/angular.html")),
    ];
    for (name, dialect, source) in fixtures {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new(name, "fixture"), &source, |b, source| {
            b.iter(|| {
                let allocator = Allocator::default();
                black_box(Parser::new(&allocator, black_box(source), dialect).parse().unwrap());
            });
        });
    }

    // A run of non-void self-closing HTML elements exercises implicit-close traversal.
    for count in [100, 1_000] {
        let source = "<my-el />".repeat(count);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("html/self-closing", count),
            &source,
            |b, source| {
                b.iter(|| {
                    let allocator = Allocator::default();
                    black_box(
                        Parser::new(&allocator, black_box(source), Dialect::Html).parse().unwrap(),
                    );
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_parser);
criterion_main!(benches);
