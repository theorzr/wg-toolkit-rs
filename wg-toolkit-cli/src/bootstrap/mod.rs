use std::io::{self, Write, BufWriter};
use std::collections::HashSet;
use std::fs::{self, File};
use std::cmp::Ordering;
use std::borrow::Cow;
use std::path::Path;

use wgtk::res::ResFilesystem;
use wgtk::pxml::{self, Value};

use crate::{BootstrapArgs, CliResult};

mod parse;
mod model;

use model::{Component, Entity, Interface, Method, Model, PropertyFlags, Ty, TyKind, VariableHeaderSize};

// NOTE: For the future, if python bytecode interpretation is needed to automatically
// generate enumeration or try to gather function arguments' names, see:
// https://github.com/python/cpython/blob/main/InternalDocs/interpreter.md


/// Entrypoint.
pub fn cmd_bootstrap(args: BootstrapArgs) -> CliResult<()> {

    let version = {

        let version_file = fs::read_to_string(args.dir.join("version.xml"))
            .map_err(|e| format!("Failed to read version.xml, reason: {e}"))?;

        version_file
            .split_once("<version>").ok_or_else(|| format!("Missing <version> from version.xml"))?.1
            .split_once("</version>").ok_or_else(|| format!("Missing </version> from version.xml"))?.0
            .trim()
            .to_string()
        
    };

    let fs = ResFilesystem::new(args.dir.join("res"))
        .map_err(|e| format!("Failed to open resource filesystem, reason: {e}"))?;
    
    let model = load(fs, version)
        .map_err(|e| format!("Failed to load model, reason: {e}"))?;
    
    let mut state = State::new();
    generate(&args.dest, &model, &mut state, )
        .map_err(|e| format!("Failed to generate model, reason: {e}"))?;

    Ok(())

}

fn load(fs: ResFilesystem, version: String) -> io::Result<Model> {

    let mut model = Model {
        version,
        ..Default::default()
    };

    println!("== Reading aliases...");
    let alias_reader = fs.read("scripts/entity_defs/alias.xml")?;
    let alias_elt = pxml::from_reader(alias_reader).unwrap();
    parse::parse_aliases(&alias_elt, &mut model.tys);

    println!("== Reading interfaces...");
    for interface_file in fs.read_dir("scripts/entity_defs/interfaces")? {
        
        let interface_file = interface_file?;
        let Some((interface_name, "")) = interface_file.name().split_once(".def") else {
            continue;
        };

        println!(" = {interface_name}");

        let interface_reader = fs.read(interface_file.path())?;
        let interface_elt = pxml::from_reader(interface_reader).unwrap();
        let interface = parse::parse_interface(&interface_elt, &mut model.tys, interface_name.to_string());
        model.interfaces.push(interface);

    }

    println!("== Reading entities...");
    let entities_reader = fs.read("scripts/entities.xml")?;
    let entities_elt = pxml::from_reader(entities_reader).unwrap();
    let entities_elt = entities_elt.get_child("ClientServerEntities").unwrap().as_element().unwrap();
    for (index, (entity_name, _)) in entities_elt.iter_children_all().enumerate() {
        
        println!(" = {entity_name}");
        let entity_reader = fs.read(format!("scripts/entity_defs/{entity_name}.def"))?;
        let entity_elt = pxml::from_reader(entity_reader).unwrap();
        let entity = parse::parse_entity(&entity_elt, &mut model.tys, index + 1, entity_name.to_string());
        model.entities.push(entity);

    }

    println!("== Reading extension components...");
    // WoT extensions (feature packages such as "la_pinger" or "battle_royale") each sit
    // at the root of the resource filesystem and, if active, carry an "extension.xml"
    // declaring a set of "static" components. Each static component is a def file (same
    // shape as an interface) under "<ext>/scripts/component_defs/" that also declares
    // which entity/entities it folds its methods/properties into via "<ofEntity>". This
    // is a WG-specific build step (no trace of it in vanilla BigWorld's entity_description
    // parsing), so its exact folding rule isn't authoritatively documented -- the order
    // used here (extensions in alphabetical directory order, static components in
    // declaration order within each extension) was empirically confirmed against a live
    // capture: la_pinger's "LaPingerComponent.pingMeAndThenJustTouchMe" lands exactly on
    // Account's exposed client method id 0x2B, right after battle_royale's
    // "AccountBattleRoyaleTournamentComponent" 2 client methods (0x29, 0x2A) and Account's
    // own last interface-derived method (0x28) -- see re-work/HANGAR_LOADING.md.
    let mut ext_names: Vec<String> = fs.read_dir("")?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.stat().is_dir())
        .map(|entry| entry.name().to_string())
        .collect();
    ext_names.sort();

    for ext_name in ext_names {

        let Ok(ext_reader) = fs.read(format!("{ext_name}/extension.xml")) else {
            continue;
        };

        let ext_elt = pxml::from_reader(ext_reader).unwrap();

        // Only "StaticComponents" fold into every instance's static method table.
        // "DynamicComponents" are attached to specific entity instances at runtime
        // (e.g. only for the duration of a particular battle mode) and don't claim a
        // fixed exposed id the way static ones do, so they're intentionally skipped.
        let Some(Value::Element(components_elt)) = ext_elt.get_child("Components") else {
            continue;
        };

        let Some(Value::Element(static_elt)) = components_elt.get_child("StaticComponents") else {
            continue;
        };

        for (component_name, _) in static_elt.iter_children_all() {

            println!(" = {ext_name}/{component_name}");

            let component_path = format!("{ext_name}/scripts/component_defs/{component_name}.def");
            let component_reader = fs.read(&component_path)?;
            let component_elt = pxml::from_reader(component_reader).unwrap();

            let of_entities = parse::parse_of_entity(&component_elt);
            let interface = parse::parse_interface(&component_elt, &mut model.tys, component_name.clone());

            model.components.push(Component {
                name: component_name.clone(),
                of_entities,
                interface,
            });

        }

    }

    println!("== Types: {}", model.tys.count());

    Ok(model)

}

fn generate(dest_dir: &Path, model: &Model, state: &mut State) -> io::Result<()> {
    generate_mod(dest_dir, model, state)
}

fn generate_mod(mod_dir: &Path, model: &Model, state: &mut State) -> io::Result<()> {

    let _ = fs::remove_dir_all(&mod_dir);
    fs::create_dir_all(&mod_dir)?;

    println!("== Writing module...");
    let mod_file = mod_dir.join("mod.rs");
    let mut writer = BufWriter::new(File::create(&mod_file)?);
    writeln!(writer, "#![allow(non_camel_case_types, non_snake_case, unused)]")?;
    writeln!(writer)?;
    writeln!(writer, "//! This module is generated by bootstrap command of the CLI.")?;
    writeln!(writer, "//! Targeted game version: {}", model.version)?;
    writeln!(writer)?;
    writeln!(writer, "pub mod alias;")?;
    writeln!(writer)?;
    generate_alias(mod_dir, model)?;

    writeln!(writer, "pub mod interface;")?;
    writeln!(writer, "pub mod entity;")?;
    generate_interfaces(mod_dir, model, &mut *state)?;
    generate_entities(mod_dir, model, &mut *state)?;

    Ok(())

}

fn generate_alias(mod_dir: &Path, model: &Model) -> io::Result<()> {

    println!("== Writing aliases...");
    let alias_file = mod_dir.join("alias.rs");
    let mut writer = BufWriter::new(File::create(&alias_file)?);

    writeln!(writer, "pub use wgtk::net::codec::{{AutoString, Python, Mailbox}};")?;
    writeln!(writer, "pub use glam::{{Vec2, Vec3, Vec4}};")?;
    writeln!(writer)?;

    let mut prev_dict = false;

    for ty in model.tys.iter() {

        let identifier = generate_rust_identifier(ty.name());

        match ty.kind() {
            TyKind::Alias(alias_ty) => {
                if prev_dict {
                    writeln!(writer)?;
                    prev_dict = false;
                }
                writeln!(writer, "pub type {identifier} = {};", generate_type_ref(alias_ty))?;
            }
            TyKind::Dict(ty_dict) => {
                prev_dict = true;
                writeln!(writer)?;
                writeln!(writer, "wgtk::__struct_simple_codec! {{")?;
                writeln!(writer, "    #[derive(Debug)]")?;
                writeln!(writer, "    pub struct {identifier} {{")?;
                for prop in &ty_dict.properties {
                    let prop_identifier = generate_rust_identifier(&prop.name);
                    writeln!(writer, "        pub {prop_identifier}: {},", generate_type_ref(&prop.ty))?;
                }
                writeln!(writer, "    }}")?;
                writeln!(writer, "}}")?;
            }
            TyKind::Array(_) |
            TyKind::Tuple(_) => {
                // Arays and tuples are inlined when generating type ref, so we don't 
                // actually define them.
            }
            _ => {}  // Don't define builtins.
        }

    }

    Ok(())

}

/// From the given type, return a string referencing it, when possible it inlines it.
fn generate_type_ref(ty: &Ty) -> Cow<'_, str> {
    Cow::Borrowed(match ty.kind() {
        TyKind::Int8 => "i8",
        TyKind::Int16 => "i16",
        TyKind::Int32 => "i32",
        TyKind::Int64 => "i64",
        TyKind::UInt8 => "u8",
        TyKind::UInt16 => "u16",
        TyKind::UInt32 => "u32",
        TyKind::UInt64 => "u64",
        TyKind::Float32 => "f32",
        TyKind::Float64 => "f64",
        TyKind::Vector2 => "Vec2",
        TyKind::Vector3 => "Vec3",
        TyKind::Vector4 => "Vec4",
        TyKind::String => "AutoString",
        TyKind::Python => "Python",
        TyKind::Mailbox => "Mailbox",
        TyKind::Array(ty_seq) |
        TyKind::Tuple(ty_seq) => {
            
            let inline = if let Some(size) = ty_seq.size {
                format!("Box<[{}; {size}]>", generate_type_ref(&ty_seq.ty))
            } else {
                format!("Vec<{}>", generate_type_ref(&ty_seq.ty))
            };

            return Cow::Owned(inline);

        }
        _ => ty.name()
    })
}

/// Deterministically generate a Rust-compatible identifier for types or fields.
fn generate_rust_identifier(name: &str) -> Cow<'_, str> {
    match name {
        "type" => Cow::Borrowed("r#type"),
        name => Cow::Borrowed(name),
    }
}

fn generate_interfaces(mod_dir: &Path, model: &Model, state: &mut State) -> io::Result<()> {

    println!("== Writing interfaces...");
    let interface_file = mod_dir.join("interface.rs");
    let mut writer = BufWriter::new(File::create(&interface_file)?);

    writeln!(writer, "use super::alias::*;")?;
    writeln!(writer)?;

    for interface in &model.interfaces {
        generate_interface(&mut writer, model, interface, &mut *state)?;
    }

    // Extension components are folded into entities' method tables (see
    // `generate_entity_methods`), but -- unlike regular interfaces -- they're never
    // `pub i_xxx: ComponentName` composed into an entity's property struct, so we only
    // need their method argument structs here, not a full `generate_interface` pass.
    for component in &model.components {
        for app_state in &mut state.apps {
            generate_interface_methods(&mut writer, model, &component.interface, app_state)?;
        }
    }

    Ok(())

}

fn generate_entities(mod_dir: &Path, model: &Model, state: &mut State) -> io::Result<()> {

    println!("== Writing entities...");
    let entity_file = mod_dir.join("entity.rs");
    let mut writer = BufWriter::new(File::create(&entity_file)?);

    writeln!(writer, "use wgtk::net::app::common::entity::{{Entity, SimpleEntity}};")?;
    writeln!(writer)?;
    writeln!(writer, "use super::alias::*;")?;
    writeln!(writer, "use super::interface::*;")?;
    writeln!(writer)?;

    for entity in &model.entities {
        generate_entity(&mut writer, model, entity, &mut *state)?;
    }

    writeln!(writer, "// ============================================== //")?;
    writeln!(writer, "// ====== {:^32} ====== //", "[COLLECTION]")?;
    writeln!(writer, "// ============================================== //")?;
    writeln!(writer)?;

    writeln!(writer, "/// This internal trait can be used to visit each entity type in order.")?;
    writeln!(writer, "pub trait EntityTypeCollection {{")?;
    writeln!(writer, "    /// Create a new instance of this collection that is expected to have the given")?;
    writeln!(writer, "    /// number of entity types.")?;
    writeln!(writer, "    fn new(len: usize) -> Self;")?;
    writeln!(writer, "    /// Visit the given entity type.")?;
    writeln!(writer, "    fn add<E: Entity>(&mut self)")?;
    writeln!(writer, "    where")?;
    writeln!(writer, "        E: std::fmt::Debug,")?;
    writeln!(writer, "        E::ClientMethod: std::fmt::Debug,")?;
    writeln!(writer, "        E::BaseMethod: std::fmt::Debug,")?;
    writeln!(writer, "        E::CellMethod: std::fmt::Debug;")?;
    writeln!(writer, "}}")?;
    writeln!(writer)?;

    writeln!(writer, "/// Visit all entity types in order.")?;
    writeln!(writer, "pub fn collect_entity_types<C: EntityTypeCollection>() -> C {{")?;
    writeln!(writer, "    let mut c = C::new({});", model.entities.len())?;
    for entity in &model.entities {
        writeln!(writer, "    c.add::<{}>();", entity.interface.name)?;
    }
    writeln!(writer, "    c")?;
    writeln!(writer, "}}")?;

    // writeln!(writer, "wgtk::__bootstrap_enum_entities! {{")?;
    // writeln!(writer, "    /// Generic entity type enumeration allowing decoding of any entities.")?;
    // writeln!(writer, "    #[derive(Debug)]")?;
    // writeln!(writer, "    pub enum Generic: Generic_Client, Generic_Base, Generic_Cell {{")?;
    // for entity in &model.entities {
    //     writeln!(writer, "        {} = 0x{:02X},", entity.interface.name, entity.id)?;
    // }
    // writeln!(writer, "    }}")?;
    // writeln!(writer, "}}")?;
    // writeln!(writer)?;

    Ok(())

}

fn generate_entity(
    mut writer: impl Write, 
    model: &Model, 
    entity: &Entity,
    state: &mut State,
) -> io::Result<()> {

    generate_interface(&mut writer, model, &entity.interface, state)?;
    
    for app_state in &mut state.apps {
        generate_entity_methods(&mut writer, model, entity, app_state)?;
    }
    
    writeln!(writer, "impl {} {{", entity.interface.name)?;
    writeln!(writer, "    const TYPE_ID: u16 = 0x{:02X};", entity.id)?;
    writeln!(writer, "}}")?;
    writeln!(writer)?;

    writeln!(writer, "impl SimpleEntity for {} {{", entity.interface.name)?;
    writeln!(writer, "    type ClientMethod = {}_Client;", entity.interface.name)?;
    writeln!(writer, "    type BaseMethod = {}_Base;", entity.interface.name)?;
    writeln!(writer, "    type CellMethod = {}_Cell;", entity.interface.name)?;
    writeln!(writer, "}}")?;
    writeln!(writer)?;

    Ok(())

}

fn generate_entity_methods(
    mut writer: impl Write,
    model: &Model, 
    entity: &Entity,
    app_state: &mut AppState,
)  -> io::Result<()> {

    /// An exposed method for the network protocol, this is used to list all exposed 
    /// methods on an entity and then compute the methods' exposed ids by sorting them.
    #[derive(Debug)]
    struct ExposedMethod<'a> {
        interface: &'a Interface,
        method: &'a Method,
        stream_size: StreamSize,
    }

    /// This method recursively register all methods for the entity in order to sort them
    /// later depending on their arguments' size and then compute there exposed id for
    /// the network protocol.
    /// 
    /// IMPORTANT: The initial order of the exposed method is really important because we
    /// will use a stable sort, and some orders should not be changed.
    fn add_internal_methods<'m>(
        exposed_methods: &mut Vec<ExposedMethod<'m>>, 
        model: &'m Model, 
        interface: &'m Interface,
        app_state: &mut AppState,
    ) {

        for interface_name in &interface.implements {

            let interface = model.interfaces.iter()
                .find(|i| &i.name == interface_name)
                .expect("unknown implemented interface");

            add_internal_methods(exposed_methods, model, interface, &mut *app_state);

        }
        
        for method in (app_state.interface_methods)(interface) {
            if is_method_exposed(method) {
                exposed_methods.push(ExposedMethod {
                    interface,
                    method,
                    stream_size: compute_method_stream_size(method),
                });
            }
        }

    }

    let mut methods = Vec::new();
    add_internal_methods(&mut methods, model, &entity.interface, &mut *app_state);

    // We want to sort fixed methods first and variable last, and then sort between
    // their configured fixed or variable size.
    methods.sort_by(|a, b| {
        match (a.stream_size, b.stream_size) {
            (StreamSize::Variable(a_size), StreamSize::Variable(b_size)) =>
                a_size.cmp(&b_size),
            (StreamSize::Fixed(a_size), StreamSize::Fixed(b_size)) =>
                a_size.cmp(&b_size),
            (StreamSize::Fixed(_), StreamSize::Variable(_)) =>
                Ordering::Less,
            (StreamSize::Variable(_), StreamSize::Fixed(_)) =>
                Ordering::Greater,
        }
    });

    // Extension "static" components fold their methods in *after* the entity's own
    // interface-derived, size-sorted table -- confirmed live (see the Component doc
    // comment and re-work/HANGAR_LOADING.md): unlike the sort above, these are NOT
    // resorted by stream size, they simply keep claiming the next exposed ids in
    // component order (then declared-method order within a component). This is why
    // adding/removing an unrelated extension never shifts any other exposed id.
    for component in &model.components {

        if !component.of_entities.iter().any(|e| e == &entity.interface.name) {
            continue;
        }

        for method in (app_state.interface_methods)(&component.interface) {
            if is_method_exposed(method) {
                methods.push(ExposedMethod {
                    interface: &component.interface,
                    method,
                    stream_size: compute_method_stream_size(method),
                });
            }
        }

    }

    writeln!(writer, "wgtk::__enum_entity_methods! {{  // Entity methods on {}", app_state.name)?;
    writeln!(writer, "    pub enum {}_{} {{", 
        entity.interface.name, app_state.suffix)?;

    for (exposed_id, method) in methods.iter().enumerate() {

        let element_length = match method.stream_size {
            StreamSize::Fixed(length) => Cow::Owned(format!("{length}")),
            StreamSize::Variable(VariableHeaderSize::Variable8) => Cow::Borrowed("var8"),
            StreamSize::Variable(VariableHeaderSize::Variable16) => Cow::Borrowed("var16"),
            StreamSize::Variable(VariableHeaderSize::Variable24) => Cow::Borrowed("var24"),
            StreamSize::Variable(VariableHeaderSize::Variable32) => Cow::Borrowed("var32"),
        };

        writeln!(writer, "        {}_{}(0x{exposed_id:02X}, {element_length}),", 
            method.interface.name, method.method.name)?;

    }
    
    writeln!(writer, "    }}")?;
    writeln!(writer, "}}")?;
    writeln!(writer)?;

    Ok(())

}

fn generate_interface(
    mut writer: impl Write, 
    model: &Model, 
    interface: &Interface,
    state: &mut State,
) -> io::Result<()> {
    
    writeln!(writer, "// ============================================== //")?;
    writeln!(writer, "// ====== {:^32} ====== //", interface.name)?;
    writeln!(writer, "// ============================================== //")?;
    writeln!(writer)?;
    
    writeln!(writer, "wgtk::__struct_simple_codec! {{")?;
    writeln!(writer, "    #[derive(Debug)]")?;
    writeln!(writer, "    pub struct {} {{", interface.name)?;
    
    for interface_name in &interface.implements {
        if !state.empty_interfaces.contains(interface_name) {
            writeln!(writer, "        pub i_{interface_name}: {interface_name},")?;
        }
    }

    let mut count = 0;
    for property in &interface.properties {
        if matches!(property.flags, PropertyFlags::AllClients | PropertyFlags::OwnClient | PropertyFlags::BaseAndClient) {

            let mut name = Cow::Borrowed("");
            let mut ty = Cow::Borrowed("");

            for patch in PATCHES {
                if let Patch::InterfaceProperty(func) = patch {
                    (func)(&interface.name, &property.name, &mut name, &mut ty);
                }
            }

            if name.is_empty() {
                name = Cow::Borrowed(&property.name);
            }

            if ty.is_empty() {
                ty = generate_type_ref(&property.ty);
            }

            writeln!(writer, "        pub {name}: {ty},")?;
            count += 1;

        }
    }

    if count == 0 {
        state.empty_interfaces.insert(interface.name.clone());
    }

    writeln!(writer, "    }}")?;
    writeln!(writer, "}}")?;
    writeln!(writer)?;

    for app_state in &mut state.apps {
        generate_interface_methods(&mut writer, model, interface, app_state)?;
    }

    Ok(())

}

fn generate_interface_methods(
    mut writer: impl Write,
    _model: &Model, 
    interface: &Interface,
    app_state: &mut AppState,
)  -> io::Result<()> {

    let mut unique_names = HashSet::new();
    
    writeln!(writer, "wgtk::__struct_simple_codec! {{  // Methods on {}", app_state.name)?;
    writeln!(writer)?;

    for method in (app_state.interface_methods)(interface) {

        if !is_method_exposed(method) {
            continue;
        }

        if !unique_names.insert(method.name.as_str()) {
            panic!("function name present multiple times: {}", method.name);
        }

        writeln!(writer, "    #[derive(Debug)]")?;
        writeln!(writer, "    pub struct {}_{} {{", interface.name, method.name)?;

        for (arg_idx, arg) in method.args.iter().enumerate() {

            let mut name = Cow::Borrowed("");
            let mut ty = Cow::Borrowed("");

            for patch in PATCHES {
                if let Patch::InterfaceMethodArg(func) = patch {
                    (func)(&interface.name, &method.name, arg_idx, &mut name, &mut ty);
                }
            }

            if name.is_empty() {
                name = Cow::Owned(format!("a{arg_idx}"));
            }

            if ty.is_empty() {
                ty = generate_type_ref(&arg.ty);
            }

            writeln!(writer, "        pub {name}: {ty},")?;

        }

        writeln!(writer, "    }}")?;
        writeln!(writer)?;

    }

    writeln!(writer, "}}")?;
    writeln!(writer)?;

    Ok(())

}

/// Return the stream size of this type, none if the type has no known size.
fn compute_type_stream_size(ty: &Ty) -> Option<usize> {
    match ty.kind() {
        TyKind::Int8 | TyKind::UInt8 => Some(1),
        TyKind::Int16 | TyKind::UInt16 => Some(2),
        TyKind::Int32 | TyKind::UInt32 => Some(4),
        TyKind::Int64 | TyKind::UInt64 => Some(8),
        TyKind::Float32 => Some(4),
        TyKind::Float64 => Some(8),
        TyKind::Vector2 => Some(4 * 2),
        TyKind::Vector3 => Some(4 * 3),
        TyKind::Vector4 => Some(4 * 4),
        TyKind::String => None,
        TyKind::Python => None,
        TyKind::Mailbox => None,  // TODO:
        TyKind::Alias(ty) => 
            compute_type_stream_size(ty),
        TyKind::Dict(ty_dict) => 
            ty_dict.properties.iter()
                .map(|prop| compute_type_stream_size(&prop.ty))
                .sum(),  // Using sum on Option: any None will result in a None.
        TyKind::Array(ty_seq) |
        TyKind::Tuple(ty_seq) => 
            ty_seq.size.map(|len| len as usize)
                .zip(compute_type_stream_size(&ty_seq.ty))
                .map(|(len, element_size)| len * element_size)
    }
}

/// This returns the preferred stream size.
fn compute_method_stream_size(method: &Method) -> StreamSize {
    
    let size = method.args.iter()
        .map(|arg| compute_type_stream_size(&arg.ty))
        .sum::<Option<usize>>();

    match size {
        Some(size) => StreamSize::Fixed(size),
        // TODO: Also return this is exposed and has sub msg id?
        None => StreamSize::Variable(method.variable_header_size)
    }

}

fn is_method_exposed(method: &Method) -> bool {
    method.exposed_to_all_clients || method.exposed_to_own_client
}

/// Internal state when bootstrapping.
#[derive(Debug)]
struct State {
    /// A set of interfaces without any fields (sizeof=0) for which it's useless to 
    /// generate variants.
    empty_interfaces: HashSet<String>,
    apps: [AppState; 3],
}

#[derive(Debug)]
struct AppState {
    name: &'static str,
    suffix: &'static str,
    interface_methods: fn(&Interface) -> &[Method],
}

impl State {
    fn new() -> Self {
        Self { 
            empty_interfaces: HashSet::new(), 
            apps: [
                AppState::new("client", "Client", |i| &i.client_methods),
                AppState::new("base", "Base", |i| &i.base_methods),
                AppState::new("cell", "Cell", |i| &i.cell_methods),
            ],
        }
    }
}

impl AppState {
    fn new(name: &'static str, suffix: &'static str, interface_methods: fn(&Interface) -> &[Method]) -> Self {
        Self {
            name,
            suffix,
            interface_methods,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum StreamSize {
    Fixed(usize),
    Variable(VariableHeaderSize),
}


#[derive(Debug, Clone)]
#[allow(unused)]
enum Patch {
    InterfaceProperty(fn(interface: &str, field: &str, name: &mut Cow<str>, ty: &mut Cow<str>)),
    InterfaceMethodArg(fn(interface: &str, method: &str, index: usize, name: &mut Cow<str>, ty: &mut Cow<str>)),
}

/// Patches to apply when generating code for World of Tanks.
const PATCHES: &[Patch] = &[
    Patch::InterfaceMethodArg(|interface, method, index, name, ty| {
        match (interface, method, index) {
            ("ClientCommandsPort", _, _) if method.starts_with("doCmd") => {
                *name = match index {
                    0 => "request_id".into(),
                    1 => "command_id".into(),
                    _ => format!("arg{}", index - 2).into(),
                };
            }
            ("ClientCommandsPort", _, _) if method.starts_with("onCmdResponse") => {
                *name = match index {
                    0 => "request_id".into(),
                    1 => "result_id".into(),
                    2 => "error".into(),
                    3 => "ext".into(),
                    _ => return,
                };
            }
            ("Chat", "chatCommandFromClient", _) => {
                *name = match index {
                    0 => "request_id".into(),
                    1 => "command_id".into(),
                    2 => "channel_id".into(),
                    3 => "i64_arg".into(),
                    4 => "i16_arg".into(),
                    5 => "str_arg0".into(),
                    6 => "str_arg1".into(),
                    _ => return,
                };
            }
            ("Chat", "inviteCommand", _) => {
                *name = match index {
                    0 => "request_id".into(),
                    1 => "command_id".into(),
                    2 => "invalid_type".into(),
                    3 => "receiver_name".into(),
                    4 => "i64_arg".into(),
                    5 => "i16_arg".into(),
                    6 => "str_arg0".into(),
                    7 => "str_arg1".into(),
                    _ => return,
                };
            }
            ("Chat", "ackCommand", _) => {
                *name = match index {
                    0 => "request_id".into(),
                    1 => "command_id".into(),
                    2 => "time".into(),
                    3 => "invite_id".into(),
                    _ => return,
                };
            }
            ("AccountUnitBrowser", "accountUnitBrowser_subscribe", 0) => *name = "unit_type_flags".into(),
            ("AccountUnitBrowser", "accountUnitBrowser_subscribe", 1) => *name = "show_other_locations".into(),
            ("AccountUnitBrowser", "accountUnitBrowser_recenter", 0) => *name = "target_rating".into(),
            ("AccountUnitBrowser", "accountUnitBrowser_recenter", 1) => *name = "unit_type_flags".into(),
            ("AccountUnitBrowser", "accountUnitBrowser_recenter", 2) => *name = "show_other_locations".into(),
            ("AccountUnitBrowser", "accountUnitBrowser_doCmd", 0) => *name = "cmd".into(),
            ("AccountAuthTokenProviderClient", "onTokenReceived", _) => {
                *name = match index {
                    0 => "request_id".into(),
                    1 => "token_type".into(),  // See TOKEN_TYPE in constants.py
                    2 => "data".into(),
                    _ => return,
                };
                if index == 2 {
                    *ty = "Python".into();
                }
            }
            ("RespawnController_Avatar", "redrawVehicleOnRespawn", 0) => *name = "vehicle_id".into(),
            ("RespawnController_Avatar", "redrawVehicleOnRespawn", 1) => *name = "new_vehicle_compact_description".into(),
            ("RespawnController_Avatar", "redrawVehicleOnRespawn", 2) => *name = "new_vehicle_outfit_compact_description".into(),
            ("RespawnController_Avatar", "explodeVehicleBeforeRespawn", 0) => *name = "vehicle_id".into(),
            ("RespawnController_Avatar", "updateRespawnVehicles", 0) => *name = "vehicles".into(),
            ("RespawnController_Avatar", "updateRespawnCooldowns", 0) => *name = "cooldowns".into(),
            ("RespawnController_Avatar", "updateRespawnInfo", 0) => *name = "info".into(),
            ("RespawnController_Avatar", "updateVehicleLimits", 0) => *name = "limits".into(),
            ("RespawnController_Avatar", "updatePlayerLives", 0) => *name = "lives".into(),
            ("RespawnController_Avatar", "onTeamLivesRestored", 0) => *name = "teams".into(),
            ("RespawnController_Avatar", "respawnController_requestRespawnGroupChange", 0) => *name = "lane_id".into(),
            ("RespawnController_Avatar", "respawnController_chooseVehicleForRespawn", 0) => *name = "int_cd".into(),
            ("RespawnController_Avatar", "respawnController_chooseRespawnZone", 0) => *name = "respawn_zone".into(),
            ("RespawnController_Avatar", "respawnController_switchSetup", 0) => *name = "vehicle_id".into(),
            ("RespawnController_Avatar", "respawnController_switchSetup", 1) => *name = "group_id".into(),
            ("RespawnController_Avatar", "respawnController_switchSetup", 2) => *name = "layout_index".into(),
            ("RecoveryMechanic_Avatar", "updateState", 0) => *name = "activated".into(),
            ("RecoveryMechanic_Avatar", "updateState", 1) => *name = "state".into(),
            ("RecoveryMechanic_Avatar", "updateState", 2) => *name = "timer_duration".into(),
            ("RecoveryMechanic_Avatar", "updateState", 3) => *name = "end_of_timer".into(),
            ("PlayerMessenger_chat2", "messenger_onActionByServer_chat2" | "messenger_onActionByClient_chat2", 0) => *name = "action_id".into(),
            ("PlayerMessenger_chat2", "messenger_onActionByServer_chat2" | "messenger_onActionByClient_chat2", 1) => *name = "request_id".into(),
            ("PlayerMessenger_chat2", "messenger_onActionByServer_chat2" | "messenger_onActionByClient_chat2", 2) => *name = "args".into(),
            ("AvatarEpic", "welcomeToSector", _) => {
                *name = match index {
                    0 => "sector_id".into(),
                    1 => "group_id".into(),
                    2 => "group_state".into(),
                    3 => "good_group".into(),
                    4 => "action_time".into(),
                    5 => "action_duration".into(),
                    _ => return,
                };
            }
            ("AvatarEpic", "onStepRepairPointAction", _) => {
                *name = match index {
                    0 => "repair_point_index".into(),
                    1 => "action".into(),
                    2 => "next_action_time".into(),
                    3 => "points_healed".into(),
                    _ => return,
                };
            }
            ("AvatarEpic", "onSectorBaseAction", 0) => *name = "sector_base_id".into(),
            ("AvatarEpic", "onSectorBaseAction", 1) => *name = "action".into(),
            ("AvatarEpic", "onSectorBaseAction", 2) => *name = "next_action_time".into(),
            ("AvatarEpic", 
                "enteringProtectionZone" | 
                "leavingProtectionZone" | 
                "protectionZoneShooting", 0) => *name = "zone_id".into(),
            ("AvatarEpic", "onSectorShooting", 0) => *name = "sector_id".into(),
            ("AvatarEpic", "onXPUpdated", 0) => *name = "xp".into(),
            ("AvatarEpic", "onCrewRoleFactorAndRankUpdate", 0) => *name = "new_factor".into(),
            ("AvatarEpic", "onCrewRoleFactorAndRankUpdate", 1) => *name = "ally_vehicle_id".into(),
            ("AvatarEpic", "onCrewRoleFactorAndRankUpdate", 2) => *name = "ally_new_rank".into(),
            ("AvatarEpic", "syncPurchasedAbilities", 0) => *name = "abilities".into(),
            ("AvatarEpic", "onRandomReserveOffer", 0) => *name = "offer".into(),
            ("AvatarEpic", "onRandomReserveOffer", 1) => *name = "level".into(),
            ("AvatarEpic", "onRandomReserveOffer", 2) => *name = "slot_index".into(),
            ("AvatarEpic", "onRankUpdate", 0) => *name = "new_rank".into(),
            ("AvatarEpic", "showDestructibleShotResults" | "onDestructibleDestroyed", 0) => *name = "destructible_entity_id".into(),
            ("AvatarEpic", "showDestructibleShotResults", 1) => *name = "hit_flags".into(),
            ("AvatarEpic", "onDestructibleDestroyed", 1) => *name = "shooter_id".into(),
            ("AccountPrebattle", "accountPrebattle_createTraining", 0) => *name = "arena_type_id".into(),
            ("AccountPrebattle", "accountPrebattle_createTraining", 1) => *name = "round_length".into(),
            ("AccountPrebattle", "accountPrebattle_createTraining", 2) => *name = "is_opened".into(),
            ("AccountPrebattle", "accountPrebattle_createTraining", 3) => *name = "comment".into(),
            ("AccountPrebattle", "accountPrebattle_createDevPrebattle", 0) => *name = "bonus_type".into(),
            ("AccountPrebattle", "accountPrebattle_createDevPrebattle", 1) => *name = "arena_gui_type".into(),
            ("AccountPrebattle", "accountPrebattle_createDevPrebattle", 2) => *name = "arena_type_id".into(),
            ("AccountPrebattle", "accountPrebattle_createDevPrebattle", 3) => *name = "round_length".into(),
            ("AccountPrebattle", "accountPrebattle_createDevPrebattle", 4) => *name = "comment".into(),
            ("AccountPrebattle", "accountPrebattle_sendPrebattleInvites", 0) => *name = "accounts".into(),
            ("AccountPrebattle", "accountPrebattle_sendPrebattleInvites", 1) => *name = "comment".into(),
            ("AccountGlobalMapConnector", "accountGlobalMapConnector_callGlobalMapMethod", 0) => *name = "request_id".into(),
            ("AccountGlobalMapConnector", "accountGlobalMapConnector_callGlobalMapMethod", 1) => *name = "method".into(),  // See GM_CLIENT_METHOD
            ("AccountGlobalMapConnector", "accountGlobalMapConnector_callGlobalMapMethod", 2) => *name = "i64_arg".into(), // See scripts/client/ClientGlobalMap.py
            ("AccountGlobalMapConnector", "accountGlobalMapConnector_callGlobalMapMethod", 3) => *name = "str_arg".into(),
            ("AccountAuthTokenProvider", "requestToken", 0) => *name = "request_id".into(),
            ("AccountAuthTokenProvider", "requestToken", 1) => *name = "token_type".into(),
            ("Account", "onKickedFromServer", 0) => *name = "reason".into(),
            ("Account", "onKickedFromServer", 1) => *name = "kick_reason_type".into(),
            ("Account", "onKickedFromServer", 2) => *name = "expiry_time".into(),
            ("Account", 
                "onEnqueued" | 
                "onDequeued" | 
                "onEnqueueFailure" | 
                "onKickedFromQueue", 0) => *name = "queue_type".into(),
            ("Account", "onEnqueueFailure", 1) => *name = "error_code".into(),
            ("Account", "onEnqueueFailure", 2) => *name = "error_str".into(),
            ("Account", "onIGRTypeChanged" | "showGUI", 0) => {
                *name = "data".into();
                *ty = "Python".into();
            }
            ("Account", "onArenaJoinFailure", 0) => *name = "error_code".into(),
            ("Account", "onArenaJoinFailure", 1) => *name = "error_str".into(),
            ("Account", "onPrebattleJoined", 0) => *name = "prebattle_id".into(),
            ("Account", "onPrebattleJoinFailure", 0) => *name = "error_code".into(),
            ("Account", "onKickedFromArena" | "onKickedFromPrebattle", 0) => *name = "reason_code".into(),
            ("Account", "onCenterIsLongDisconnected", 0) => *name = "is_long_disconnected".into(),
            ("Account", "receiveActiveArenas", 0) => *name = "arenas".into(),
            ("Account", "receiveServerStats", 0) => *name = "stats".into(),
            ("Account", "receiveQueueInfo", 0) => *name = "info".into(),
            ("Account", "updatePrebattle", 0) => *name = "update_type".into(),
            ("Account", "updatePrebattle", 1) => *name = "str_arg".into(),
            ("Account", "update", 0) => *name = "diff".into(),
            ("Account", "resyncDossiers", 0) => *name = "is_full_resync".into(),
            ("Account", "onUnitUpdate", 0) => *name = "unit_manager_id".into(),
            ("Account", "onUnitUpdate", 1) => *name = "packed_unit".into(),
            ("Account", "onUnitUpdate", 2) => *name = "packed_ops".into(),
            ("Account", "onUnitCallOk", 0) => *name = "request_id".into(),
            ("Account", "onUnitNotify", 0) => *name = "unit_manager_id".into(),
            ("Account", "onUnitNotify", 1) => *name = "notify_code".into(),
            ("Account", "onUnitNotify", 2) => *name = "notify_str".into(),
            ("Account", "onUnitNotify", 3) => *name = "args".into(),
            ("Account", "onUnitError", 0) => *name = "request_id".into(),
            ("Account", "onUnitError", 1) => *name = "unit_manager_id".into(),
            ("Account", "onUnitError", 2) => *name = "error_code".into(),
            ("Account", "onUnitError", 3) => *name = "error_str".into(),
            ("Account", "onUnitBrowserError", 0) => *name = "error_code".into(),
            ("Account", "onUnitBrowserError", 1) => *name = "error_str".into(),
            ("Account", "onUnitBrowserResultsSet", 0) => {
                *name = "browser_results".into();
                *ty = "Python".into();
            }
            ("Account", "onUnitBrowserResultsUpdate", 0) => {
                *name = "browser_updates".into();
                *ty = "Python".into();
            }
            ("Account", "onGlobalMapUpdate", 0) => *name = "packed_ops".into(),
            ("Account", "onGlobalMapUpdate", 1) => *name = "packed_update".into(),
            ("Account", "onGlobalMapReply", 0) => *name = "request_id".into(),
            ("Account", "onGlobalMapReply", 1) => *name = "result_code".into(),
            ("Account", "onGlobalMapReply", 2) => *name = "result_str".into(),
            ("Account", "onSendPrebattleInvites", 0) => *name = "id".into(),
            ("Account", "onSendPrebattleInvites", 1) => *name = "name".into(),
            ("Account", "onSendPrebattleInvites", 2) => *name = "clan_id".into(),
            ("Account", "onSendPrebattleInvites", 3) => *name = "clan_abbrev".into(),
            ("Account", "onSendPrebattleInvites", 4) => *name = "prebattle_id".into(),
            ("Account", "onSendPrebattleInvites", 5) => *name = "status".into(),
            ("Account", "onClanInfoReceived", 0) => *name = "id".into(),
            ("Account", "onClanInfoReceived", 1) => *name = "name".into(),
            ("Account", "onClanInfoReceived", 2) => *name = "abbrev".into(),
            ("Account", "onClanInfoReceived", 3) => *name = "motto".into(),
            ("Account", "onClanInfoReceived", 4) => *name = "description".into(),
            ("Account", "receiveNotification", 0) => *name = "notification".into(),
            ("Account", "requestToken", 0) => *name = "request_id".into(),
            ("Account", "requestToken", 1) => *name = "token_type".into(),
            ("Account", "logStreamCorruption", 0) => *name = "stream_id".into(),
            ("Account", "logStreamCorruption", 1) => *name = "original_packet_len".into(),
            ("Account", "logStreamCorruption", 2) => *name = "packet_len".into(),
            ("Account", "logStreamCorruption", 3) => *name = "original_crc32".into(),
            ("Account", "logStreamCorruption", 4) => *name = "crc32".into(),
            _ => {}
        }
    })
];
