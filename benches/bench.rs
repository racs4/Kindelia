#![feature(test)]
extern crate test;

// use rust_example::fib;

// use kindelia::
use test::Bencher;

// examples:

#[bench]
fn test(b: &mut Bencher) {
  b.iter(|| std::thread::sleep(std::time::Duration::from_millis(20)));
}

#[bench]
fn test2(b: &mut Bencher) {
  b.iter(|| std::thread::sleep(std::time::Duration::from_millis(10)));
}
