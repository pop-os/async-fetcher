extern crate async_fetcher;
extern crate flate2;
extern crate futures;
#[macro_use]
extern crate log;
extern crate reqwest;
extern crate sha2;
extern crate tokio;
extern crate xz2;

use async_fetcher::{AsyncFetcher, FetchError, FetchEvent};
use flate2::write::GzDecoder;
use futures::Future;
use reqwest::async::Client;
use std::sync::Arc;
use tokio::runtime::Runtime;
use xz2::write::XzDecoder;

pub fn main() {
    init_logging().unwrap();

    let files: Vec<(String, String)> = vec![
        (
            // The URL to fetch the file from, if required.
            "http://apt.pop-os.org/proprietary/dists/cosmic/Contents-amd64.xz".into(),
            // The destination of the file after it has been processed.
            "Contents-amd64".into(),
        ),
        (
            "http://apt.pop-os.org/proprietary/dists/cosmic/Contents-all".into(),
            "Contents-all".into(),
        ),
        (
            "http://apt.pop-os.org/proprietary/dists/cosmic/Contents-i386.gz".into(),
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
        .map(move |(url, dest)| {
            // Store the fetched file into a temporary location.
            let temporary = [&dest, ".partial"].concat();
            let temporary_ = temporary.clone();
            let url_ = url.clone();
            let dest_ = dest.to_owned();

            // Construct a future which will download our file. Note that what is being constructed is
            // a future, which means that no computations are being formed here. A data structure is
            // being created, which stores all of the state required for the computation, as well as
            // the instructions to be executed with that state.
            let request = AsyncFetcher::new(&client, url.clone())
                // Define how to handle the callbacks that occur.
                .with_progress_callback(move |event| {
                    match event {
                        FetchEvent::Get => {
                            info!("GET {}", url_)
                        }
                        FetchEvent::AlreadyFetched => {
                            info!("OK {}", url_)
                        }
                        FetchEvent::Processing => {
                            info!("Processing {}", temporary_)
                        }
                        FetchEvent::Progress(bytes) => {
                            info!("{} bytes written to {}", bytes, temporary_)
                        }
                        FetchEvent::Total(bytes) => {
                            info!("{} is {} bytes", temporary_, bytes)
                        }
                        FetchEvent::DownloadComplete => {
                            info!("{} is complete", temporary_)
                        }
                        FetchEvent::Finished => {
                            info!("{} is complete", dest_)
                        }
                    }
                })
                // Specify the destination path where a source file may already exist.
                // The destination will have the checksum verified.
                .request_to_path(dest.into())
                // Download the file to this temporary path (to prevent overwriting a good file).
                .then_download(temporary.into());

            // Dynamically choose the correct decompressor for the given file.
            let future: Box<dyn Future<Item = (), Error = FetchError> + Send> =
                if url.ends_with(".xz") {
                    Box::new(
                        request
                            .then_process(move |file| Ok(Box::new(XzDecoder::new(file))))
                            .into_future()
                    )
                } else if url.ends_with(".gz") {
                    Box::new(
                        request
                            .then_process(move |file| Ok(Box::new(GzDecoder::new(file))))
                            .into_future()
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

// Configuring the logger

use log::{Level, LevelFilter, Metadata, Record, SetLoggerError};

static LOGGER: SimpleLogger = SimpleLogger;

pub fn init_logging() -> Result<(), SetLoggerError> {
    log::set_logger(&LOGGER).map(|()| log::set_max_level(LevelFilter::Info))
}

struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata())
            && (record.target() == "async_fetcher" || record.target() == "example")
        {
            eprintln!("{} - {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}
