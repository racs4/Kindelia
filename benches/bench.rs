#![feature(test)]
extern crate test;

// use rust_example::fib;

// use kindelia::
use test::Bencher;

#[bench]
fn tree(b: &mut Bencher) {
  b.iter(|| {
    std::thread::sleep(std::time::Duration::from_millis(10));
  })
}

#[bench]
fn bench_fib_10(b: &mut Bencher) {
  b.iter(|| {
    // let _ = fib(10);
  });
}

#[bench]
fn bench_fib_20(b: &mut Bencher) {
  b.iter(|| {
    // let _ = fib(20);
  });
}
