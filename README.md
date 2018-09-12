Pipe Logger Lib
====================

[![Build Status](https://travis-ci.org/magiclen/pipe-logger-lib.svg?branch=master)](https://travis-ci.org/magiclen/pipe-logger-lib)
[![Build status](https://ci.appveyor.com/api/projects/status/mp76f391o4s8h5uv/branch/master?svg=true)](https://ci.appveyor.com/project/magiclen/pipe-logger-lib/branch/master)

Stores, rotates, compresses process logs.

## Example

```rust
extern crate pipe_logger_lib;

use pipe_logger_lib::*;

use std::fs;
use std::path::Path;

let test_folder = {
  let folder = Path::join(&Path::join(Path::new("tests"), Path::new("out")), "log-example");

  fs::remove_dir_all(&folder);

  fs::create_dir_all(&folder).unwrap();

  folder
};

let test_log_file = Path::join(&test_folder, Path::new("mylog.txt"));

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