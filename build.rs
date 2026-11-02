//! Compiles translations, embeds Windows resources and links GLEW for libprojectM.

/// Known names for vcpkg's static GLEW library, in preferred order.
/// Names differ across vcpkg versions and triplets, so use the installed one.
#[cfg(windows)]
const GLEW_NAMES: &[&str] = &["glew32s", "libglew32", "glew32"];

/// Returns the vcpkg triplet matching the target architecture and CRT mode.
#[cfg(windows)]
fn vcpkg_triplet() -> Option<String> {
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        _ => return None,
    };
    let static_crt = std::env::var("CARGO_CFG_TARGET_FEATURE")
        .unwrap_or_default()
        .split(',')
        .any(|feature| feature == "crt-static");
    let suffix = if static_crt { "static" } else { "static-md" };
    Some(format!("{arch}-windows-{suffix}"))
}

/// Returns the known GLEW library installed in `lib`.
#[cfg(windows)]
fn glew_library(lib: &std::path::Path) -> Option<&'static str> {
    let present: Vec<String> = std::fs::read_dir(lib)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.eq_ignore_ascii_case("lib") {
                Some(path.file_stem()?.to_str()?.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect();
    GLEW_NAMES
        .iter()
        .copied()
        .find(|name| present.iter().any(|found| found == name))
}

fn main() {
    println!("cargo:rerun-if-changed=brand.toml");
    let manifest = std::fs::read_to_string("brand.toml").expect("read brand.toml");
    let field = |name: &str| -> String {
        manifest
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == name).then(|| value.trim().trim_matches('"').to_owned())
            })
            .unwrap_or_else(|| panic!("brand.toml is missing {name}"))
    };
    let name = field("name");
    let slug = field("slug");
    let bundle_id = field("macos_bundle_id");
    let credential_service = field("credential_service");
    assert!(credential_service.starts_with(&bundle_id));
    let generated = format!(
        "pub const PRODUCT_NAME: &str = {name:?};\npub const PRODUCT_SLUG: &str = {slug:?};\npub const MACOS_BUNDLE_ID: &str = {bundle_id:?};\npub const CREDENTIAL_SERVICE: &str = {credential_service:?};\n"
    );
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("brand.rs"), generated).expect("write generated brand constants");

    fastframe_i18n::build::compile_catalogs("assets/i18n");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=packaging/windows/tumlo.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("packaging/windows/tumlo.ico")
            .set("ProductName", &name)
            // Task Manager and the Windows mixer name the process by its description.
            .set("FileDescription", &name);
        if let Err(error) = resource.compile() {
            println!("cargo:warning=Windows resources not embedded: {error}");
        }
        // Static libprojectM requires the GLEW library installed by vcpkg.
        if std::env::var_os("CARGO_FEATURE_MILKDROP").is_some() {
            println!("cargo:rerun-if-env-changed=VCPKG_INSTALLATION_ROOT");
            if let (Some(root), Some(triplet)) =
                (std::env::var_os("VCPKG_INSTALLATION_ROOT"), vcpkg_triplet())
            {
                let lib = std::path::Path::new(&root)
                    .join("installed")
                    .join(triplet)
                    .join("lib");
                println!("cargo:rustc-link-search=native={}", lib.display());
                match glew_library(&lib) {
                    Some(name) => println!("cargo:rustc-link-lib=static={name}"),
                    None => {
                        // Include the directory listing in the error for diagnosis.
                        let listing = std::fs::read_dir(&lib)
                            .map(|entries| {
                                entries
                                    .flatten()
                                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_else(|error| format!("unreadable: {error}"));
                        println!(
                            "cargo:warning=no GLEW library in {}; it holds: {listing}",
                            lib.display()
                        );
                    }
                }
            }
        }
    }
}
