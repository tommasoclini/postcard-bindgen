//! C23 bindings backed by the postcard-c runtime.
//!
//! Strings and byte sequences borrow the input buffer when decoded. Other
//! variable-length values use bounded, inline storage. Generation fails for
//! 128-bit integers, missing object definitions, recursive types, or C names
//! that collide after conversion.

use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    io,
};

use convert_case::{Boundary, Case, Converter};

use crate::{
    registry::{BindingType, Container, ContainerCollection, EnumVariantType, StructField},
    type_info::{NumberMeta, ValueType},
};

/// Controls serialization, deserialization, names, and value length bounds.
///
/// Defaults to deserialization, module-qualified names, and 64 entries per
/// unbounded inline sequence or map. Rust fixed-size arrays and inline heapless
/// collections retain their declared bounds. String and byte views have no
/// generated length limits.
#[derive(Debug, Clone)]
pub struct GenerationSettings {
    ser: bool,
    des: bool,
    module_structure: bool,
    max_sequence_length: usize,
    max_map_length: usize,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        Self {
            ser: false,
            des: true,
            module_structure: true,
            max_sequence_length: 64,
            max_map_length: 64,
        }
    }
}

impl GenerationSettings {
    /// Enables both serialization and deserialization.
    pub fn enable_all() -> Self {
        Self {
            ser: true,
            ..Self::default()
        }
    }

    pub fn serialization(mut self, enabled: bool) -> Self {
        self.ser = enabled;
        self
    }

    pub fn deserialization(mut self, enabled: bool) -> Self {
        self.des = enabled;
        self
    }

    /// Includes Rust module names in C type and function names.
    pub fn module_structure(mut self, enabled: bool) -> Self {
        self.module_structure = enabled;
        self
    }

    /// Maximum item count for inline sequences without a Rust capacity.
    /// Does not limit borrowed byte sequences.
    pub fn max_sequence_length(mut self, length: usize) -> Self {
        self.max_sequence_length = length;
        self
    }

    /// Maximum entry count for maps without a Rust capacity.
    pub fn max_map_length(mut self, length: usize) -> Self {
        self.max_map_length = length;
        self
    }
}

/// Generated declarations and implementation. Compile the source together with
/// `postcard.c`; include the header from any number of translation units.
#[derive(Debug)]
pub struct GeneratedBindings {
    pub header: String,
    pub source: String,
}

/// Generates C23 bindings. `package_name` namespaces all public symbols.
/// The generated implementation includes `bindings.h`. Function parameters use
/// `[static 1]`: callers must supply pointers to valid slice and value objects.
pub fn generate(
    containers: ContainerCollection,
    settings: impl Borrow<GenerationSettings>,
    package_name: &str,
) -> io::Result<GeneratedBindings> {
    let settings = settings.borrow();
    let prefix = identifier(package_name);
    let mut generator = Generator {
        settings,
        prefix: prefix.clone(),
        containers: BTreeMap::new(),
        names: BTreeMap::new(),
        symbols: BTreeSet::new(),
        visiting: BTreeSet::new(),
        visited: BTreeSet::new(),
        declarations: String::new(),
        definitions: Vec::new(),
        next_helper: 0,
    };
    generator.reserve(&format!("{prefix}_check_varint"))?;
    generator.reserve(&format!("{prefix}_valid_utf8"))?;
    for container in containers.all_containers() {
        let key = object_key(&container.path.to_string(), container.name);
        let parts = container
            .path
            .parts()
            .skip(1)
            .filter(|_| settings.module_structure)
            .filter(|p| !p.is_empty())
            .map(identifier)
            .chain(std::iter::once(identifier(container.name)))
            .collect::<Vec<_>>();
        let name = format!("{prefix}_{}", parts.join("_"));
        generator.reserve(&name)?;
        generator.names.insert(key.clone(), name);
        if generator
            .containers
            .insert(key.clone(), container)
            .is_some()
        {
            return Err(invalid(format!("duplicate C binding: {key}")));
        }
    }
    let keys = generator.containers.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        generator.container(&key)?;
    }
    let mut prototypes = String::new();
    let mut functions = String::new();
    let mut needs_varint = false;
    let mut needs_utf8 = false;
    for definition in &generator.definitions {
        for serialize in [true, false] {
            if !(if serialize {
                settings.ser
            } else {
                settings.des
            }) {
                continue;
            }
            let operation = if serialize { "encode" } else { "decode" };
            let qualifier = if serialize { "const " } else { "" };
            let signature = format!(
                "[[nodiscard]] postcard_error_t {}_{operation}(postcard_slice_t slice[static 1], {qualifier}{} value[static 1])",
                definition.name, definition.name
            );
            writeln!(prototypes, "{signature};").unwrap();
            let mut body = Body {
                text: String::new(),
                next_local: 0,
                serialize,
                prefix: &prefix,
                needs_varint: false,
                needs_utf8: false,
                indent: 1,
            };
            body.line("if (slice->data == nullptr || slice->len > slice->cap) return POSTCARD_ERROR_INVALID_INPUT;");
            body.line("(void)value;");
            body.line("[[maybe_unused]] postcard_error_t error = POSTCARD_SUCCESS;");
            match &definition.kind {
                DefinitionKind::Struct(fields) => {
                    if fields.is_empty() && !serialize {
                        body.line("value->unit = 0;");
                    }
                    body.fields(fields, "(*value)");
                }
                DefinitionKind::Enum(variants) => {
                    let tag = if serialize {
                        "value->tag".to_owned()
                    } else {
                        let tag = body.local("tag");
                        body.line(&format!("uint32_t {tag};"));
                        body.varint(32);
                        body.call(&format!("postcard_decode_variant(slice, &{tag})"));
                        tag
                    };
                    body.line(&format!("switch ({tag}) {{"));
                    for variant in variants {
                        body.line(&format!("case {}_{}: {{", definition.name, variant.name));
                        if serialize {
                            body.call(&format!(
                                "postcard_encode_variant(slice, {})",
                                variant.index
                            ));
                        }
                        body.fields(&variant.fields, &format!("value->data.{}", variant.name));
                        body.line("break;\n}");
                    }
                    body.line("default: return POSTCARD_ERROR_INVALID_INPUT;\n}");
                    if !serialize {
                        body.line(&format!("value->tag = ({}_tag){tag};", definition.name));
                    }
                }
            }
            body.line("return POSTCARD_SUCCESS;");
            needs_varint |= body.needs_varint;
            needs_utf8 |= body.needs_utf8;
            writeln!(functions, "{signature} {{\n{}}}\n", body.text).unwrap();
        }
    }
    let guard = format!("{prefix}_BINDINGS_H");
    let header = format!("/* Generated by postcard-bindgen. Strings are UTF-8 bytes, not NUL-terminated. */\n#ifndef {guard}\n#define {guard}\n\n#if !defined(__STDC_VERSION__) || __STDC_VERSION__ < 202311L\n#error \"postcard-bindgen bindings require C23 or later\"\n#endif\n\n#include \"postcard.h\"\n\n{}\n/* slice and value must point to valid objects; NULL violates the [static 1] contract.\n * Decoded strings and byte sequences borrow slice->data; keep that buffer alive\n * and unchanged while using them. Empty views must not be dereferenced.\n * Functions advance slice->len. On failure, input/output may be partially updated. */\n{prototypes}\n#endif /* {guard} */\n", generator.declarations);
    let mut source = "/* Generated by postcard-bindgen. */\n#include \"bindings.h\"\n\n".to_owned();
    if needs_varint {
        source.push_str(&VARINT_HELPER.replace("@PREFIX@", &prefix));
    }
    if needs_utf8 {
        source.push_str(&UTF8_HELPER.replace("@PREFIX@", &prefix));
    }
    source.push_str(&functions);
    Ok(GeneratedBindings { header, source })
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn object_key(path: &str, name: &str) -> String {
    format!("{path}::{name}")
}

// Normalize Rust/serde names before escaping punctuation and C keywords.
fn identifier(name: &str) -> String {
    let name = Converter::new()
        .set_boundaries(&[
            Boundary::LowerUpper,
            Boundary::DigitUpper,
            Boundary::Acronym,
        ])
        .to_case(Case::Snake)
        .convert(name);
    let mut result = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            result.push(c);
        } else {
            write!(result, "_u{:x}_", c as u32).unwrap();
        }
    }
    if result.is_empty() || result.starts_with(|c: char| c.is_ascii_digit() || c == '_') {
        result.insert_str(0, "field");
    }
    const KEYWORDS: &[&str] = &[
        "auto",
        "break",
        "case",
        "char",
        "const",
        "continue",
        "default",
        "do",
        "double",
        "else",
        "enum",
        "extern",
        "float",
        "for",
        "goto",
        "if",
        "inline",
        "int",
        "long",
        "register",
        "restrict",
        "return",
        "short",
        "signed",
        "sizeof",
        "static",
        "struct",
        "switch",
        "typedef",
        "union",
        "unsigned",
        "void",
        "volatile",
        "while",
        "bool",
        "true",
        "false",
        "alignas",
        "alignof",
        "constexpr",
        "nullptr",
        "static_assert",
        "thread_local",
        "typeof",
        "typeof_unqual",
    ];
    if KEYWORDS.contains(&result.as_str()) {
        result.push_str("_field");
    }
    result
}

#[derive(Clone)]
struct CType {
    name: String,
    kind: Kind,
}

#[derive(Clone)]
enum Kind {
    Number {
        suffix: String,
        nonzero: bool,
        bits: usize,
    },
    Bool,
    Object,
    String,
    Bytes,
    Array {
        inner: Box<CType>,
        length: Option<usize>,
        capacity: usize,
    },
    Optional(Box<CType>),
    Fields(Vec<Field>),
    Map {
        key: Box<CType>,
        value: Box<CType>,
        capacity: usize,
    },
}

#[derive(Clone)]
struct Field {
    name: String,
    ty: CType,
}

struct Variant {
    name: String,
    index: usize,
    fields: Vec<Field>,
}

struct Definition {
    name: String,
    kind: DefinitionKind,
}

enum DefinitionKind {
    Struct(Vec<Field>),
    Enum(Vec<Variant>),
}

struct Generator<'a> {
    settings: &'a GenerationSettings,
    prefix: String,
    containers: BTreeMap<String, Container>,
    names: BTreeMap<String, String>,
    symbols: BTreeSet<String>,
    visiting: BTreeSet<String>,
    visited: BTreeSet<String>,
    declarations: String,
    definitions: Vec<Definition>,
    next_helper: usize,
}

impl Generator<'_> {
    fn reserve(&mut self, name: &str) -> io::Result<()> {
        if !self.symbols.insert(name.to_owned()) {
            return Err(invalid(format!("C identifier collision: {name}")));
        }
        Ok(())
    }

    fn container(&mut self, key: &str) -> io::Result<()> {
        if self.visited.contains(key) {
            return Ok(());
        }
        if !self.visiting.insert(key.to_owned()) {
            return Err(invalid(format!(
                "recursive types cannot use inline C storage: {key}"
            )));
        }
        let container = self.containers.get(key).cloned().ok_or_else(|| {
            invalid(format!(
                "missing C binding for {key}; include it in generate_bindings!"
            ))
        })?;
        let name = self.names[key].clone();
        for operation in ["encode", "decode"] {
            self.reserve(&format!("{name}_{operation}"))?;
        }
        let kind = match container.r#type {
            BindingType::Struct(ty) => DefinitionKind::Struct(self.fields(&ty.fields)?),
            BindingType::TupleStruct(ty) => DefinitionKind::Struct(self.tuple(&ty.fields)?),
            BindingType::UnitStruct(_) => DefinitionKind::Struct(Vec::new()),
            BindingType::Enum(ty) => {
                if ty.variants.is_empty() {
                    return Err(invalid(format!("C cannot represent an empty enum: {key}")));
                }
                self.reserve(&format!("{name}_tag"))?;
                let mut variants = Vec::new();
                let mut members = BTreeSet::new();
                for variant in ty.variants {
                    if u32::try_from(variant.index).is_err() {
                        return Err(invalid("enum discriminant exceeds postcard u32 range"));
                    }
                    let member = identifier(variant.name);
                    if !members.insert(member.clone()) {
                        return Err(invalid(format!("C enum variant collision: {member}")));
                    }
                    self.reserve(&format!("{name}_{member}"))?;
                    self.reserve(&format!("{name}_new_{member}"))?;
                    let fields = match variant.inner_type {
                        EnumVariantType::Empty => Vec::new(),
                        EnumVariantType::Tuple(fields) => self.tuple(&fields)?,
                        EnumVariantType::NewType(fields) => self.fields(&fields)?,
                    };
                    variants.push(Variant {
                        name: member,
                        index: variant.index,
                        fields,
                    });
                }
                DefinitionKind::Enum(variants)
            }
        };
        match &kind {
            DefinitionKind::Struct(fields) => {
                self.structure(&name, fields);
            }
            DefinitionKind::Enum(variants) => {
                writeln!(self.declarations, "typedef enum : uint32_t {{").unwrap();
                for variant in variants {
                    writeln!(
                        self.declarations,
                        "  {name}_{} = {},",
                        variant.name, variant.index
                    )
                    .unwrap();
                }
                writeln!(
                    self.declarations,
                    "}} {name}_tag;\n\ntypedef struct {{\n  {name}_tag tag;"
                )
                .unwrap();
                if variants.iter().any(|v| !v.fields.is_empty()) {
                    self.declarations.push_str("  union {\n");
                    for variant in variants.iter().filter(|v| !v.fields.is_empty()) {
                        self.declarations.push_str("    struct {\n");
                        for field in &variant.fields {
                            writeln!(self.declarations, "      {} {};", field.ty.name, field.name)
                                .unwrap();
                        }
                        writeln!(self.declarations, "    }} {};", variant.name).unwrap();
                    }
                    self.declarations.push_str("  } data;\n");
                }
                writeln!(self.declarations, "}} {name};\n").unwrap();
                for variant in variants {
                    let member = &variant.name;
                    if variant.fields.is_empty() {
                        writeln!(
                            self.declarations,
                            "#define {name}_new_{member}() \\\n  (({name}) {{ .tag = {name}_{member} }})"
                        )
                        .unwrap();
                    } else {
                        writeln!(
                            self.declarations,
                            "#define {name}_new_{member}(...) \\\n  (({name}) {{ .tag = {name}_{member}, .data.{member} = {{ __VA_ARGS__ }} }})"
                        )
                        .unwrap();
                    }
                }
                self.declarations.push('\n');
            }
        }
        self.definitions.push(Definition { name, kind });
        self.visiting.remove(key);
        self.visited.insert(key.to_owned());
        Ok(())
    }

    fn fields(&mut self, fields: &[StructField]) -> io::Result<Vec<Field>> {
        let mut names = BTreeSet::new();
        fields
            .iter()
            .map(|field| {
                let name = identifier(field.name);
                if !names.insert(name.clone()) {
                    return Err(invalid(format!("C field identifier collision: {name}")));
                }
                Ok(Field {
                    name,
                    ty: self.value_type(&field.v_type)?,
                })
            })
            .collect()
    }

    fn tuple(&mut self, fields: &[ValueType]) -> io::Result<Vec<Field>> {
        fields
            .iter()
            .enumerate()
            .map(|(i, ty)| {
                Ok(Field {
                    name: format!("field_{i}"),
                    ty: self.value_type(ty)?,
                })
            })
            .collect()
    }

    fn structure(&mut self, name: &str, fields: &[Field]) {
        self.declarations.push_str("typedef struct {\n");
        if fields.is_empty() {
            self.declarations
                .push_str("  uint8_t unit; /* No wire bytes. */\n");
        }
        for field in fields {
            writeln!(self.declarations, "  {} {};", field.ty.name, field.name).unwrap();
        }
        writeln!(self.declarations, "}} {name};\n").unwrap();
    }

    fn value_type(&mut self, ty: &ValueType) -> io::Result<CType> {
        let kind = match ty {
            ValueType::Number(meta) => {
                let (name, suffix, nonzero, bits) = match meta {
                    NumberMeta::Integer { bytes, signed, zero_able } if [1, 2, 4, 8].contains(bytes) => {
                        let bits = bytes * 8;
                        (format!("{}int{bits}_t", if *signed { "" } else { "u" }),
                         format!("{}{bits}", if *signed { "i" } else { "u" }), !zero_able, bits)
                    }
                    NumberMeta::FloatingPoint { bytes: 4 } => ("float".to_owned(), "f32".to_owned(), false, 0),
                    NumberMeta::FloatingPoint { bytes: 8 } => ("double".to_owned(), "f64".to_owned(), false, 0),
                    _ => return Err(invalid("postcard-c supports integers through 64 bits and f32/f64; 128-bit integers are unsupported")),
                };
                return Ok(CType {
                    name,
                    kind: Kind::Number {
                        suffix,
                        nonzero,
                        bits,
                    },
                });
            }
            ValueType::Bool(_) => {
                return Ok(CType {
                    name: "bool".to_owned(),
                    kind: Kind::Bool,
                })
            }
            ValueType::Object(meta) => {
                let key = object_key(&meta.path.to_string(), meta.name);
                self.container(&key)?;
                return Ok(CType {
                    name: self.names[&key].clone(),
                    kind: Kind::Object,
                });
            }
            ValueType::String(_) => {
                return Ok(CType {
                    name: "postcard_string_view_t".to_owned(),
                    kind: Kind::String,
                });
            }
            ValueType::Array(meta)
                if meta.length.is_none()
                    && matches!(
                        meta.items_type.as_ref(),
                        ValueType::Number(NumberMeta::Integer {
                            bytes: 1,
                            signed: false,
                            zero_able: true,
                        })
                    ) =>
            {
                return Ok(CType {
                    name: "postcard_bytes_view_t".to_owned(),
                    kind: Kind::Bytes,
                });
            }
            ValueType::Array(meta) => Kind::Array {
                inner: Box::new(self.value_type(&meta.items_type)?),
                length: meta.length,
                capacity: meta
                    .length
                    .or(meta.max_length)
                    .unwrap_or(self.settings.max_sequence_length),
            },
            ValueType::Optional(meta) => Kind::Optional(Box::new(self.value_type(&meta.inner)?)),
            ValueType::Tuple(meta) => Kind::Fields(self.tuple(&meta.items_types)?),
            ValueType::Range(meta) => {
                let ty = self.value_type(&meta.bounds_type)?;
                Kind::Fields(vec![
                    Field {
                        name: "start".into(),
                        ty: ty.clone(),
                    },
                    Field {
                        name: "end".into(),
                        ty,
                    },
                ])
            }
            ValueType::Map(meta) => Kind::Map {
                key: Box::new(self.value_type(&meta.key_type)?),
                value: Box::new(self.value_type(&meta.value_type)?),
                capacity: meta.max_length.unwrap_or(self.settings.max_map_length),
            },
        };
        let name = format!("{}_value_{}", self.prefix, self.next_helper);
        self.next_helper += 1;
        self.reserve(&name)?;
        match &kind {
            Kind::Array {
                inner,
                length,
                capacity,
            } => {
                self.declarations.push_str("typedef struct {\n");
                if length.is_none() {
                    self.declarations.push_str("  size_t len;\n");
                }
                writeln!(
                    self.declarations,
                    "  {} items[{}];\n}} {name}; /* Capacity: {capacity} items. */\n",
                    inner.name,
                    capacity.max(&1)
                )
                .unwrap();
            }
            Kind::Optional(inner) => {
                writeln!(
                    self.declarations,
                    "typedef struct {{\n  bool is_some;\n  {} value;\n}} {name};\n",
                    inner.name
                )
                .unwrap();
            }
            Kind::Fields(fields) => self.structure(&name, fields),
            Kind::Map {
                key,
                value,
                capacity,
            } => {
                writeln!(self.declarations, "typedef struct {{\n  size_t len;\n  struct {{ {} key; {} value; }} entries[{}];\n}} {name}; /* Capacity: {capacity} entries. */\n", key.name, value.name, capacity.max(&1)).unwrap();
            }
            _ => unreachable!(),
        }
        Ok(CType { name, kind })
    }
}

struct Body<'a> {
    text: String,
    next_local: usize,
    serialize: bool,
    prefix: &'a str,
    needs_varint: bool,
    needs_utf8: bool,
    indent: usize,
}

impl Body<'_> {
    fn line(&mut self, line: &str) {
        for line in line.lines() {
            let line = line.trim();
            if line.starts_with('}') {
                self.indent -= 1;
            }
            writeln!(self.text, "{}{line}", "  ".repeat(self.indent)).unwrap();
            if line.ends_with('{') {
                self.indent += 1;
            }
        }
    }

    fn local(&mut self, kind: &str) -> String {
        let name = format!("{kind}_{}", self.next_local);
        self.next_local += 1;
        name
    }

    fn call(&mut self, call: &str) {
        self.line(&format!(
            "error = {call};\n  if (error != POSTCARD_SUCCESS) return error;"
        ));
    }

    fn varint(&mut self, bits: usize) {
        self.needs_varint = true;
        self.call(&format!("{}_check_varint(slice, {bits})", self.prefix));
    }

    fn fields(&mut self, fields: &[Field], parent: &str) {
        for field in fields {
            self.value(&field.ty, &format!("({parent}).{}", field.name));
        }
    }

    fn length(&mut self, expression: &str, capacity: usize, map: bool) {
        let suffix = if map { "map" } else { "seq" };
        if self.serialize {
            self.line(&format!(
                "if ({expression} > {capacity}) return POSTCARD_ERROR_INVALID_INPUT;"
            ));
            self.call(&format!("postcard_start_{suffix}(slice, {expression})"));
        } else {
            self.varint(64);
            self.call(&format!(
                "postcard_decode_{suffix}_len(slice, &({expression}))"
            ));
            self.line(&format!(
                "if ({expression} > {capacity}) return POSTCARD_ERROR_BUFFER_TOO_SMALL;"
            ));
        }
    }

    fn value(&mut self, ty: &CType, expression: &str) {
        let expr = format!("({expression})");
        match &ty.kind {
            Kind::Number {
                suffix,
                nonzero,
                bits,
            } => {
                if !self.serialize && *bits > 8 {
                    self.varint(*bits);
                }
                if self.serialize && *nonzero {
                    self.line(&format!(
                        "if ({expr} == 0) return POSTCARD_ERROR_INVALID_INPUT;"
                    ));
                }
                let operation = if self.serialize { "encode" } else { "decode" };
                let argument = if self.serialize {
                    expr.clone()
                } else {
                    format!("&{expr}")
                };
                self.call(&format!("postcard_{operation}_{suffix}(slice, {argument})"));
                if !self.serialize && *nonzero {
                    self.line(&format!(
                        "if ({expr} == 0) return POSTCARD_ERROR_INVALID_INPUT;"
                    ));
                }
            }
            Kind::Bool => {
                let (operation, argument) = if self.serialize {
                    ("encode", expr)
                } else {
                    ("decode", format!("&{expr}"))
                };
                self.call(&format!("postcard_{operation}_bool(slice, {argument})"));
            }
            Kind::Object => {
                let operation = if self.serialize { "encode" } else { "decode" };
                self.call(&format!("{}_{operation}(slice, &{expr})", ty.name));
            }
            Kind::String => {
                self.needs_utf8 = true;
                if self.serialize {
                    self.line(&format!("if ({expr}.len != 0 && {expr}.data == nullptr) return POSTCARD_ERROR_INVALID_INPUT;"));
                    self.line(&format!("if (!{}_valid_utf8({expr}.data, {expr}.len)) return POSTCARD_ERROR_INVALID_INPUT;", self.prefix));
                    self.call(&format!(
                        "postcard_encode_string(slice, {expr}.len, {expr}.len ? {expr}.data : \"\")"
                    ));
                } else {
                    self.call(&format!("postcard_decode_string_view(slice, &{expr})"));
                    self.line(&format!("if (!{}_valid_utf8({expr}.data, {expr}.len)) return POSTCARD_ERROR_INVALID_INPUT;", self.prefix));
                }
            }
            Kind::Bytes => {
                if self.serialize {
                    self.line(&format!("if ({expr}.len != 0 && {expr}.data == nullptr) return POSTCARD_ERROR_INVALID_INPUT;"));
                    self.call(&format!(
                        "postcard_encode_byte_array(slice, {expr}.len, {expr}.len ? {expr}.data : (const uint8_t *)\"\")"
                    ));
                } else {
                    self.call(&format!("postcard_decode_byte_array_view(slice, &{expr})"));
                }
            }
            Kind::Optional(inner) => {
                if self.serialize {
                    self.call(&format!("{expr}.is_some ? postcard_encode_option_some(slice) : postcard_encode_option_none(slice)"));
                } else {
                    self.call(&format!(
                        "postcard_decode_option_tag(slice, &{expr}.is_some)"
                    ));
                }
                self.line(&format!("if ({expr}.is_some) {{"));
                self.value(inner, &format!("{expr}.value"));
                self.line("}");
            }
            Kind::Array {
                inner,
                length,
                capacity,
            } => {
                let count = if let Some(length) = length {
                    if *length == 0 {
                        return;
                    }
                    length.to_string()
                } else {
                    self.length(&format!("{expr}.len"), *capacity, false);
                    format!("{expr}.len")
                };
                let index = self.local("index");
                self.line(&format!(
                    "for (size_t {index} = 0; {index} < {count}; ++{index}) {{"
                ));
                self.value(inner, &format!("{expr}.items[{index}]"));
                self.line("}");
            }
            Kind::Fields(fields) => self.fields(fields, &expr),
            Kind::Map {
                key,
                value,
                capacity,
            } => {
                self.length(&format!("{expr}.len"), *capacity, true);
                let index = self.local("index");
                self.line(&format!(
                    "for (size_t {index} = 0; {index} < {expr}.len; ++{index}) {{"
                ));
                self.value(key, &format!("{expr}.entries[{index}].key"));
                self.value(value, &format!("{expr}.entries[{index}].value"));
                self.line("}");
            }
        }
    }
}

// Validate widths before delegating to postcard-c (including the high bits of
// the tenth byte for u64). Does not consume input or require canonical varints.
const VARINT_HELPER: &str = r#"[[nodiscard]] static inline postcard_error_t @PREFIX@_check_varint(const postcard_slice_t slice[static 1], unsigned bits) {
  size_t remaining = slice->cap - slice->len;
  unsigned count = (bits + 6) / 7;
  for (unsigned i = 0; i < count; ++i) {
    if (i >= remaining) return POSTCARD_ERROR_INCOMPLETE_DATA;
    uint8_t byte = slice->data[slice->len + i];
    if (i == count - 1 && (byte >> (bits - 7 * i)) != 0) return POSTCARD_ERROR_OVERFLOW;
    if (!(byte & 0x80)) return POSTCARD_SUCCESS;
  }
  return POSTCARD_ERROR_OVERFLOW;
}

"#;

const UTF8_HELPER: &str = r#"[[nodiscard]] static inline bool @PREFIX@_valid_utf8(const char *data, size_t len) {
  size_t i = 0;
  while (i < len) {
    uint8_t first = (uint8_t)data[i++];
    if (first < 0x80) continue;
    unsigned count;
    uint32_t codepoint, minimum;
    if (first >= 0xc2 && first <= 0xdf) { count = 1; codepoint = first & 0x1f; minimum = 0x80; }
    else if (first >= 0xe0 && first <= 0xef) { count = 2; codepoint = first & 0x0f; minimum = 0x800; }
    else if (first >= 0xf0 && first <= 0xf4) { count = 3; codepoint = first & 7; minimum = 0x10000; }
    else return false;
    if (count > len - i) return false;
    while (count--) {
      uint8_t next = (uint8_t)data[i++];
      if ((next & 0xc0) != 0x80) return false;
      codepoint = (codepoint << 6) | (next & 0x3f);
    }
    if (codepoint < minimum || codepoint > 0x10ffff || (codepoint >= 0xd800 && codepoint <= 0xdfff)) return false;
  }
  return true;
}

"#;
