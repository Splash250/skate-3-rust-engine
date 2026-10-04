use std::{io::Read, path::Path};
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn run() -> skate_accounts::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 || args[1] != "init" {
        return Err(skate_accounts::Error {
            code: "usage".into(),
            message: "skate-account init NEW_DIRECTORY ADMIN_USERNAME < private-password-file"
                .into(),
        });
    }
    let mut password = String::new();
    std::io::stdin()
        .take(1027)
        .read_to_string(&mut password)
        .map_err(|_| skate_accounts::Error {
            code: "input".into(),
            message: "cannot read password from stdin".into(),
        })?;
    skate_accounts::initialize(
        Path::new(&args[2]),
        &args[3],
        password.trim_end_matches(['\r', '\n']),
    )?;
    println!(
        "Account store and local TLS certificate initialized. No credentials were written to stdout."
    );
    Ok(())
}
