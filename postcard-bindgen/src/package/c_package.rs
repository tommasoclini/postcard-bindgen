use std::{borrow::Borrow, fs, io, path::Path};

use postcard_bindgen_core::{
    code_gen::c::{generate, GenerationSettings},
    registry::ContainerCollection,
};

use super::PackageInfo;

/// Builds a C23 package containing `bindings.h`, `bindings.c`, `postcard.h`,
/// and `postcard.c`. Compile both source files once and add the package directory
/// to your C include path. Requires no runtime allocation.
/// Decoded strings and byte sequences borrow the input buffer, which must remain
/// alive and unchanged while those views are used.
///
/// The postcard-c runtime is embedded at Rust compile time, so generation never
/// needs Git or network access. Unsupported types return `InvalidInput` before
/// any output is written.
pub fn build_c_package(
    parent_dir: &Path,
    package_info: PackageInfo,
    gen_settings: impl Borrow<GenerationSettings>,
    containers: ContainerCollection,
) -> io::Result<()> {
    // A package name is one directory, never an absolute or traversal path.
    if package_info.name.is_empty()
        || package_info.name == "."
        || package_info.name == ".."
        || package_info.name.contains(['/', '\\'])
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "C package name must be a single directory name",
        ));
    }
    let bindings = generate(containers, gen_settings, &package_info.name)?;
    let dir = parent_dir.join(&package_info.name);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("bindings.h"), bindings.header)?;
    fs::write(dir.join("bindings.c"), bindings.source)?;
    fs::write(
        dir.join("postcard.h"),
        include_bytes!("../../vendor/postcard-c/postcard.h"),
    )?;
    fs::write(
        dir.join("postcard.c"),
        include_bytes!("../../vendor/postcard-c/postcard.c"),
    )?;
    Ok(())
}
