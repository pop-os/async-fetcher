extern crate actix;
extern crate actix_web;

use self::actix_web::{fs, middleware, server, App};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub const FILES: &[(&str, &str, &str)] = &[
    (
        "Contents-all.xz",
        "37adf427913772253d743022f46eb8f244afd714ebf8745a874cd1c3851cefe8",
        "ad740b679291e333d35106735870cf7217d6c76801b45a01a8e7fde58c93aaf0",
    ),
    (
        "Contents-amd64.xz",
        "ff510123c0696bb4cb98ea3bf3940a6153dec2ed15dcb0f559f551a34a1f9de4",
        "03875b3caacbe5880f91c40d85891ec22acbab5561a3b4125d8825b7f6a1b5e9",
    ),
    (
        "Contents-i386",
        "67eb11e9bed8ac046572268f6748bf9dcf7848d771dd3343fba1490a7bbefb8a",
        "67eb11e9bed8ac046572268f6748bf9dcf7848d771dd3343fba1490a7bbefb8a",
    ),
];

pub const CACHE_DIR: &str = "tests/files/";

pub fn launch_server() -> u16 {
    let found = Arc::new(AtomicUsize::new(0));

    let found_ = found.clone();
    thread::spawn(move || {
        let sys = actix::System::new("async-fetcher-test");

        for port in 8080u16.. {
            let server = server::new(move || {
                App::new()
                    // enable logger
                    .middleware(middleware::Logger::default())
                    .handler(
                        "/",
                        fs::StaticFiles::new("tests/common/static").expect("static files not found")
                    )
            });

            if let Ok(handle) = server.bind(&format!("127.0.0.1:{}", port)) {
                handle.start();
                found_.store(port as usize, Ordering::SeqCst);
                break;
            }
        }

        let _ = sys.run();
    });

    loop {
        let port = found.load(Ordering::SeqCst) as u16;
        if port != 0 {
            return port;
        } else {
            thread::sleep(Duration::from_millis(1));
        }
    }
}
