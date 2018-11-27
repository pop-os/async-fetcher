extern crate async_fetcher;
extern crate futures;
extern crate log;
extern crate reqwest;
extern crate tokio;

extern crate flate2;
extern crate xz2;

use async_fetcher::AsyncFetcher;
use futures::Future;
use reqwest::async::Client;
use std::{
    fs::File,
    io::{self, Write},
    sync::Arc,
};
use tokio::runtime::Runtime;

use flate2::write::GzDecoder;
use xz2::write::XzDecoder;

pub fn main() {
    init_logging().unwrap();

    let files: Vec<(String, String)> = vec![
        (
            "http://apt.pop-os.org/proprietary/dists/cosmic/Contents-amd64.xz".into(),
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
    let future_iterator = files.into_iter().map(move |(url, dest)| {
        // Store the fetched file into a temporary location.
        let temporary = [&dest, ".partial"].concat();

        // Construct a future which will download our file.
        let request = AsyncFetcher::new(url.clone())
            // Specify the destination path where a source file may already exist.
            .request_to_path(&client, dest.into())
            // Download the file to this temporary path (to prevent overwriting a good file).
            .then_download(temporary.into());


        // Dynamically choose the correct decompressor for the given file.
        let future: Box<dyn Future<Item = (), Error = io::Error> + Send> =
            if url.ends_with(".xz") {
                Box::new(request.then_process(move |file| Box::new(XzDecoder::new(file))))
            } else if url.ends_with(".gz") {
                Box::new(request.then_process(move |file| Box::new(GzDecoder::new(file))))
            } else {
                Box::new(request.then_rename())
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
    log::set_logger(&LOGGER).map(|()| log::set_max_level(LevelFilter::Debug))
}

struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) && record.target() == "async_fetcher" {
            eprintln!("{} - {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}
