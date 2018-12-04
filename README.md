# Asynchronous File Fetcher

Rust crate that provides a high level abstraction around the asynchronous [reqwest](https://crates.io/crates/reqwest) client. This abstraction is oriented towards making it easier to fetch and update files from remote locations. The primary purpose of this crate is to make it easy to fetch and decompress lists from apt repositories, for use in Pop!\_OS as a more efficient `apt` replacement with superior error handling capabilities.

The included `AsyncFetcher` is used to construct future(s) for execution on an asynchronous runtime, such as [tokio](https://tokio.rs/). The generated futures should be supplied to an asynchronous parallel runtime, to enable dispatching fetches across threads, in addition to processing multiple fetch tasks from the same thread. The general idea is to generate an iterator of fetch requests, and then combine these into a single future for parallel & asynchronous execution.

## Features

- Configurable API with optional state abstractions.
- Supports parallel execution on an asynchronous tokio runtime.
- Decompress or process the fetched file before moving it into the destination.
- Optional checksums to validate the fetched and destination files.
- Only fetches files when the existing file:
  - does not exist
  - is older than the server's copy
  - checksum does not match the request
- Integration testing to prove that the crate's functionality works.

## Examples

See the provided [examples](./examples) and [integration tests](./tests).
