# 0.2.0

- Add content-type callback to manipulate the destination path name.
- Properly implement the `IntoFuture` trait for each fetcher state.
- Add the `FetcherExt` trait to allow the caller to manipulate a state's future.
- Modify the final return type to be the final destination as an `Arc<Path>`.

# 0.1.2

- Add progress callbacks to track the progress of a fetch.

# 0.1.1

- Improve error handling using the `failure` crate.
- Set length of the partial file before downloading.
- Rename `request_with_checksum_to_path` to `request_to_path_with_checksum`.
- Make integration testing possible with a static Actix HTTP server.
- Add an integration test for decompression and checksums.

# 0.1.0

Initial release
