# Asynchronous File Fetcher

Rust crate that provides a high level abstraction around the asynchronous [reqwest](https://crates.io/crates/reqwest) client. This abstraction is oriented towards making it easier to fetch and update files from remote locations. The primary purpose of this crate is to make it easy to fetch and decompress lists from apt repositories, for use in Pop!\_OS as a more efficient `apt` replacement with superior error handling capabilities.

The included `AsyncFetcher` is used to construct future(s) for execution on an asynchronous runtime, such as [tokio](https://tokio.rs/). The generated futures should be supplied to an asynchronous parallel runtime, to enable dispatching fetches across threads, in addition to processing multiple fetch tasks from the same thread. The general idea is to generate an iterator of fetch requests, and then combine these into a single future for parallel & asynchronous execution.

## Features

- Configurable API with optional state abstractions.
- Supports execution on an asynchronous parallel tokio runtime.
- Only fetches files when the existing file does not exist or is older than the server's copy.

> See the examples directory for an elaborate example.

## Example

Example partially taken from the example.rs example.

```rust
pub fn main() {
    init_logging().unwrap();

    let files: Vec<(String, &'static str, &'static str, String)> = vec![
        (
            // The URL to fetch the file from, if required.
            "http://apt.pop-os.org/proprietary/dists/cosmic/Contents-amd64.xz".into(),
            // The checksum of the file to be downloaded.
            "ed1c4d5b21086baa1dc98f912b5d52adaaeb248e35a9b27cd7e1f06a25781502",
            // The checksum of the file after it has been processed, at its destination.
            "8e03685a2420b63ad937ab1e741f00b19d4297122cb421dcbb0700d5c2cc145b",
            // The destination of the file after it has been processed.
            "Contents-amd64".into(),
        ),
        (
            "http://apt.pop-os.org/proprietary/dists/cosmic/Contents-all".into(),
            "ad740b679291e333d35106735870cf7217d6c76801b45a01a8e7fde58c93aaf0",
            "ad740b679291e333d35106735870cf7217d6c76801b45a01a8e7fde58c93aaf0",
            "Contents-all".into(),
        ),
        (
            "http://apt.pop-os.org/proprietary/dists/cosmic/Contents-i386.gz".into(),
            "379f7a6c108bd9feb63848110a556fffb15975cdb22e4eeb25844292cd4c9285",
            "67eb11e9bed8ac046572268f6748bf9dcf7848d771dd3343fba1490a7bbefb8a",
            "Contents-i386".into(),
        ),
    ];

    // Construct a multi-threaded Tokio runtime for handling our futures.
    let mut runtime = Runtime::new().unwrap();

    // Create an asynchronous reqwest Client that will be used to fetch all requests.
    let client = Arc::new(Client::new());

    // Construct an iterator of futures for fetching our files.
    let future_iterator = files
        .into_iter()
        .map(move |(url, fetched_sha256, dest_sha256, dest)| {
            // Store the fetched file into a temporary location.
            let temporary = [&dest, ".partial"].concat();

            // Ensure the checksums survive for the duration of the future.
            let fetched_checksum: Arc<str> = Arc::from(fetched_sha256);
            let dest_checksum: Arc<str> = Arc::from(dest_sha256);

            // Construct a future which will download our file. Note that what is being constructed is
            // a future, which means that no computations are being formed here. A data structure is
            // being created, which stores all of the state required for the computation, as well as
            // the instructions to be executed with that state.
            let request = AsyncFetcher::new(&client, url.clone())
                // Specify the destination path where a source file may already exist.
                // The destination will have the checksum verified.
                .request_with_checksum_to_path::<Sha256>(dest.into(), &dest_checksum)
                // Download the file to this temporary path (to prevent overwriting a good file).
                .then_download(temporary.into())
                // Validate the checksum of the fetched file against Sha256
                .with_checksum::<Sha256>(fetched_checksum);

            // Dynamically choose the correct decompressor for the given file.
            let future: Box<dyn Future<Item = (), Error = io::Error> + Send> =
                if url.ends_with(".xz") {
                    Box::new(
                        request
                            .then_process(move |file| Box::new(XzDecoder::new(file)))
                            .with_destination_checksum::<Sha256>(dest_checksum),
                    )
                } else if url.ends_with(".gz") {
                    Box::new(
                        request
                            .then_process(move |file| Box::new(GzDecoder::new(file)))
                            .with_destination_checksum::<Sha256>(dest_checksum),
                    )
                } else {
                    Box::new(request.then_rename().into_future())
                };

            future
        });

    // Join the iterator of futures into a single future for our runtime.
    let joined = futures::future::join_all(future_iterator);

    // Execute each future asynchronously and in parallel.
    let result = runtime.block_on(joined);

    eprintln!("result: {:?}", result);
}
```
