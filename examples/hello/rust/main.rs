fn main() {
    println!(
        "hello from Rust on {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
}
