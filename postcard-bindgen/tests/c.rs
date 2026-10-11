#![cfg(feature = "generating")]

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    num::NonZeroU32,
    path::Path,
    process::{Command, Stdio},
};

use postcard_bindgen::{c, generate_bindings, PackageInfo, PostcardBindings};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PostcardBindings, Debug, PartialEq)]
struct Child {
    value: u16,
}

#[derive(Serialize, Deserialize, PostcardBindings, Debug, PartialEq)]
struct Unit;

#[derive(Serialize, Deserialize, PostcardBindings, Debug, PartialEq)]
struct Tuple(u8, bool);

#[derive(Serialize, Deserialize, PostcardBindings, Debug, PartialEq)]
enum Choice {
    Idle,
    Tuple(u16, Child),
    Record { value: i32, flag: bool },
}

#[derive(Serialize, Deserialize, PostcardBindings, Debug, PartialEq)]
struct Packet {
    child: Child,
    unit: Unit,
    tuple: Tuple,
    choice: Choice,
    option: Option<(u16, bool)>,
    samples: Vec<i16>,
    bytes: Vec<u8>,
    fixed: [u8; 3],
    empty: [u16; 0],
    name: String,
    mapping: BTreeMap<String, u32>,
    range: std::ops::Range<u16>,
    nonzero: NonZeroU32,
    unsigned: u64,
    signed: i64,
    small: i8,
    big: f64,
    tiny: f32,
}

fn info(name: &str) -> PackageInfo {
    PackageInfo {
        name: name.into(),
        version: "0.1.0".try_into().unwrap(),
    }
}

fn compile(dir: &Path, driver: &str, flags: &[&str]) -> std::path::PathBuf {
    fs::write(dir.join("driver.c"), driver).unwrap();
    // A second consumer catches definitions accidentally left in the headers.
    fs::write(
        dir.join("consumer.c"),
        "#include \"bindings.h\"\nint consumer(void) { return POSTCARD_SUCCESS; }\n",
    )
    .unwrap();
    let executable = dir.join("test-c");
    let compiler = std::env::var_os("CC").unwrap_or_else(|| "cc".into());
    let output = Command::new(compiler)
        .args(["-std=c23", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .args(flags)
        .args(["bindings.c", "postcard.c", "driver.c", "consumer.c", "-o"])
        .arg(&executable)
        .current_dir(dir)
        .output()
        .expect("C backend tests require a C23 compiler (set CC to its path)");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    executable
}

#[test]
fn c23_function_contracts_and_fixed_enum_tags() {
    use postcard_bindgen::__private::{BindingsRegistry, EnumType};

    // Exercise the full postcard variant index range, including values that
    // cannot be represented by a C99 int-backed enum.
    let mut ty = EnumType::new();
    ty.register_variant("Last");
    ty.variants[0].index = u32::MAX as usize;
    let mut registry = BindingsRegistry::default();
    registry.register_enum_binding("Limit", "c", ty.clone());
    let temp = tempfile::tempdir().unwrap();
    c::build_package(
        temp.path(),
        info("contracts"),
        c::GenerationSettings::enable_all(),
        registry.into_entries(),
    )
    .unwrap();
    let dir = temp.path().join("contracts");
    let driver = r#"#include "bindings.h"
static_assert(sizeof(contracts_limit_tag) == sizeof(uint32_t));
int main(void) {
  uint8_t buffer[5] = {};
  postcard_slice_t slice = postcard_slice_new(sizeof buffer, buffer);
  contracts_limit value = {.tag = contracts_limit_last}, decoded = {};
  if (contracts_limit_encode(&slice, &value) != POSTCARD_SUCCESS) return 1;
  if (slice.len != 5 || buffer[4] != 15) return 2;
  for (size_t i = 0; i < 4; ++i) if (buffer[i] != 255) return 3;
  slice.len = 0;
  if (contracts_limit_decode(&slice, &decoded) != POSTCARD_SUCCESS) return 4;
  return decoded.tag != contracts_limit_last;
}
"#;
    assert!(
        Command::new(compile(&dir, driver, &["-O2", "-fshort-enums"]))
            .status()
            .unwrap()
            .success()
    );

    let compiler = std::env::var_os("CC").unwrap_or_else(|| "cc".into());
    for body in [
        "return contracts_limit_encode(nullptr, &value);",
        "return contracts_limit_decode(&slice, nullptr);",
        "contracts_limit_encode(&slice, &value); return 0;",
    ] {
        fs::write(
            dir.join("invalid.c"),
            format!(
                "#include \"bindings.h\"\nint main(void) {{\n\
                 uint8_t buffer[5];\n\
                 [[maybe_unused]] postcard_slice_t slice = postcard_slice_new(sizeof buffer, buffer);\n\
                 [[maybe_unused]] contracts_limit value = {{.tag = contracts_limit_last}};\n\
                 {body}\n}}\n"
            ),
        )
        .unwrap();
        let output = Command::new(&compiler)
            .args([
                "-std=c23",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-pedantic",
                "-fsyntax-only",
                "invalid.c",
            ])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "compiler accepted a contract violation: {body}"
        );
    }
    if let Some(index) = (u32::MAX as usize).checked_add(1) {
        ty.variants[0].index = index;
        let mut registry = BindingsRegistry::default();
        registry.register_enum_binding("Limit", "c", ty);
        let error = c::generate(
            registry.into_entries(),
            c::GenerationSettings::enable_all(),
            "contracts",
        )
        .unwrap_err();
        assert!(error.to_string().contains("u32 range"));
    }
}

#[test]
fn generated_c_enum_tags_always_use_u32_varints() {
    use postcard_bindgen::__private::{BindingsRegistry, EnumType};

    let mut registry = BindingsRegistry::default();
    let mut driver =
        String::from("#include \"bindings.h\"\n#include <string.h>\nint main(void) {\n");
    for (name, maximum) in [
        ("Zero", 0),
        ("Byte", 255),
        ("BeyondByte", 256),
        ("Word", 65535),
        ("BeyondWord", 65536),
        ("Full", u32::MAX),
    ] {
        let mut ty = EnumType::new();
        ty.register_variant("Last");
        ty.variants[0].index = maximum as usize;
        if maximum != 0 {
            ty.register_variant("First");
            ty.variants[1].index = 0;
        }
        registry.register_enum_binding(name, "c", ty);
        let c_name = match name {
            "BeyondByte" => "beyond_byte".to_owned(),
            "BeyondWord" => "beyond_word".to_owned(),
            _ => name.to_lowercase(),
        };
        let wire = postcard::to_vec::<_, 5>(&maximum).unwrap();
        let wire_bytes = wire
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let invalid = if maximum < 256 { 256 } else { 1 };
        let overflow = postcard::to_vec::<_, 5>(&(1u64 << 32)).unwrap();
        let overflow_bytes = overflow
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        driver.push_str(&format!(
            "{{\n\
             static_assert(sizeof(tags_{c_name}_tag) == sizeof(uint32_t));\n\
             tags_{c_name} value = {{.tag = tags_{c_name}_last}}, decoded = {{0}};\n\
             uint8_t buffer[5], expected[] = {{{wire_bytes}}};\n\
             postcard_slice_t slice = postcard_slice_new(sizeof buffer, buffer);\n\
             if (tags_{c_name}_encode(&slice, &value) != POSTCARD_SUCCESS) return 1;\n\
             if (slice.len != sizeof expected || memcmp(buffer, expected, sizeof expected) != 0) return 2;\n\
             slice = postcard_slice_new(slice.len, buffer);\n\
             if (tags_{c_name}_decode(&slice, &decoded) != POSTCARD_SUCCESS) return 3;\n\
             if (decoded.tag != tags_{c_name}_last || slice.len != sizeof expected) return 4;\n\
             slice = postcard_slice_new(sizeof buffer, buffer);\n\
             if (postcard_encode_variant(&slice, {invalid}) != POSTCARD_SUCCESS) return 5;\n\
             slice = postcard_slice_new(slice.len, buffer);\n\
             if (tags_{c_name}_decode(&slice, &decoded) != POSTCARD_ERROR_INVALID_INPUT) return 6;\n\
             if (decoded.tag != tags_{c_name}_last) return 7;\n\
             uint8_t overflow[] = {{{overflow_bytes}}};\n\
             slice = postcard_slice_new(sizeof overflow, overflow);\n\
             if (tags_{c_name}_decode(&slice, &decoded) != POSTCARD_ERROR_OVERFLOW) return 8;\n\
             if (slice.len != 0 || decoded.tag != tags_{c_name}_last) return 9;\n\
             uint8_t truncated[] = {{128}};\n\
             slice = postcard_slice_new(sizeof truncated, truncated);\n\
             if (tags_{c_name}_decode(&slice, &decoded) != POSTCARD_ERROR_INCOMPLETE_DATA) return 10;\n\
             if (slice.len != 0 || decoded.tag != tags_{c_name}_last) return 11;\n\
             uint8_t padded_zero[] = {{128, 128, 128, 128, 0}};\n\
             slice = postcard_slice_new(sizeof padded_zero, padded_zero);\n\
             if (tags_{c_name}_decode(&slice, &decoded) != POSTCARD_SUCCESS) return 12;\n\
             if (slice.len != sizeof padded_zero || decoded.tag != 0) return 13;\n\
             }}\n"
        ));
    }
    driver.push_str("return 0;\n}\n");
    let temp = tempfile::tempdir().unwrap();
    c::build_package(
        temp.path(),
        info("tags"),
        c::GenerationSettings::enable_all(),
        registry.into_entries(),
    )
    .unwrap();
    for flags in [vec!["-O0"], vec!["-O2", "-fshort-enums"]] {
        assert!(
            Command::new(compile(&temp.path().join("tags"), &driver, &flags))
                .status()
                .unwrap()
                .success()
        );
    }
}

#[test]
fn generated_c_interoperates_with_rust() {
    let temp = tempfile::tempdir().unwrap();
    c::build_package(
        temp.path(),
        info("fixture"),
        c::GenerationSettings::enable_all()
            .max_sequence_length(4)
            .max_map_length(2),
        generate_bindings!(Packet, Child, Unit, Tuple, Choice),
    )
    .unwrap();
    let dir = temp.path().join("fixture");
    assert_eq!(
        fs::read(dir.join("postcard.h")).unwrap(),
        include_bytes!("../vendor/postcard-c/postcard.h")
    );
    assert_eq!(
        fs::read(dir.join("postcard.c")).unwrap(),
        include_bytes!("../vendor/postcard-c/postcard.c")
    );
    let mut mapping = BTreeMap::new();
    mapping.insert("a".into(), 300);
    mapping.insert("b".into(), 400);
    for mode in 0..3 {
        let expected = Packet {
            child: Child { value: 300 },
            unit: Unit,
            tuple: Tuple(42, true),
            choice: match mode {
                0 => Choice::Idle,
                1 => Choice::Tuple(700, Child { value: 500 }),
                _ => Choice::Record {
                    value: -1000,
                    flag: true,
                },
            },
            option: if mode == 0 { None } else { Some((900, false)) },
            samples: vec![-1, 0, i16::MIN, i16::MAX],
            bytes: vec![0, 128, 255, 42],
            fixed: [1, 2, 3],
            empty: [],
            name: "h\u{e9}\0".into(),
            mapping: mapping.clone(),
            range: 10..300,
            nonzero: NonZeroU32::new(100).unwrap(),
            unsigned: u64::MAX,
            signed: i64::MIN,
            small: -128,
            big: -2.5,
            tiny: 1.5,
        };
        let wire = postcard::to_vec::<_, 256>(&expected).unwrap().to_vec();
        for flags in [vec!["-O0"], vec!["-O2", "-DNDEBUG"]] {
            let driver = DRIVER.replace("@MODE@", &mode.to_string());
            let executable = compile(&dir, &driver, &flags);
            let mut child = Command::new(executable)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&wire).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, wire);
            assert_eq!(
                postcard::from_bytes::<Packet>(&output.stdout).unwrap(),
                expected
            );
        }
    }
}

const DRIVER: &str = r#"#include "bindings.h"
#include <stdio.h>
#include <string.h>
#define CHECK(x) do { if (!(x)) { fprintf(stderr, "failed at line %d: %s\n", __LINE__, #x); return 1; } } while (0)
int main(void) {
  fixture_packet input = {0}, decoded = {0};
  input.child.value = 300;
  input.tuple.field_0 = 42;
  input.tuple.field_1 = true;
  input.choice.tag = @MODE@;
  if (@MODE@ == 1) {
    input.choice.data.tuple.field_0 = 700;
    input.choice.data.tuple.field_1.value = 500;
  } else if (@MODE@ == 2) {
    input.choice.data.record.value = -1000;
    input.choice.data.record.flag = true;
  }
  input.option.is_some = @MODE@ != 0;
  input.option.value.field_0 = 900;
  input.option.value.field_1 = false;
  input.samples.len = 4;
  input.samples.items[0] = -1;
  input.samples.items[1] = 0;
  input.samples.items[2] = INT16_MIN;
  input.samples.items[3] = INT16_MAX;
  input.bytes.len = 4;
  input.bytes.data = (const uint8_t[]){0, 128, 255, 42};
  input.fixed.items[0] = 1;
  input.fixed.items[1] = 2;
  input.fixed.items[2] = 3;
  input.name.len = 4;
  input.name.data = "h\xc3\xa9\0";
  input.mapping.len = 2;
  input.mapping.entries[0].key.len = 1;
  input.mapping.entries[0].key.data = "a";
  input.mapping.entries[0].value = 300;
  input.mapping.entries[1].key.len = 1;
  input.mapping.entries[1].key.data = "b";
  input.mapping.entries[1].value = 400;
  input.range.start = 10;
  input.range.end = 300;
  input.nonzero = 100;
  input.unsigned_field = UINT64_MAX;
  input.signed_field = INT64_MIN;
  input.small = -128;
  input.big = -2.5;
  input.tiny = 1.5f;
  uint8_t bytes[256], rust[256];
  postcard_slice_t slice = postcard_slice_new(sizeof bytes, bytes);
  CHECK(fixture_packet_encode(&slice, &input) == POSTCARD_SUCCESS);
  size_t len = slice.len;
  size_t rust_len = fread(rust, 1, sizeof rust, stdin);
  CHECK(rust_len == len && memcmp(rust, bytes, len) == 0);
  slice = postcard_slice_new(rust_len, rust);
  CHECK(fixture_packet_decode(&slice, &decoded) == POSTCARD_SUCCESS);
  CHECK(slice.len == rust_len);
  CHECK(decoded.child.value == 300 && decoded.nonzero == 100);
  CHECK(decoded.choice.tag == @MODE@);
  if (@MODE@ == 1) CHECK(decoded.choice.data.tuple.field_1.value == 500);
  if (@MODE@ == 2) CHECK(decoded.choice.data.record.value == -1000);
  CHECK(decoded.option.is_some == (@MODE@ != 0));
  CHECK(decoded.samples.len == 4 && decoded.samples.items[2] == INT16_MIN);
  CHECK(decoded.bytes.len == 4 && memcmp(decoded.bytes.data, input.bytes.data, 4) == 0);
  CHECK(decoded.name.len == 4 && memcmp(decoded.name.data, input.name.data, 4) == 0);
  slice = postcard_slice_new(sizeof bytes, bytes);
  CHECK(fixture_packet_encode(&slice, &decoded) == POSTCARD_SUCCESS);
  CHECK(slice.len == rust_len && memcmp(bytes, rust, rust_len) == 0);
  CHECK(fwrite(bytes, 1, slice.len, stdout) == slice.len);
  return 0;
}
"#;

#[derive(PostcardBindings)]
#[allow(dead_code)]
struct Checks {
    string: String,
    sequence: Vec<u16>,
    nonzero: NonZeroU32,
    number: u64,
}

#[test]
fn generated_c_views_borrow_payloads_and_validate_inputs() {
    #[derive(Serialize, PostcardBindings)]
    struct Views<'a> {
        text: &'a str,
        bytes: &'a [u8],
        fixed: [u8; 3],
        signed: Vec<i8>,
        nonzero: Vec<std::num::NonZeroU8>,
    }
    let temp = tempfile::tempdir().unwrap();
    c::build_package(
        temp.path(),
        info("views"),
        c::GenerationSettings::enable_all().max_sequence_length(3),
        generate_bindings!(Views),
    )
    .unwrap();
    let driver = r#"#include "bindings.h"
#include <stdio.h>
#include <string.h>
#define CHECK(x) do { if (!(x)) { fprintf(stderr, "failed at line %d: %s\n", __LINE__, #x); return 1; } } while (0)
int main(void) {
  uint8_t wire[] = {4, 'h', 0xc3, 0xa9, 0, 3, 0, 128, 255, 1, 2, 3, 2, 255, 128, 2, 1, 255};
  postcard_slice_t slice = postcard_slice_new(sizeof wire, wire);
  views_views decoded = {0};
  CHECK(views_views_decode(&slice, &decoded) == POSTCARD_SUCCESS);
  CHECK(slice.len == sizeof wire);
  CHECK(decoded.text.data == (const char *)wire + 1 && decoded.text.len == 4);
  CHECK(decoded.bytes.data == wire + 6 && decoded.bytes.len == 3);
  CHECK(decoded.fixed.items[2] == 3);
  CHECK(decoded.signed_field.items[0] == -1 && decoded.signed_field.items[1] == INT8_MIN);
  CHECK(decoded.nonzero.items[1] == 255);
  uint8_t output[32];
  slice = postcard_slice_new(sizeof output, output);
  CHECK(views_views_encode(&slice, &decoded) == POSTCARD_SUCCESS);
  CHECK(slice.len == sizeof wire && memcmp(output, wire, sizeof wire) == 0);
  CHECK(fwrite(output, 1, slice.len, stdout) == slice.len);

  views_views empty = {0};
  slice = postcard_slice_new(sizeof output, output);
  CHECK(views_views_encode(&slice, &empty) == POSTCARD_SUCCESS);
  size_t len = slice.len;
  slice = postcard_slice_new(len, output);
  CHECK(views_views_decode(&slice, &empty) == POSTCARD_SUCCESS);
  CHECK(slice.len == len && empty.text.len == 0 && empty.bytes.len == 0);
  CHECK(empty.text.data == (const char *)output + 1 && empty.bytes.data == output + 2);

  empty.text = (postcard_string_view_t){.data = nullptr, .len = 1};
  slice = postcard_slice_new(sizeof output, output);
  CHECK(views_views_encode(&slice, &empty) == POSTCARD_ERROR_INVALID_INPUT);
  empty.text.len = 0;
  empty.bytes = (postcard_bytes_view_t){.data = nullptr, .len = 1};
  slice.len = 0;
  CHECK(views_views_encode(&slice, &empty) == POSTCARD_ERROR_INVALID_INPUT);
  empty.bytes = (postcard_bytes_view_t){.data = wire, .len = 4};
  slice.len = 0;
  CHECK(views_views_encode(&slice, &empty) == POSTCARD_SUCCESS);

  uint8_t truncated_text[] = {2, 'a'};
  slice = postcard_slice_new(sizeof truncated_text, truncated_text);
  const char *text = decoded.text.data;
  CHECK(views_views_decode(&slice, &decoded) == POSTCARD_ERROR_INCOMPLETE_DATA);
  CHECK(slice.len == 0 && decoded.text.data == text && decoded.text.len == 4);
  uint8_t truncated_bytes[] = {0, 2, 1};
  slice = postcard_slice_new(sizeof truncated_bytes, truncated_bytes);
  const uint8_t *bytes = decoded.bytes.data;
  CHECK(views_views_decode(&slice, &decoded) == POSTCARD_ERROR_INCOMPLETE_DATA);
  CHECK(slice.len == 1 && decoded.bytes.data == bytes && decoded.bytes.len == 3);
  uint8_t oversized_bytes[] = {0, 4, 1, 2, 3, 4, 0, 0, 0, 0, 0};
  slice = postcard_slice_new(sizeof oversized_bytes, oversized_bytes);
  CHECK(views_views_decode(&slice, &decoded) == POSTCARD_SUCCESS);
  CHECK(decoded.bytes.len == 4 && decoded.bytes.data == oversized_bytes + 2);
  uint8_t overflow[] = {0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 2};
  slice = postcard_slice_new(sizeof overflow, overflow);
  CHECK(views_views_decode(&slice, &decoded) == POSTCARD_ERROR_OVERFLOW);
  CHECK(slice.len == 1);

  char long_text[600];
  uint8_t long_bytes[600], long_wire[1300], roundtrip[1300];
  memset(long_text, 'a', sizeof long_text);
  memset(long_bytes, 255, sizeof long_bytes);
  views_views long_value = {
    .text = {.data = long_text, .len = sizeof long_text},
    .bytes = {.data = long_bytes, .len = sizeof long_bytes},
  };
  slice = postcard_slice_new(sizeof long_wire, long_wire);
  CHECK(views_views_encode(&slice, &long_value) == POSTCARD_SUCCESS);
  len = slice.len;
  slice = postcard_slice_new(len, long_wire);
  CHECK(views_views_decode(&slice, &decoded) == POSTCARD_SUCCESS);
  CHECK(slice.len == len && decoded.text.len == 600 && decoded.bytes.len == 600);
  CHECK(decoded.text.data == (const char *)long_wire + 2);
  CHECK(decoded.bytes.data == long_wire + 604);
  CHECK(memcmp(decoded.text.data, long_text, 600) == 0);
  CHECK(memcmp(decoded.bytes.data, long_bytes, 600) == 0);
  slice = postcard_slice_new(sizeof roundtrip, roundtrip);
  CHECK(views_views_encode(&slice, &decoded) == POSTCARD_SUCCESS);
  CHECK(slice.len == len && memcmp(roundtrip, long_wire, len) == 0);
  return 0;
}
"#;
    let expected = Views {
        text: "h\u{e9}\0",
        bytes: &[0, 128, 255],
        fixed: [1, 2, 3],
        signed: vec![-1, i8::MIN],
        nonzero: vec![
            std::num::NonZeroU8::new(1).unwrap(),
            std::num::NonZeroU8::new(255).unwrap(),
        ],
    };
    let wire = postcard::to_vec::<_, 32>(&expected).unwrap();
    for flags in [vec!["-O0"], vec!["-O2", "-DNDEBUG"]] {
        let output = Command::new(compile(&temp.path().join("views"), driver, &flags))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, wire.as_slice());
    }
}

#[test]
fn generated_c_rejects_invalid_values_and_bytes() {
    let temp = tempfile::tempdir().unwrap();
    c::build_package(
        temp.path(),
        info("checks"),
        c::GenerationSettings::enable_all().max_sequence_length(2),
        generate_bindings!(Checks, Choice, Child),
    )
    .unwrap();
    let driver = r#"#include "bindings.h"
#include <stdio.h>
#define CHECK(x) do { if (!(x)) { fprintf(stderr, "failed at line %d: %s\n", __LINE__, #x); return 1; } } while (0)
int main(void) {
  uint8_t buffer[128] = {0};
  checks_checks value = {0};
  value.nonzero = 1;
  postcard_slice_t slice = postcard_slice_new(sizeof buffer, buffer);
  slice.data = nullptr;
  CHECK(checks_checks_encode(&slice, &value) == POSTCARD_ERROR_INVALID_INPUT);
  CHECK(checks_checks_decode(&slice, &value) == POSTCARD_ERROR_INVALID_INPUT);
  slice = postcard_slice_new(sizeof buffer, buffer);
  slice.len = slice.cap + 1;
  CHECK(checks_checks_encode(&slice, &value) == POSTCARD_ERROR_INVALID_INPUT);
  slice.len = 0;
  value.string.len = 3;
  CHECK(checks_checks_encode(&slice, &value) == POSTCARD_ERROR_INVALID_INPUT);
  value.string.len = 0;
  value.sequence.len = 3;
  CHECK(checks_checks_encode(&slice, &value) == POSTCARD_ERROR_INVALID_INPUT);
  value.sequence.len = 0;
  value.nonzero = 0;
  slice.len = 0;
  CHECK(checks_checks_encode(&slice, &value) == POSTCARD_ERROR_INVALID_INPUT);
  value.nonzero = 1;
  value.string.len = 1;
  value.string.data = "\xff";
  slice.len = 0;
  CHECK(checks_checks_encode(&slice, &value) == POSTCARD_ERROR_INVALID_INPUT);
  uint8_t truncated[] = {2, 'a'};
  slice = postcard_slice_new(sizeof truncated, truncated);
  CHECK(checks_checks_decode(&slice, &value) == POSTCARD_ERROR_INCOMPLETE_DATA);
  uint8_t invalid_utf8[] = {2, 0xc0, 0x80};
  slice = postcard_slice_new(sizeof invalid_utf8, invalid_utf8);
  CHECK(checks_checks_decode(&slice, &value) == POSTCARD_ERROR_INVALID_INPUT);
  uint8_t too_many[] = {0, 3, 1, 2, 3};
  slice = postcard_slice_new(sizeof too_many, too_many);
  CHECK(checks_checks_decode(&slice, &value) == POSTCARD_ERROR_BUFFER_TOO_SMALL);
  uint8_t zero[] = {0, 0, 0};
  slice = postcard_slice_new(sizeof zero, zero);
  CHECK(checks_checks_decode(&slice, &value) == POSTCARD_ERROR_INVALID_INPUT);
  uint8_t overflow[] = {0, 0, 1, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2};
  slice = postcard_slice_new(sizeof overflow, overflow);
  CHECK(checks_checks_decode(&slice, &value) == POSTCARD_ERROR_OVERFLOW);
  uint8_t bad_tag[] = {3};
  checks_choice choice = {0};
  slice = postcard_slice_new(sizeof bad_tag, bad_tag);
  CHECK(checks_choice_decode(&slice, &choice) == POSTCARD_ERROR_INVALID_INPUT);
  choice.tag = 3;
  slice = postcard_slice_new(sizeof buffer, buffer);
  CHECK(checks_choice_encode(&slice, &choice) == POSTCARD_ERROR_INVALID_INPUT);
  return 0;
}
"#;
    for flags in [vec!["-O0"], vec!["-O2", "-DNDEBUG"]] {
        assert!(
            Command::new(compile(&temp.path().join("checks"), driver, &flags))
                .status()
                .unwrap()
                .success()
        );
    }
}

#[test]
fn generation_settings_and_empty_types_compile() {
    #[derive(PostcardBindings)]
    #[allow(dead_code)]
    struct EmptyStrings([String; 0]);
    let temp = tempfile::tempdir().unwrap();
    for settings in [
        c::GenerationSettings::default(),
        c::GenerationSettings::enable_all().deserialization(false),
        c::GenerationSettings::default().deserialization(false),
        c::GenerationSettings::enable_all(),
    ] {
        c::build_package(
            temp.path(),
            info("unit"),
            settings,
            generate_bindings!(Unit, EmptyStrings, Checks),
        )
        .unwrap();
        let executable = compile(&temp.path().join("unit"), "#include \"bindings.h\"\nint main(void) { unit_unit value = {0}; (void)value; return 0; }\n", &[]);
        assert!(Command::new(executable).status().unwrap().success());
    }
    c::build_package(
        temp.path(),
        info("empty"),
        c::GenerationSettings::enable_all(),
        generate_bindings!(),
    )
    .unwrap();
    compile(
        &temp.path().join("empty"),
        "#include \"bindings.h\"\nint main(void) { return 0; }\n",
        &[],
    );
}

#[test]
fn unsupported_types_and_invalid_names_fail_before_writing() {
    #[derive(PostcardBindings)]
    #[allow(dead_code)]
    struct Wide(u128);
    let temp = tempfile::tempdir().unwrap();
    let error = c::build_package(
        temp.path(),
        info("wide"),
        c::GenerationSettings::enable_all(),
        generate_bindings!(Wide),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(!temp.path().join("wide").exists());
    let error = c::build_package(
        temp.path(),
        info("missing"),
        c::GenerationSettings::enable_all(),
        generate_bindings!(Packet),
    )
    .unwrap_err();
    assert!(error.to_string().contains("missing C binding"));
    assert!(!temp.path().join("missing").exists());
    for name in ["", ".", "..", "../outside", "/absolute", "a\\b"] {
        assert!(c::build_package(
            temp.path(),
            info(name),
            c::GenerationSettings::default(),
            generate_bindings!(Unit)
        )
        .is_err());
    }
    // Inline storage cannot represent self-referential object layouts.
    use postcard_bindgen::__private::{
        BindingsRegistry, GenBinding, ObjectMeta, StructType, ValueType,
    };
    struct Recursive;
    impl GenBinding for Recursive {
        fn get_type() -> ValueType {
            ValueType::Object(ObjectMeta {
                name: "Recursive",
                path: postcard_bindgen::__private::Path::new("c", "::"),
            })
        }
    }
    let mut registry = BindingsRegistry::default();
    let mut structure = StructType::new();
    structure.register_field::<Recursive>("next");
    registry.register_struct_binding("Recursive", "c", structure);
    let error = c::generate(
        registry.into_entries(),
        c::GenerationSettings::default(),
        "recursive",
    )
    .unwrap_err();
    assert!(error.to_string().contains("recursive types"));
}

mod outer {
    #[derive(serde::Serialize, postcard_bindgen::PostcardBindings)]
    #[allow(dead_code)]
    pub struct Item {
        pub field: u8,
    }
}

mod other {
    #[derive(serde::Serialize, postcard_bindgen::PostcardBindings)]
    #[allow(dead_code)]
    pub struct Item {
        pub field: u16,
    }
}

#[test]
fn module_names_and_c_identifiers() {
    #[derive(PostcardBindings)]
    #[allow(dead_code)]
    #[derive(Serialize)]
    struct Renamed {
        #[serde(rename = "type")]
        first: u8,
        #[serde(rename = "int")]
        second: u8,
        #[serde(rename = "1-name")]
        third: u8,
        #[serde(rename = "samplingRate")]
        fourth: u8,
        #[serde(rename = "HTTPStatus")]
        fifth: u8,
        #[serde(rename = "nullptr")]
        sixth: u8,
        #[serde(rename = "constexpr")]
        seventh: u8,
        nested: outer::Item,
        range: std::ops::Range<outer::Item>,
    }
    let temp = tempfile::tempdir().unwrap();
    c::build_package(
        temp.path(),
        info("names"),
        c::GenerationSettings::enable_all(),
        generate_bindings!(Renamed, outer::Item, other::Item),
    )
    .unwrap();
    compile(
        &temp.path().join("names"),
        r#"#include "bindings.h"
int main(void) {
  names_outer_item first = {0};
  names_other_item second = {0};
  names_renamed value = {0};
  value.type = 1;
  value.int_field = 2;
  value.field1_u2d_name = 3;
  value.sampling_rate = 4;
  value.http_status = 5;
  value.nullptr_field = 6;
  value.constexpr_field = 7;
  value.nested = first;
  value.range.start = first;
  (void)second; (void)value;
  return 0;
}
"#,
        &[],
    );
    c::build_package(
        temp.path(),
        info("flat"),
        c::GenerationSettings::enable_all().module_structure(false),
        generate_bindings!(Renamed, outer::Item),
    )
    .unwrap();
    compile(
        &temp.path().join("flat"),
        "#include \"bindings.h\"\nint main(void) { flat_item item = {0}; (void)item; return 0; }\n",
        &[],
    );
    let error = c::build_package(
        temp.path(),
        info("collision"),
        c::GenerationSettings::default().module_structure(false),
        generate_bindings!(outer::Item, other::Item),
    )
    .unwrap_err();
    assert!(error.to_string().contains("collision"));
    assert!(!temp.path().join("collision").exists());
    #[derive(PostcardBindings)]
    #[allow(dead_code)]
    #[derive(Serialize)]
    struct Collision {
        #[serde(rename = "Int")]
        first: u8,
        int_field: u8,
    }
    assert!(c::generate(
        generate_bindings!(Collision),
        c::GenerationSettings::default(),
        "test"
    )
    .is_err());
}

#[cfg(feature = "heapless")]
#[test]
fn heapless_bounds_override_defaults_and_zero_capacities_work() {
    #[derive(Serialize, PostcardBindings)]
    struct Bounded {
        text: heapless::String<3>,
        items: heapless::Vec<u16, 2>,
        map: heapless::LinearMap<u8, u16, 1>,
        bytes: heapless::Vec<u8, 3>,
        empty: heapless::Vec<u8, 0>,
        empty_text: heapless::String<0>,
    }
    let temp = tempfile::tempdir().unwrap();
    c::build_package(
        temp.path(),
        info("bounded"),
        c::GenerationSettings::enable_all()
            .max_sequence_length(0)
            .max_map_length(0),
        generate_bindings!(Bounded),
    )
    .unwrap();
    let driver = r#"#include "bindings.h"
#include <stdio.h>
int main(void) {
  bounded_bounded value = {0}, decoded = {0};
  static_assert(sizeof value.items.items / sizeof value.items.items[0] == 2, "heapless sequence capacity");
  static_assert(sizeof value.map.entries / sizeof value.map.entries[0] == 1, "heapless map capacity");
  value.text.len = 3;
  value.text.data = "abc";
  value.items.len = 2; value.items.items[0] = 1; value.items.items[1] = 300;
  value.map.len = 1; value.map.entries[0].key = 5; value.map.entries[0].value = 900;
  value.bytes.len = 3; value.bytes.data = (const uint8_t[]){1, 128, 255};
  uint8_t buffer[32];
  postcard_slice_t slice = postcard_slice_new(sizeof buffer, buffer);
  if (bounded_bounded_encode(&slice, &value) != POSTCARD_SUCCESS) return 1;
  size_t len = slice.len;
  slice = postcard_slice_new(len, buffer);
  if (bounded_bounded_decode(&slice, &decoded) != POSTCARD_SUCCESS) return 2;
  if (decoded.text.len != 3 || decoded.items.len != 2 || decoded.map.len != 1 || decoded.empty.len != 0) return 3;
  if (decoded.items.items[1] != 300 || decoded.map.entries[0].value != 900) return 4;
  if (decoded.text.data != (const char *)buffer + 1 || decoded.empty_text.len != 0) return 6;
  if (decoded.empty_text.data != (const char *)buffer + len) return 8;
  if (decoded.bytes.len != 3 || decoded.bytes.data[2] != 255) return 7;
  if (fwrite(buffer, 1, len, stdout) != len) return 5;
  /* Borrowed views also ignore Rust heapless capacities. */
  value.text = (postcard_string_view_t){.data = "four", .len = 4};
  value.bytes = (postcard_bytes_view_t){.data = (const uint8_t[]){1, 2, 3, 4}, .len = 4};
  value.empty = (postcard_bytes_view_t){.data = (const uint8_t[]){5}, .len = 1};
  value.empty_text = (postcard_string_view_t){.data = "x", .len = 1};
  uint8_t larger[64];
  slice = postcard_slice_new(sizeof larger, larger);
  if (bounded_bounded_encode(&slice, &value) != POSTCARD_SUCCESS) return 9;
  slice = postcard_slice_new(slice.len, larger);
  if (bounded_bounded_decode(&slice, &decoded) != POSTCARD_SUCCESS) return 10;
  if (decoded.text.len != 4 || decoded.bytes.len != 4 || decoded.empty.len != 1 || decoded.empty_text.len != 1) return 11;
  return 0;
}
"#;
    let output = Command::new(compile(&temp.path().join("bounded"), driver, &[]))
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut value = Bounded {
        text: heapless::String::try_from("abc").unwrap(),
        items: heapless::Vec::new(),
        map: heapless::LinearMap::new(),
        bytes: heapless::Vec::new(),
        empty: heapless::Vec::new(),
        empty_text: heapless::String::new(),
    };
    value.items.extend_from_slice(&[1, 300]).unwrap();
    value.map.insert(5, 900).unwrap();
    value.bytes.extend_from_slice(&[1, 128, 255]).unwrap();
    assert_eq!(
        output.stdout,
        postcard::to_vec::<_, 32>(&value).unwrap().as_slice()
    );
}
