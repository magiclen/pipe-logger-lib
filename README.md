Pipe Logger Lib
====================

[![CI](https://github.com/magiclen/pipe-logger-lib/actions/workflows/ci.yml/badge.svg)](https://github.com/magiclen/pipe-logger-lib/actions/workflows/ci.yml)

Stores, rotates, compresses process logs. `xz` library is used for a compression by default. 
But there exists an option to use `gzip` by passing flags `--no-default-features --features=gzip`
to cargo.

## Example

```rust
extern crate pipe_logger_lib;

use pipe_logger_lib::*;

use std::fs;
use std::path::Path;

let test_folder = {
  let folder = Path::new("tests").join("out").join("log-example");

  fs::remove_dir_all(&folder);

  fs::create_dir_all(&folder).unwrap();

  folder
};

let test_log_file = test_folder.join("mylog.txt");

let mut builder = PipeLoggerBuilder::new(&test_log_file);

builder
    .set_tee(Some(Tee::Stdout))
    .set_rotate(Some(RotateMethod::FileSize(30))) // bytes
    .set_count(Some(10))
    .set_compress(false);

{
    let mut logger = builder.build().unwrap();

    logger.write_line("Hello world!").unwrap();

    let rotated_log_file_1 = logger.write_line("This is a convenient logger.").unwrap().unwrap();

    logger.write_line("Other logs...").unwrap();
    logger.write_line("Other logs...").unwrap();

    let rotated_log_file_2 = logger.write_line("Rotate again!").unwrap().unwrap();

    logger.write_line("Ops!").unwrap();
}

fs::remove_dir_all(test_folder).unwrap();
```

Now, the contents of `test_log_file` are,

```text
Ops!
```

The contents of `rotated_log_file_1` are,

```text
Hello world!
This is a convenient logger.
```

The contents of `rotated_log_file_2` are,

```text
Other logs...
Other logs...
Rotate again!
```

## Crates.io

https://crates.io/crates/pipe-logger-lib

## Documentation

https://docs.rs/pipe-logger-lib

## Official CLI

https://crates.io/crates/pipe-logger

## License

[MIT](LICENSE)