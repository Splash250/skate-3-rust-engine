fn main() {
    if let Err(error) = skate_xiso::run(std::env::args_os().skip(1)) {
        eprintln!("skate-xiso: {error}");
        std::process::exit(2);
    }
}
