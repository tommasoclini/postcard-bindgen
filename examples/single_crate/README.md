# Single-crate sensor protocol

One crate owns the Rust wire types and generates C bindings from the same source
file when its `c-bindings` feature is enabled. The default library is `no_std`,
uses no heap allocation, and has no postcard-bindgen dependency.

The protocol includes:

- `Gyro`: X/Y/Z angular velocity in radians per second.
- `Acc`: X/Y/Z acceleration in meters per second squared.
- `ImuSample`: device timestamp, gyro, acceleration, and optional temperature.
- `SamplingRate` and `SensorConfig`: frequency and enabled sensors.
- `Request`: read a sample or configure the device.
- `Response`: a numbered sample, configuration acknowledgement, or error code.

## Generate C bindings

From this directory:

```sh
cargo build --features c-bindings
```

From the repository root, use
`cargo build --manifest-path examples/single_crate/Cargo.toml --features c-bindings`.
Initialize the repository's postcard-c submodule first if needed:

```sh
git submodule update --init --recursive
```

`build.rs` includes `src/types.rs` directly as a host-side module. The library also
compiles that file and re-exports its types from `src/lib.rs`. The conditional
`PostcardBindings` derives are enabled only with `c-bindings`. No separate
binding-generation crate or duplicate type definitions are needed.

The build script places the generated files in `proto/` inside this example
directory:

```text
bindings.h
bindings.c
postcard.h
postcard.c
```

The library exposes the exact directory as `single_crate::C_BINDINGS_DIR` when
`c-bindings` is enabled. Print it with this small example:

```sh
cargo run --quiet --features c-bindings --example bindings_dir
```

The build script regenerates bindings when `src/types.rs` changes. Add every new
custom type to its `generate_bindings!` list. If shared types start using another
crate, add that dependency under both `[dependencies]` and `[build-dependencies]`.

## Use the Rust protocol

A firmware application can depend on this crate without the generation feature:

```toml
[dependencies]
single-crate = { path = "../postcard-bindgen/examples/single_crate", default-features = false }
postcard = { version = "1", default-features = false }
```

Then serialize requests into a caller-owned buffer:

```rust
use single_crate::{Request, SamplingRate, SensorConfig};

let request = Request::Configure(SensorConfig {
    sampling_rate: SamplingRate::Hz100,
    gyro_enabled: true,
    acc_enabled: true,
});
let mut buffer = [0u8; 128];
let bytes = postcard::to_slice(&request, &mut buffer).unwrap();
// Send bytes over the application's transport.
```

Run binding generation as a native host build. With the current derive
implementation, the `c-bindings` feature also enables generator dependencies in
the library, so leave this feature disabled when building embedded firmware.
For example, this checks the default library on a Cortex-M target:

```sh
cargo check --no-default-features --target thumbv7em-none-eabihf
```

## Use the C protocol

`c/roundtrip.c` constructs a sample containing both gyro and acceleration,
serializes it, decodes it, and checks a second serialization. It writes the
postcard bytes to stdout.

From this directory, build and run it:

```sh
PROTOCOL_BINDINGS_DIR=$(cargo run --quiet --features c-bindings --example bindings_dir)
cc -std=c23 -O2 -I"$PROTOCOL_BINDINGS_DIR" \
    c/roundtrip.c \
    "$PROTOCOL_BINDINGS_DIR/bindings.c" \
    "$PROTOCOL_BINDINGS_DIR/postcard.c" \
    -o target/sensor-roundtrip
target/sensor-roundtrip > target/sample.postcard
```

Compile the generated `bindings.c` and runtime `postcard.c` once. C names use the
prefix `proto_`; for example, `proto_gyro` and
`proto_response_encode`. The build script omits the internal
Rust `types` module from these names and converts type and variant names to
snake case, for example `proto_imu_sample` and `proto_request_get_sample`.
The bindings require C23. Function parameters use `[static 1]`, so pass pointers
to valid slice and value objects. Check the `[[nodiscard]]` error result from
each encode or decode call.

## Validate the example

```sh
cargo test --all-features
```

The tests check Rust round trips and compile the C example, comparing its output
byte-for-byte with Rust's postcard serialization. A C23 compiler is required for
this test; set `CC` to its path if it is not named `cc`.

Field and enum variant order determine the wire format. These types describe
message payloads; the application supplies message framing and transport.
