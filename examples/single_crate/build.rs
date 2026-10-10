// Compile the wire types into the host build script as well as the library.
// build.rs runs before the library exists, so it cannot import single_crate.
#[cfg(feature = "c-bindings")]
#[path = "src/types.rs"]
#[allow(dead_code)]
mod types;

fn main() -> std::io::Result<()> {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=src/types.rs");

    #[cfg(feature = "c-bindings")]
    {
        use postcard_bindgen::{PackageInfo, c, generate_bindings};
        use types::*;

        let output = std::path::PathBuf::from(
            // std::env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR to build scripts"),
            ".",
        );
        let package_name = "proto";
        c::build_package(
            &output,
            PackageInfo {
                name: package_name.into(),
                version: env!("CARGO_PKG_VERSION").try_into().unwrap(),
            },
            c::GenerationSettings::enable_all().module_structure(false),
            generate_bindings!(
                Gyro,
                Acc,
                ImuSample,
                SamplingRate,
                SensorConfig,
                Request,
                Response,
                Log,
                Log2
            ),
        )?;
        println!(
            "cargo::rustc-env=SINGLE_CRATE_C_BINDINGS_DIR={}",
            output.join(package_name).canonicalize()?.display()
        );
    }

    Ok(())
}
