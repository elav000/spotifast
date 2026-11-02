//! Current product identity, generated from the repository's brand manifest.

include!(concat!(env!("OUT_DIR"), "/brand.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_fields_are_consistent_and_source_neutral() {
        assert_eq!(PRODUCT_NAME, "Tumlo");
        assert_eq!(PRODUCT_SLUG, "tumlo");
        assert!(MACOS_BUNDLE_ID.ends_with(".Tumlo"));
        assert_eq!(CREDENTIAL_SERVICE, format!("{MACOS_BUNDLE_ID}.credentials"));
    }
}
