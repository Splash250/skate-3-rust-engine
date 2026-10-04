/// Return a packaged helper name for the current target.
pub fn name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn helper_name_matches_target() {
        let name = super::name("skate3setup");
        assert_eq!(name.ends_with(".exe"), cfg!(windows));
    }
}
