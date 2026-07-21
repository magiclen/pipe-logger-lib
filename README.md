Pipe Logger Lib
===============

[![CI](https://github.com/magiclen/pipe-logger-lib/actions/workflows/ci.yml/badge.svg)](https://github.com/magiclen/pipe-logger-lib/actions/workflows/ci.yml)

Stores, rotates, and compresses process logs.

`count` includes the active log file, so a count of 10 keeps up to 9 rotated files.

Call `PipeLogger::flush` to wait for queued compression work while keeping the logger open.

Call `PipeLogger::finish` to wait for queued compression work and report its final result.

## Example

```rust
use std::{error::Error, fs, path::Path};

use pipe_logger_lib::{CompressionMethod, PipeLoggerBuilder, RotateMethod, Tee};

fn main() -> Result<(), Box<dyn Error>> {
    let folder = Path::new("logs");

    fs::create_dir_all(folder)?;

    let mut builder = PipeLoggerBuilder::new(folder.join("application.log"));

    builder
        .set_tee(Some(Tee::Stdout))
        .set_rotate(Some(RotateMethod::FileSize(1024 * 1024)))
        .set_count(Some(10))
        .set_compression(Some(CompressionMethod::Xz(6)));

    let mut logger = builder.build()?;

    logger.write_line("The application started.")?;
    logger.finish()?;

    Ok(())
}
```

## Crates.io

https://crates.io/crates/pipe-logger-lib

## Documentation

https://docs.rs/pipe-logger-lib

## Official CLI

https://crates.io/crates/pipe-logger

## License

[MIT](LICENSE)
