pub mod dev;

#[cfg(target_os = "linux")]
pub mod prod;

#[cfg(not(target_os = "linux"))]
pub mod prod {
    pub fn run() {
        eprintln!("the io_uring server needs Linux; run with APP_ENV=dev instead");
        std::process::exit(1);
    }
}
