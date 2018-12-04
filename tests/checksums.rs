extern crate async_fetcher;
extern crate flate2;
extern crate futures;
extern crate log;
extern crate reqwest;
extern crate sha2;
extern crate tokio;
extern crate xz2;

use async_fetcher::{AsyncFetcher, FetchError};
use flate2::write::GzDecoder;
use futures::Future;
use reqwest::async::Client;
use sha2::Sha256;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::runtime::Runtime;
use xz2::write::XzDecoder;

mod common;
use common::{launch_server, CACHE_DIR, FILES};

#[test]
fn decompression_and_checksums() {
    let port = launch_server();

    let cache_path = [CACHE_DIR, "checksum"].concat();

    {
        let checksum_path = Path::new(&cache_path);
        if checksum_path.exists() {
            fs::remove_dir_all(&checksum_path).expect("failed to clean up");
        }

        fs::create_dir_all(&checksum_path).expect("failed to set up");
    }

    // Construct a multi-threaded Tokio runtime for handling our futures.
    let mut runtime = Runtime::new().expect("failed to create runtime");

    // Create an asynchronous reqwest Client that will be used to fetch all requests.
    let client = Arc::new(Client::new());

    // Construct an iterator of futures for fetching our files.
    let future_iterator = FILES.iter()
        .map(move |(name, compressed, decompressed)| (
            format!("http://127.0.0.1:{}/{}", port, name),
            compressed,
            decompressed,
            format!("{}/{}", cache_path, name)
        ))
        .map(move |(url, fetched_sha256, dest_sha256, dest)| {
            // Store the fetched file into a temporary location.
            let temporary = [&dest, ".partial"].concat();

            // Ensure the checksums survive for the duration of the future.
            let fetched_checksum: Arc<str> = Arc::from(*fetched_sha256);
            let dest_checksum: Arc<str> = Arc::from(*dest_sha256);

            let request = AsyncFetcher::new(&client, url.clone())
                .request_to_path_with_checksum::<Sha256>(dest.into(), &dest_checksum)
                .then_download(temporary.into())
                .with_checksum::<Sha256>(fetched_checksum);

            // Dynamically choose the correct decompressor for the given file.
            let future: Box<dyn Future<Item = (), Error = FetchError> + Send> =
                if url.ends_with(".xz") {
                    Box::new(
                        request
                            .then_process(move |file| Ok(Box::new(XzDecoder::new(file))))
                            .with_destination_checksum::<Sha256>(dest_checksum),
                    )
                } else if url.ends_with(".gz") {
                    Box::new(
                        request
                            .then_process(move |file| Ok(Box::new(GzDecoder::new(file))))
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
    runtime.block_on(joined).expect("runtime error");
    fs::remove_dir_all([CACHE_DIR, "checksum"].concat()).expect("failed to clean up");
}
