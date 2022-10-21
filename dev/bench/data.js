window.BENCHMARK_DATA = {
  "lastUpdate": 1666370031874,
  "repoUrl": "https://github.com/racs4/Kindelia",
  "entries": {
    "Rust Benchmark": {
      "f9468955f163ca327f58eb674e55a6fa31bcdeef": {
        "commit": {
          "author": {
            "email": "rheidner.achiles@gmail.com",
            "name": "rheidner",
            "username": "racs4"
          },
          "committer": {
            "email": "rheidner.achiles@gmail.com",
            "name": "rheidner",
            "username": "racs4"
          },
          "distinct": true,
          "id": "f9468955f163ca327f58eb674e55a6fa31bcdeef",
          "message": "asd",
          "timestamp": "2022-10-21T13:25:43-03:00",
          "tree_id": "f8747f218a95e948360a36ae5c7cefdeb495b04d",
          "url": "https://github.com/racs4/Kindelia/commit/f9468955f163ca327f58eb674e55a6fa31bcdeef",
          "original_ref": "bench-ci-testing",
          "parent": "4c5aedb41883bb555c39376772a5ecea1c6b0c05"
        },
        "date": 1666370020287,
        "tool": "cargo",
        "benches": [
          {
            "name": "test",
            "value": 20157880,
            "range": "± 60069",
            "unit": "ns/iter"
          },
          {
            "name": "test2",
            "value": 10166543,
            "range": "± 66115",
            "unit": "ns/iter"
          }
        ]
      },
      "fdb1f0f52bf352b2c1b82e79eec215a3d01576f2": {
        "commit": {
          "author": {
            "email": "rheidner.achiles@gmail.com",
            "name": "rheidner",
            "username": "racs4"
          },
          "committer": {
            "email": "rheidner.achiles@gmail.com",
            "name": "rheidner",
            "username": "racs4"
          },
          "distinct": true,
          "id": "fdb1f0f52bf352b2c1b82e79eec215a3d01576f2",
          "message": "dasda",
          "timestamp": "2022-10-21T13:25:58-03:00",
          "tree_id": "7950efb8b3d6b19e3831c14a3aecebf4eb05acc1",
          "url": "https://github.com/racs4/Kindelia/commit/fdb1f0f52bf352b2c1b82e79eec215a3d01576f2",
          "original_ref": "bench-event",
          "parent": "beb3a173635a1a1d53be18ab02372688da8009c8"
        },
        "date": 1666370031177,
        "tool": "cargo",
        "benches": [
          {
            "name": "max_message_serialize",
            "value": 52651,
            "range": "± 1187",
            "unit": "ns/iter"
          },
          {
            "name": "max_message_deserialize",
            "value": 48424,
            "range": "± 324",
            "unit": "ns/iter"
          },
          {
            "name": "deserialize_block_with_txs",
            "value": 191337,
            "range": "± 17194",
            "unit": "ns/iter"
          }
        ]
      }
    }
  },
  "branches": {
    "bench-ci-testing": "f9468955f163ca327f58eb674e55a6fa31bcdeef",
    "bench-event": "fdb1f0f52bf352b2c1b82e79eec215a3d01576f2"
  }
}