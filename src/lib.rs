// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

//! Asynchronously fetch files from HTTP servers
//!
//! - Concurrently fetch multiple files at the same time.
//! - Define alternative mirrors for the source of a file.
//! - Use multiple concurrent connections per file.
//! - Use mirrors for concurrent connections.
//! - Resume a download which has been interrupted.
//! - Progress events for fetches
//!
//! ```
//! let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
//!
//! let shutdown = async_shutdown::Shutdown::new();
//!
//! let results_stream = Fetcher::default()
//!     // Define a max number of ranged connections per file.
//!     .connections_per_file(4)
//!     // Max size of a connection's part, concatenated on completion.
//!     .max_part_size(4 * 1024 * 1024)
//!     // The channel for sending progress notifications.
//!     .events(events_tx)
//!     // Maximum number of retry attempts.
//!     .retries(3)
//!     // Cancels the fetching process when a shutdown is triggered.
//!     .shutdown(shutdown)
//!     // How long to wait before aborting a download that hasn't progressed.
//!     .timeout(Duration::from_secs(15))
//!     // Finalize the struct into an `Arc` for use with fetching.
//!     .build()
//!     // Take a stream of `Source` inputs and generate a stream of fetches.
//!     // Spawns
//!     .stream_from(input_stream, 4)
//! ```

#[macro_use]
extern crate derive_new;
#[macro_use]
extern crate derive_setters;
#[macro_use]
extern crate log;
#[macro_use]
extern crate thiserror;

pub mod iface;

mod checksum;
mod checksum_system;
mod concatenator;
mod get;
mod get_many;
mod range;
mod source;
mod time;
mod utils;

pub use self::checksum::*;
pub use self::checksum_system::*;
pub use self::concatenator::*;
pub use self::source::*;

use self::get::{get, FetchLocation};
use self::get_many::get_many;
use self::time::{date_as_timestamp, update_modified};
use async_shutdown::Shutdown;
use futures::{
    prelude::*,
    stream::{self, StreamExt},
};

use http::StatusCode;
use http::{request::Builder as HttpBuilder, Request as HttpRequest};
use httpdate::HttpDate;
use isahc::config::RedirectPolicy;
use isahc::AsyncBody;
use isahc::{HttpClient as IsahcClient, Response as IsahcResponse};
use numtoa::NumToA;
#[cfg(feature = "reqwest")]
use reqwest::{
    Client as ReqwestClient, RequestBuilder as ReqwestBuilder, Response as ReqwestResponse,
};

use std::sync::atomic::Ordering;
use std::{
    fmt::Debug,
    io,
    path::Path,
    pin::Pin,
    sync::{atomic::AtomicU16, Arc},
    time::{Duration, UNIX_EPOCH},
};
use tokio::fs;
use tokio::sync::mpsc;

/// The result of a fetched task from a stream of input sources.
pub type AsyncFetchOutput<Data> = (Arc<Path>, Arc<Data>, Result<(), Error>);

/// A channel for sending `FetchEvent`s to.
pub type EventSender<Data> = mpsc::UnboundedSender<(Arc<Path>, Data, FetchEvent)>;

/// An error from the asynchronous file fetcher.
#[derive(Debug, Error)]
pub enum Error {
    #[error("task was canceled")]
    Canceled,
    #[error("http client error")]
    IsahcClient(#[source] isahc::Error),
    #[cfg(feature = "reqwest")]
    #[error("http client error")]
    ReqwestClient(#[source] reqwest::Error),
    #[error("unable to concatenate fetched parts")]
    Concatenate(#[source] io::Error),
    #[error("unable to create file")]
    FileCreate(#[source] io::Error),
    #[error("unable to set timestamp on {:?}", _0)]
    FileTime(Arc<Path>, #[source] io::Error),
    #[error("content length is an invalid range")]
    InvalidRange(#[source] io::Error),
    #[error("unable to remove file with bad metadata")]
    MetadataRemove(#[source] io::Error),
    #[error("destination has no file name")]
    Nameless,
    #[error("network connection was interrupted while fetching")]
    NetworkChanged,
    #[error("unable to open fetched part")]
    OpenPart(Arc<Path>, #[source] io::Error),
    #[error("destination lacks parent")]
    Parentless,
    #[error("connection timed out")]
    TimedOut,
    #[error("error writing to file")]
    Write(#[source] io::Error),
    #[error("network input error")]
    Read(#[source] io::Error),
    #[error("failed to rename partial to destination")]
    Rename(#[source] io::Error),
    #[error("server responded with an error: {}", _0)]
    Status(StatusCode),
    #[error("internal tokio join handle error")]
    TokioSpawn(#[source] tokio::task::JoinError),
    #[error("the request builder did not match the client used")]
    InvalidGetRequestBuilder,
}

impl From<isahc::Error> for Error {
    fn from(e: isahc::Error) -> Self {
        Self::IsahcClient(e)
    }
}

#[cfg(feature = "reqwest")]
impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::ReqwestClient(e)
    }
}

/// Events which are submitted by the fetcher.
#[derive(Debug)]
pub enum FetchEvent {
    /// States that we know the length of the file being fetched.
    ContentLength(u64),
    /// Notifies that the file has been fetched.
    Fetched,
    /// Notifies that a file is being fetched.
    Fetching,
    /// Reports the amount of bytes that have been read for a file.
    Progress(u64),
    /// Notification that a fetch is being re-attempted.
    Retrying,
}

/// An asynchronous file fetcher for clients fetching files.
///
/// The futures generated by the fetcher are compatible with single and multi-threaded
/// runtimes, allowing you to choose between the runtime that works best for your
/// application. A single-threaded runtime is generally recommended for fetching files,
/// as your network connection is unlikely to be faster than a single CPU core.
#[derive(new, Setters)]
pub struct Fetcher<Data> {
    /// Creates an instance of a client. The caller can decide if the instance
    /// is shared or unique.
    #[setters(skip)]
    client: Client,

    /// The number of concurrent connections to sustain per file being fetched.
    /// # Note
    /// Defaults to 1 connection
    #[new(value = "1")]
    connections_per_file: u16,

    /// Configure the delay between file requests.
    /// # Note
    /// Defaults to no delay
    #[new(value = "0")]
    delay_between_requests: u64,

    /// The number of attempts to make when a request fails.
    /// # Note
    /// Defaults to 3 retries.
    #[new(value = "3")]
    retries: u16,

    /// The maximum size of a part file when downloading in parts.
    /// # Note
    /// Defaults to 2 MiB.
    #[new(value = "2 * 1024 * 1024")]
    max_part_size: u32,

    /// Time in ms between progress messages
    /// # Note
    /// Defaults to 500.
    #[new(value = "500")]
    progress_interval: u64,

    /// The time to wait between chunks before giving up.
    #[new(default)]
    #[setters(strip_option)]
    timeout: Option<Duration>,

    /// Holds a sender for submitting events to.
    #[new(default)]
    #[setters(into)]
    #[setters(strip_option)]
    events: Option<Arc<EventSender<Arc<Data>>>>,

    /// Utilized to know when to shut down the fetching process.
    #[new(value = "Shutdown::new()")]
    shutdown: Shutdown,
}

/// The underlying Client used for the Fetcher
pub enum Client {
    Isahc(IsahcClient),
    #[cfg(feature = "reqwest")]
    Reqwest(ReqwestClient),
}

pub(crate) enum RequestBuilder {
    Http(HttpBuilder),
    #[cfg(feature = "reqwest")]
    Reqwest(ReqwestBuilder),
}

impl<Data> Default for Fetcher<Data> {
    fn default() -> Self {
        use isahc::config::Configurable;
        let client = IsahcClient::builder()
            // Keep a TCP connection alive for up to 90s
            .tcp_keepalive(Duration::from_secs(90))
            // Follow up to 10 redirect links
            .redirect_policy(RedirectPolicy::Limit(10))
            // Allow the server to be eager about sending packets
            .tcp_nodelay()
            // Cache DNS records for 24 hours
            .dns_cache(Duration::from_secs(60 * 60 * 24))
            .build()
            .expect("failed to create HTTP Client");

        Self::new(Client::Isahc(client))
    }
}

impl<Data: Send + Sync + 'static> Fetcher<Data> {
    /// Finalizes the fetcher to prepare it for fetch tasks.
    pub fn build(self) -> Arc<Self> {
        Arc::new(self)
    }

    /// Given an input stream of source fetches, returns an output stream of fetch results.
    ///
    /// Spawns up to `concurrent` + `1` number of concurrent async tasks on the runtime.
    /// One task for managing the fetch tasks, and one task per fetch request.
    pub fn stream_from(
        self: Arc<Self>,
        inputs: impl Stream<Item = (Source, Arc<Data>)> + Send + 'static,
        concurrent: usize,
    ) -> Pin<Box<dyn Stream<Item = AsyncFetchOutput<Data>> + Send + 'static>> {
        let shutdown = self.shutdown.clone();
        let cancel_trigger = shutdown.wait_shutdown_triggered();
        // Takes input requests and converts them into a stream of fetch requests.
        let stream = inputs
            .map(move |(Source { dest, urls, part }, extra)| {
                let fetcher = self.clone();
                async move {
                    if fetcher.delay_between_requests != 0 {
                        let delay = Duration::from_millis(fetcher.delay_between_requests);
                        tokio::time::sleep(delay).await;
                    }

                    tokio::spawn(async move {
                        let _token = match fetcher.shutdown.delay_shutdown_token() {
                            Ok(token) => token,
                            Err(_) => return (dest, extra, Err(Error::Canceled)),
                        };

                        let task = async {
                            match part {
                                Some(part) => {
                                    match fetcher.request(urls, part.clone(), extra.clone()).await {
                                        Ok(()) => {
                                            fs::rename(&*part, &*dest).await.map_err(Error::Rename)
                                        }
                                        Err(why) => Err(why),
                                    }
                                }
                                None => fetcher.request(urls, dest.clone(), extra.clone()).await,
                            }
                        };

                        let result = task.await;

                        (dest, extra, result)
                    })
                    .await
                    .unwrap()
                }
            })
            .buffer_unordered(concurrent)
            .take_until(cancel_trigger);

        Box::pin(stream)
    }

    /// Request a file from one or more URIs.
    ///
    /// At least one URI must be provided as a source for the file. Each additional URI
    /// serves as a mirror for failover and load-balancing purposes.
    pub async fn request(
        self: Arc<Self>,
        uris: Arc<[Box<str>]>,
        to: Arc<Path>,
        extra: Arc<Data>,
    ) -> Result<(), Error> {
        self.send(|| (to.clone(), extra.clone(), FetchEvent::Fetching));

        remove_parts(&to).await;

        let attempts = Arc::new(AtomicU16::new(0));

        let fetch = || async {
            loop {
                let task = self.clone().inner_request(
                    &self.client,
                    uris.clone(),
                    to.clone(),
                    extra.clone(),
                    attempts.clone(),
                );

                let result = task.await;

                if let Err(Error::NetworkChanged) | Err(Error::TimedOut) = result {
                    let mut attempts = 5;
                    while attempts != 0 {
                        tokio::time::sleep(Duration::from_secs(3)).await;

                        match &self.client {
                            Client::Isahc(client) => {
                                let future = head_isahc(client, &uris[0]);
                                let net_check =
                                    crate::utils::timed_interrupt(Duration::from_secs(3), future);

                                if net_check.await.is_ok() {
                                    tokio::time::sleep(Duration::from_secs(3)).await;
                                    break;
                                }
                            }
                            #[cfg(feature = "reqwest")]
                            Client::Reqwest(client) => {
                                let future = head_reqwest(client, &uris[0]);
                                let net_check =
                                    crate::utils::timed_interrupt(Duration::from_secs(3), future);

                                if net_check.await.is_ok() {
                                    tokio::time::sleep(Duration::from_secs(3)).await;
                                    break;
                                }
                            }
                        };

                        attempts -= 1;
                    }

                    self.send(|| (to.clone(), extra.clone(), FetchEvent::Retrying));
                    remove_parts(&to).await;
                    tokio::time::sleep(Duration::from_secs(3)).await;

                    continue;
                }

                return result;
            }
        };

        let task = async {
            let mut attempted = false;
            loop {
                if attempted {
                    self.send(|| (to.clone(), extra.clone(), FetchEvent::Retrying));
                }

                attempted = true;
                remove_parts(&to).await;

                let error = match fetch().await {
                    Ok(()) => return Ok(()),
                    Err(error) => error,
                };

                if let Error::Canceled = error {
                    return Err(error);
                }

                tokio::time::sleep(Duration::from_secs(3)).await;

                // Uncondtionally retry connection errors.
                if let Error::IsahcClient(ref error) = error {
                    use std::error::Error;
                    if let Some(source) = error.source() {
                        if let Some(error) = source.downcast_ref::<isahc::Error>() {
                            if error.is_network() {
                                error!("retrying due to connection error: {}", error);
                                continue;
                            }
                        }
                    }
                }

                #[cfg(feature = "reqwest")]
                if let Error::ReqwestClient(ref error) = error {
                    use std::error::Error;
                    if let Some(source) = error.source() {
                        if let Some(error) = source.downcast_ref::<reqwest::Error>() {
                            if error.is_connect() || error.is_request() {
                                error!("retrying due to connection error: {}", error);
                                continue;
                            }
                        }
                    }
                }

                error!("retrying after error encountered: {}", error);

                if attempts.fetch_add(1, Ordering::SeqCst) > self.retries {
                    return Err(error);
                }
            }
        };

        let result = task.await;

        remove_parts(&to).await;

        match result {
            Ok(()) => {
                self.send(|| (to.clone(), extra.clone(), FetchEvent::Fetched));

                Ok(())
            }
            Err(why) => Err(why),
        }
    }

    async fn inner_request(
        self: Arc<Self>,
        client: &Client,
        uris: Arc<[Box<str>]>,
        to: Arc<Path>,
        extra: Arc<Data>,
        attempts: Arc<AtomicU16>,
    ) -> Result<(), Error> {
        let mut length = None;
        let mut modified = None;
        let mut resume = 0;

        match client {
            Client::Isahc(client) => {
                let head_response = head_isahc(client, &*uris[0]).await?;

                if let Some(response) = head_response.as_ref() {
                    length = response.content_length();
                    modified = response.last_modified();
                }
            }
            #[cfg(feature = "reqwest")]
            Client::Reqwest(client) => {
                let head_response = head_reqwest(client, &*uris[0]).await?;

                if let Some(response) = head_response.as_ref() {
                    length = response.content_length();
                    modified = response.last_modified();
                }
            }
        }

        // If the file already exists, validate that it is the same.
        if to.exists() {
            if let (Some(length), Some(last_modified)) = (length, modified) {
                match fs::metadata(to.as_ref()).await {
                    Ok(metadata) => {
                        let modified = metadata.modified().map_err(Error::Write)?;
                        let ts = modified
                            .duration_since(UNIX_EPOCH)
                            .expect("time went backwards");

                        if metadata.len() == length {
                            if ts.as_secs() == date_as_timestamp(last_modified) {
                                info!("already fetched {}", to.display());
                                return Ok(());
                            } else {
                                error!("removing file with outdated timestamp: {:?}", to);
                                let _ = fs::remove_file(to.as_ref())
                                    .await
                                    .map_err(Error::MetadataRemove)?;
                            }
                        } else {
                            resume = metadata.len();
                        }
                    }
                    Err(why) => {
                        error!("failed to fetch metadata of {:?}: {}", to, why);
                        fs::remove_file(to.as_ref())
                            .await
                            .map_err(Error::MetadataRemove)?;
                    }
                }
            }
        }

        // If set, this will use multiple connections to download a file in parts.
        if self.connections_per_file > 1 {
            if let Some(length) = length {
                if supports_range(client, &*uris[0], resume, Some(length)).await? {
                    self.send(|| (to.clone(), extra.clone(), FetchEvent::ContentLength(length)));

                    if resume != 0 {
                        self.send(|| (to.clone(), extra.clone(), FetchEvent::Progress(resume)));
                    }

                    let result = get_many(
                        self.clone(),
                        to.clone(),
                        uris,
                        resume,
                        length,
                        modified,
                        extra,
                        attempts.clone(),
                    )
                    .await;

                    if let Err(why) = result {
                        return Err(why);
                    }

                    if let Some(modified) = modified {
                        update_modified(&to, modified)?;
                    }

                    return Ok(());
                }
            }
        }

        if let Some(length) = length {
            self.send(|| (to.clone(), extra.clone(), FetchEvent::ContentLength(length)));

            if resume > length {
                resume = 0;
            }
        }

        let mut request = match client {
            Client::Isahc(_) => RequestBuilder::Http(HttpRequest::get(&*uris[0])),
            #[cfg(feature = "reqwest")]
            Client::Reqwest(client) => RequestBuilder::Reqwest(client.get(&*uris[0])),
        };

        if resume != 0 {
            if let Ok(true) = supports_range(client, &*uris[0], resume, length).await {
                match request {
                    RequestBuilder::Http(inner) => {
                        request = RequestBuilder::Http(
                            inner.header("Range", range::to_string(resume, length)),
                        );
                    }
                    #[cfg(feature = "reqwest")]
                    RequestBuilder::Reqwest(inner) => {
                        request = RequestBuilder::Reqwest(
                            inner.header("Range", range::to_string(resume, length)),
                        );
                    }
                }
                self.send(|| (to.clone(), extra.clone(), FetchEvent::Progress(resume)));
            } else {
                resume = 0;
            }
        }

        let path = match crate::get(
            self.clone(),
            request,
            FetchLocation::create(to.clone(), resume != 0).await?,
            to.clone(),
            extra.clone(),
            attempts.clone(),
        )
        .await
        {
            Ok((path, _)) => path,
            Err(Error::Status(StatusCode::NOT_MODIFIED)) => to,

            // Server does not support if-modified-since
            Err(Error::Status(StatusCode::NOT_IMPLEMENTED)) => {
                let request = match client {
                    Client::Isahc(_) => RequestBuilder::Http(HttpRequest::get(&*uris[0])),
                    #[cfg(feature = "reqwest")]
                    Client::Reqwest(client) => RequestBuilder::Reqwest(client.get(&*uris[0])),
                };

                let (path, _) = crate::get(
                    self.clone(),
                    request,
                    FetchLocation::create(to.clone(), resume != 0).await?,
                    to.clone(),
                    extra.clone(),
                    attempts,
                )
                .await?;

                path
            }

            Err(why) => return Err(why),
        };

        if let Some(modified) = modified {
            update_modified(&path, modified)?;
        }

        Ok(())
    }

    fn send(&self, event: impl FnOnce() -> (Arc<Path>, Arc<Data>, FetchEvent)) {
        if let Some(sender) = self.events.as_ref() {
            let _ = sender.send(event());
        }
    }
}

async fn head_isahc(
    client: &IsahcClient,
    uri: &str,
) -> Result<Option<IsahcResponse<AsyncBody>>, Error> {
    let request = HttpRequest::head(uri).body(()).unwrap();

    match validate_isahc(client.send_async(request).await?).map(Some) {
        result @ Ok(_) => result,
        Err(Error::Status(StatusCode::NOT_MODIFIED))
        | Err(Error::Status(StatusCode::NOT_IMPLEMENTED)) => Ok(None),
        Err(other) => Err(other),
    }
}

#[cfg(feature = "reqwest")]
async fn head_reqwest(client: &ReqwestClient, uri: &str) -> Result<Option<ReqwestResponse>, Error> {
    let request = client.head(uri).build().unwrap();

    match validate_reqwest(client.execute(request).await?).map(Some) {
        result @ Ok(_) => result,
        Err(Error::Status(StatusCode::NOT_MODIFIED))
        | Err(Error::Status(StatusCode::NOT_IMPLEMENTED)) => Ok(None),
        Err(other) => Err(other),
    }
}

async fn supports_range(
    client: &Client,
    uri: &str,
    resume: u64,
    length: Option<u64>,
) -> Result<bool, Error> {
    match client {
        Client::Isahc(client) => {
            let request = HttpRequest::head(uri)
                .header("Range", range::to_string(resume, length).as_str())
                .body(())
                .unwrap();

            let response = client.send_async(request).await?;

            if response.status() == StatusCode::PARTIAL_CONTENT {
                if let Some(header) = response.headers().get("Content-Range") {
                    if let Ok(header) = header.to_str() {
                        if header.starts_with(&format!("bytes {}-", resume)) {
                            return Ok(true);
                        }
                    }
                }

                Ok(false)
            } else {
                validate_isahc(response).map(|_| false)
            }
        }
        #[cfg(feature = "reqwest")]
        Client::Reqwest(client) => {
            let request = client
                .head(uri)
                .header("Range", range::to_string(resume, length).as_str())
                .build()
                .unwrap();

            let response = client.execute(request).await?;

            if response.status() == StatusCode::PARTIAL_CONTENT {
                if let Some(header) = response.headers().get("Content-Range") {
                    if let Ok(header) = header.to_str() {
                        if header.starts_with(&format!("bytes {}-", resume)) {
                            return Ok(true);
                        }
                    }
                }

                Ok(false)
            } else {
                validate_reqwest(response).map(|_| false)
            }
        }
    }
}

fn validate_isahc(response: IsahcResponse<AsyncBody>) -> Result<IsahcResponse<AsyncBody>, Error> {
    let status = response.status();

    if status.is_informational() || status.is_success() {
        Ok(response)
    } else {
        Err(Error::Status(status))
    }
}

#[cfg(feature = "reqwest")]
fn validate_reqwest(response: ReqwestResponse) -> Result<ReqwestResponse, Error> {
    let status = response.status();

    if status.is_informational() || status.is_success() {
        Ok(response)
    } else {
        Err(Error::Status(status))
    }
}

trait ResponseExt {
    fn content_length(&self) -> Option<u64>;
    fn last_modified(&self) -> Option<HttpDate>;
}

impl<T> ResponseExt for IsahcResponse<T> {
    fn content_length(&self) -> Option<u64> {
        let header = self.headers().get("content-length")?;
        header.to_str().ok()?.parse::<u64>().ok()
    }

    fn last_modified(&self) -> Option<HttpDate> {
        let header = self.headers().get("last-modified")?;
        httpdate::parse_http_date(header.to_str().ok()?)
            .ok()
            .map(HttpDate::from)
    }
}

#[cfg(feature = "reqwest")]
impl ResponseExt for ReqwestResponse {
    fn content_length(&self) -> Option<u64> {
        let header = self.headers().get("content-length")?;
        header.to_str().ok()?.parse::<u64>().ok()
    }

    fn last_modified(&self) -> Option<HttpDate> {
        let header = self.headers().get("last-modified")?;
        httpdate::parse_http_date(header.to_str().ok()?)
            .ok()
            .map(HttpDate::from)
    }
}

/// Cleans up after a process that may have been aborted.
async fn remove_parts(to: &Path) {
    let original_filename = match to.file_name().and_then(|x| x.to_str()) {
        Some(name) => name,
        None => return,
    };

    if let Some(parent) = to.parent() {
        if let Ok(mut dir) = tokio::fs::read_dir(parent).await {
            while let Ok(Some(entry)) = dir.next_entry().await {
                if let Some(entry_name) = entry.file_name().to_str() {
                    if let Some(potential_part) = entry_name.strip_prefix(original_filename) {
                        if potential_part.starts_with(".part") {
                            let path = entry.path();
                            let _ = tokio::fs::remove_file(path).await;
                        }
                    }
                }
            }
        }
    }
}
