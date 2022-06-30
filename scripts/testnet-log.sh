#!/bin/sh
cargo run --release -- start example/block_2.kdl > log.txt& PID=$!; sleep $1; kill $PID 
