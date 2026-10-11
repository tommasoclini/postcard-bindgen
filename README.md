# Postcard Bindgen

[![Build status](https://github.com/teamplayer3/postcard-bindgen/workflows/Rust/badge.svg)](https://github.com/teamplayer3/postcard-bindgen/actions)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](https://github.com/teamplayer3/postcard-bindgen)
[![Crates.io](https://img.shields.io/crates/v/postcard-bindgen.svg)](https://crates.io/crates/postcard-bindgen)
[![Documentation](https://docs.rs/postcard-bindgen/badge.svg)](https://docs.rs/postcard-bindgen)

`Postcard Bindgen` generates code for other languages to serialize and deserialize [postcard](https://github.com/jamesmunns/postcard) byte format. This facilitates communication between, for example, a microcontroller and a mobile app using the `postcard` crate.

Structs and enums can be annotated with `PostcardBindings` to generate code. The generated code can be exported as an npm package for JavaScript, a pip package for Python, or C header and source files.

## Supported Languages

* 🌐 **JavaScript** (>= ES2021)
* 🐍 **Python** (>= 3.10)
* **C** (C23 or later)

## Usage

> Rust 1.88 or later is required. Nightly Rust is no longer needed to generate bindings.

Annotate structs and enums with `Serialize`/`Deserialize` from the [serde crate](https://github.com/serde-rs/serde) and the `PostcardBindings` macro from this crate.

The process has two steps:
1. Annotate structs and enums in a library crate.
2. Import the annotated structs and enums in a binary crate, add the generation logic, and run the binary crate to generate the npm package.

> Enable the `generating` feature if `postcard-bindgen` is added as a dependency in the generation binary crate.

## Example

This example shows how to generate an npm package. The struct `Test` and the generation logic are in the same Rust file.

```rust
#[derive(Serialize, PostcardBindings)]
struct Test {
    name: u8,
    other: u16,
}

fn main() {
    javascript::build_package(
        std::env::current_dir().unwrap().as_path(),
        PackageInfo {
            name: "generation-test".into(),
            version: "0.1.0".try_into().unwrap(),
        },
        javascript::GenerationSettings::enable_all(),
        generate_bindings!(Test),
    )
    .unwrap();
}
```

The following code can now be used to serialize an object in JavaScript.

```js
import { serialize } from "generation-test";

const test = {
    name: "test",
    other: 23
}

const bytes = serialize("Test", test)
```

## C bindings

Enable `generating` and generate a package from every referenced Rust struct or
enum:

```rust
use postcard_bindgen::{c, generate_bindings, PackageInfo, PostcardBindings};
use serde::Serialize;

#[derive(Serialize, PostcardBindings)]
struct Packet {
    sequence: u32,
    label: String,
    samples: Vec<i16>,
}

fn main() -> std::io::Result<()> {
    c::build_package(
        std::env::current_dir()?.as_path(),
        PackageInfo {
            name: "telemetry".into(),
            version: "0.1.0".try_into().unwrap(),
        },
        c::GenerationSettings::enable_all()

            .max_sequence_length(16)
            .max_map_length(8),
        generate_bindings!(Packet),
    )
}
```

The `telemetry` directory contains `bindings.h`, `bindings.c`, `postcard.h`, and
`postcard.c`. The runtime files come from the pinned
[postcard-c](https://github.com/tommasoclini/postcard-c) submodule and are copied
unchanged. They are bundled in the published crate; generation needs no network
access.

```c
#include "bindings.h"

int main(void) {
    telemetry_packet packet = {0};
    packet.sequence = 42;
    packet.label.len = 2;
    packet.label.data = "ok";
    packet.samples.len = 1;
    packet.samples.items[0] = -10;

    uint8_t buffer[128];
    postcard_slice_t slice = postcard_slice_new(sizeof buffer, buffer);
    if (telemetry_packet_encode(&slice, &packet) != POSTCARD_SUCCESS) {
        return 1;
    }

    /* For decoding, cap is the number of available input bytes. */
    telemetry_packet decoded = {0};
    slice = postcard_slice_new(slice.len, buffer);
    return telemetry_packet_decode(&slice, &decoded) != POSTCARD_SUCCESS;
}
```

Compile both generated source files once, alongside your application:

```sh
cc -std=c23 -O2 -Itelemetry main.c telemetry/bindings.c telemetry/postcard.c -o app
```

Add `-flto` at compilation and linking to allow optimization across source files.
When using several generated packages, compile one shared copy of `postcard.c`.

The C backend borrows strings and byte sequences, uses fixed-capacity storage
for other collections, and never allocates memory:

| Rust type | C representation |
| --- | --- |
| Integers through 64 bits, `f32`, `f64`, `bool` | Fixed-width integers, `float`, `double`, `bool` |
| Structs and tuple structs | Structs; tuple fields are `field_0`, `field_1`, etc. |
| Unit structs | A placeholder `unit` member; no wire bytes |
| Enums | A `uint32_t`-backed enum `tag`, plus a `data` union for variants with payloads |
| `Option<T>` | `is_some` and `value`; only access `value` when present |
| Strings | `postcard_string_view_t`: `len` and `const char *data`; UTF-8 bytes with no trailing NUL |
| Fixed arrays | `items`; no length prefix on the wire |
| `u8` slices and vectors | `postcard_bytes_view_t`: `len` and `const uint8_t *data` |
| Other slices, vectors | `len` and inline `items` |
| Maps | `len` and `entries`, each with `key` and `value` |
| Tuples and ranges | Structs with `field_N`, or `start` and `end` |

Rust fixed array lengths and heapless capacities for inline collections are
preserved. Unbounded inline collections default to 64 sequence items and 64 map
entries; the settings above override these defaults. Strings and byte slices
have no generated length limits, including those from Rust heapless types.
Zero-capacity inline collections have a one-element storage placeholder but
still accept only empty values. `usize`/`isize` use 64-bit C
integers, following the existing registry's type metadata.

String and byte-sequence decoding uses the postcard-c `*_view` methods without
copying the payload. Decoded views point into the input buffer, which must remain
alive and unchanged while the decoded value is used. Empty views must not be
dereferenced. To encode a string or byte sequence, set `data` to your payload and
`len` to its byte length; a zero-length value may have a null data pointer.
Re-encode decoded views into a separate output buffer so encoding cannot overwrite
the borrowed input. Fixed `[u8; N]` arrays keep inline `items` and no length prefix.

Names begin with `<package>_`, followed by the Rust module path (excluding the
crate name) and type name. Names are converted to snake case: `ImuSample` becomes
`imu_sample`, and `HTTPResponse` becomes `http_response`. Digits stay attached,
so `Hz100` becomes `hz100`. `.module_structure(false)` omits module names.
C keywords used as fields gain `_field`; punctuation is escaped as `_u<hex>_`.
Generation reports name collisions, missing referenced types, recursive types,
empty enums, and unsupported 128-bit integers as `InvalidInput` errors before
writing output.

Generated bindings require C23. Encode and decode functions use
`postcard_slice_t slice[static 1]` and `type value[static 1]` parameters: callers
must pass pointers to valid objects. Encoding accepts a `const` value.
`[[nodiscard]]` asks the compiler to diagnose ignored return values. Enum tags
use an explicit `uint32_t` underlying type, independent of `-fshort-enums`.
Enum discriminants are decoded as `varint(u32)`, as specified in the
[Postcard tagged-union wire format](https://postcard.jamesmunns.com/wire-format#tagged-unions).

Encoding and decoding propagate `postcard_error_t` errors, validate
tags, UTF-8, nonzero integers, and storage bounds, and advance `slice.len` by the
bytes consumed. On error, the slice and destination may be partially updated.
The default settings generate deserialization only; `enable_all()` enables both,
and `.serialization(bool)` / `.deserialization(bool)` control them separately.

To build this repository from Git, initialize the runtime submodule first:

```sh
git submodule update --init --recursive
cargo run -p postcard-bindgen --example generate_c_bindings --features "std generating"
```

The example writes to `c-bindings`. C integration tests use `cc` by default;
set `CC` to the path of another C23 compiler if needed.

For a sensor protocol crate that generates C bindings from its own types in a
feature-gated build script, see [examples/single_crate](examples/single_crate).

## Test Bindings Fixture

From the workspace directory, generate the JavaScript bindings and serialized
fixture used by the TypeScript test project:

```bash
cargo +1.88 run -p postcard-bindgen --example generate_bindings --features="std generating heapless"
```

The example writes the `js-test-bindings` Node package to `postcard-bindgen/js-test-bindings` and the serialized Rust structs to `postcard-bindgen/serialized.bytes`, respectively. The TypeScript project at
`postcard-bindgen/test-bindings-proj` uses `js-test-bindings` as a local
dependency, loads `serialized.bytes`, and deserializes it with the generated
bindings. It compares that result with the value produced by serializing and
deserializing the same structure in JavaScript.

Then run the TypeScript project:

```bash
cd postcard-bindgen/test-bindings-proj
npm install
npm run run
```

## Type mappings

<table>
<tr><td> Type Name </td> <td> Rust </td> <td> Js </td><td> Python </td></tr>
<tr><td>Unit Type</td><td>

```rust
struct UnitStruct;
```
</td><td>

```javascript
{}
```
</td><td>

```python
class UnitStruct:
    pass

t = UnitStruct()
```
</td></tr>
<tr><td>Tuple Struct</td><td>

```rust
struct TupleStruct(u8, u16, u32);
```
</td><td>

```javascript
[123, 1234, 12345]
```
</td><td>

```python
class TupleStruct(tuple[u8]):
    ...

t = TupleStruct(123, 1234, 12345)
```
</td></tr>
<tr><td>Struct</td><td>

```rust
struct Struct {
    a: u8,
    b: u16
};
```
</td><td>

```javascript
{
    a: 123,
    b: 1234
}
```
</td><td>

```python
@dataclass
class Struct
    a: u8
    b: u16

t = Struct(a = 123, b = 1234)
```
</td></tr>
<tr><td>Enum</td><td>

```rust
enum Enum {
    A,
    B(u8),
    C {
        a: u8
    }
};
```
</td><td>

```javascript
{
    tag: "A",
},
{
    tag: "B",
    value: 123
},
{
    tag: "C",
    value: {
        a: 123
    }
}
```
</td><td>

```python
class Enum_A:
    pass

class Enum_B(tuple[u8]):
    ...

@dataclass
class Enum_C
    a: u8

Enum = Enum_A | Enum_B | Enum_C

a = Enum_A()
b = Enum_B(23)
c = Enum_C(a = 23)
```
</td></tr>
<tr><td>Option</td><td>

```rust
struct OptionTuple(Option<u8>);

struct OptionStruct {
    a: Option<u8>
}
```
</td><td>

```javascript
// OptionTuple(Some(123))
[123]
// OptionTuple(None)
[undefined]

// OptionStruct { a: Some(123) }
{
    a: 123
}
// OptionStruct { a: None }
{}
// or
{
    a: undefined
}
```
</td><td>

```python
# OptionTuple(Some(123))
OptionTuple(123)
# OptionTuple(None)
OptionTuple(None)

# OptionStruct { a: Some(123) }
OptionStruct(a = 123)
# OptionStruct { a: None }
OptionStruct(a = None)
```
</td></tr>
<tr><td>Map</td><td>

```rust
let map_string_key = HashMap::<String, u8>::new();

let map_any_key = HashMap::<u16, u8>::new();
```
</td><td>

```javascript
// map_string_key
{
    key: value
}

// map_any_key
new Map()
```
</td><td>

```python
# map_string_key
: Dict[str, u8] = {
    key: value
}

# map_any_key
: Dict[u16, u8] = {
    key: value
}
```
</td></tr>
</table>

### License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Postcard Bindgen by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions
