//! Parsing logic for the script definitions.

use crate::pxml::{Element, Value};

use super::def::{Arg, Entity, Interface, Method, Property, PropertyFlags, VariableHeaderSize};
use super::ty::{Ty, TyDict, TyDictProp, TyKind, TySeq, TySystem};


pub fn parse_aliases(elt: &Element, tys: &mut TySystem) {
    for (name, val) in elt.iter_children_all() {
        parse_ty(val, &mut *tys, Some(name.clone()));
    }
}

pub fn parse_interface(elt: &Element, tys: &mut TySystem, name: String) -> Interface {

    let mut interface = Interface {
        name: name.into(),
        implements: Vec::new(),
        properties: Vec::new(),
        temp_properties: Vec::new(),
        client_methods: Vec::new(),
        base_methods: Vec::new(),
        cell_methods: Vec::new(),
    };

    if let Some(Value::Element(elt)) = elt.get_child("Implements") {
        for interface_elt in elt.iter_children("Interface") {
            if let Some(interface_name) = interface_elt.as_string() {
                interface.implements.push(interface_name.to_string());
            }
        }
    }

    if let Some(Value::Element(elt)) = elt.get_child("TempProperties") {
        for (temp_name, _) in elt.iter_children_all() {
            interface.temp_properties.push(temp_name.clone());
        }
    }

    if let Some(Value::Element(elt)) = elt.get_child("Properties") {
        parse_properties(&elt, &mut *tys, &mut interface.properties);
    }

    if let Some(Value::Element(elt)) = elt.get_child("ClientMethods") {
        parse_methods(&elt, &mut *tys, &mut interface.client_methods, true);
    }

    if let Some(Value::Element(elt)) = elt.get_child("BaseMethods") {
        parse_methods(&elt, &mut *tys, &mut interface.base_methods, false);
    }

    if let Some(Value::Element(elt)) = elt.get_child("CellMethods") {
        parse_methods(&elt, &mut *tys, &mut interface.cell_methods, false);
    }

    interface

}

/// Parse the `<ofEntity>` element of a component def, returning the names of the
/// entities this component targets (usually exactly one).
pub fn parse_of_entity(elt: &Element) -> Vec<String> {
    parse_names(elt, "ofEntity")
}

/// Parse a list of plain child-tag names directly under `elt.get_child(tag)`, e.g.
/// `<ServerOnlyEntities><Foo/><Bar/></ServerOnlyEntities>`. Returns an empty list if the
/// tag is absent, or present but empty -- including a self-closing empty tag (e.g.
/// `frontline`'s `<ClientServerEntities/>`), which parses as a blank string rather than
/// an element.
pub fn parse_names(elt: &Element, tag: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(Value::Element(elt)) = elt.get_child(tag) {
        for (name, _) in elt.iter_children_all() {
            names.push(name.clone());
        }
    }
    names
}

pub fn parse_entity(elt: &Element, tys: &mut TySystem, id: usize, name: String, from_extension: Option<String>) -> Entity {

    let interface = parse_interface(elt, tys, name);

    let entity = Entity {
        interface,
        id,
        parent: elt.get_child("Parent").and_then(Value::as_string).map(str::to_string),
        from_extension,
    };

    entity

}

pub fn parse_properties(elt: &Element, tys: &mut TySystem, properties: &mut Vec<Property>) {
    for (name, val) in elt.iter_children_all() {
        if let Value::Element(property_elt) = val {
            properties.push(parse_property(&property_elt, &mut *tys, name.clone()));
        }
    }
}

pub fn parse_property(elt: &Element, tys: &mut TySystem, name: String) -> Property {

    let ty_val = elt.get_child("Type").expect("property should contain a type");
    let ty = parse_ty(ty_val, &mut *tys, None);

    let flags = elt.get_child("Flags")
        .and_then(Value::as_string)
        .unwrap_or_default();

    // TODO: AllowUnsafeData
    // TODO: Backupable
    // TODO: ExposedForReplay

    Property {
        name: name.into(),
        ty,
        persistent: elt.get_child("Persistent")
            .and_then(Value::as_boolean)
            .unwrap_or_default(),
        identifier: elt.get_child("Identifier")
            .and_then(Value::as_boolean)
            .unwrap_or_default(),
        indexed: elt.get_child("Indexed")
            .and_then(Value::as_boolean)
            .unwrap_or_default(),
        database_len: elt.get_child("DatabaseLength")
            .and_then(Value::as_integer)
            .and_then(|v| u32::try_from(v).ok()),
        default: None, // TODO:
        flags: match flags {
            "" => PropertyFlags::None,
            "BASE" => PropertyFlags::Base,
            "BASE_AND_CLIENT" => PropertyFlags::BaseAndClient,
            "OWN_CLIENT" => PropertyFlags::OwnClient,
            "CELL_PRIVATE" => PropertyFlags::CellPrivate,
            "CELL_PUBLIC" => PropertyFlags::CellPublic,
            "ALL_CLIENTS" => PropertyFlags::AllClients,
            raw => panic!("unknown property flags: {raw}"),
        },
    }

}

pub fn parse_methods(elt: &Element, tys: &mut TySystem, methods: &mut Vec<Method>, client: bool) {
    let empty_element = Element::new();
    for (name, val) in elt.iter_children_all() {
        let method_elt = match val {
            Value::Element(elt) => &**elt,
            Value::String(s) if s.is_empty() => &empty_element,
            _ => panic!("unknown method def: {name}: {val:?}")
        };
        methods.push(parse_method(method_elt, &mut *tys, name.clone(), client));
    }
}

pub fn parse_method(elt: &Element, tys: &mut TySystem, name: String, client: bool) -> Method {
    
    let mut method = Method {
        name: name.into(),
        exposed_to_all_clients: client,
        exposed_to_own_client: false,
        variable_header_size: VariableHeaderSize::Variable8,
        args: Vec::new(),
    };

    if let Some(exposed) = elt.get_child("Exposed") {

        assert!(!client, "exposed flags are not supported on client method");

        match exposed.as_string().unwrap_or_default() {
            "ALL_CLIENTS" => {
                // Only supported on cell.
                method.exposed_to_all_clients = true;
            }
            "OWN_CLIENT" => {
                method.exposed_to_own_client = true;
            }
            "" => {
                method.exposed_to_all_clients = true;
                method.exposed_to_own_client = true;
            }
            raw => panic!("unknown method exposed flag: {raw}")
        }

    }

    if let Some(size) = elt.get_child("VariableLengthHeaderSize").and_then(Value::as_integer) {
        method.variable_header_size = match size {
            1 => VariableHeaderSize::Variable8,
            2 => VariableHeaderSize::Variable16,
            3 => VariableHeaderSize::Variable24,
            4 => VariableHeaderSize::Variable32,
            _ => panic!("invalid variable length header size: {size}")
        };
    }

    // TODO: AllowUnsafeData
    // TODO: IgnoreIfNoClient
    // TODO: ReplayExposureLevel

    for arg_val in elt.iter_children("Arg") {
        let ty = parse_ty(arg_val, &mut *tys, None);
        method.args.push(Arg {
            ty,
        });
    }

    method
    
}

/// Parse the type from the given value, the type may be created and registered in the
/// type system if not previously existing. The given alias name is used when defining
/// aliases, it allows giving a non-anonymous name to a type, it also allows creating
/// an `Alias` type kind for simple type references.
pub fn parse_ty(val: &Value, tys: &mut TySystem, alias_name: Option<String>) -> Ty {
    match val {
        Value::String(name) => {

            let Some(ty) = tys.find(&name) else {
                panic!("unknown type: {name}");
            };

            if let Some(alias_name) = alias_name {
                tys.register(Some(alias_name), TyKind::Alias(ty))
            } else {
                ty
            }

        }
        Value::Element(elt)
        if elt.value.as_string() == Some("FIXED_DICT") => {
            
            let properties_elt = elt.get_child("Properties")
                .and_then(Value::as_element)
                .expect("fixed dict should have properties");

            let mut dict = TyDict::default();
            for (field_name, field_val) in properties_elt.iter_children_all() {

                let field_elt = field_val.as_element().expect("field should be an element");
                let type_val = field_elt.get_child("Type").expect("field should contain a type");
                let ty = parse_ty(type_val, tys, None);

                dict.properties.push(TyDictProp {
                    name: field_name.clone().into(),
                    ty,
                    default: None,
                });

            }

            tys.register(alias_name, TyKind::Dict(dict))

        }
        Value::Element(elt) => {

            let kind = match elt.value.as_string() {
                None => panic!("missing type element value: {val:?}"),
                Some("ARRAY") => TyKind::Array,
                Some("TUPLE") => TyKind::Tuple,
                Some(name) => {
                    // TODO: Support for default value.
                    match tys.find(&name) {
                        Some(ty) => return ty,
                        None => panic!("unknown type: {name}")
                    }
                }
            };

            let ty = elt.get_child("of")
                .map(|val| parse_ty(val, &mut *tys, None))
                .expect("missing array type: {val:?}");

            let size = elt.get_child("size")
                .map(|val| val.as_integer().unwrap())
                .and_then(|v| u32::try_from(v).ok());

            let kind = (kind)(TySeq {
                ty,
                size,
            });

            tys.register(alias_name, kind)

        }
        _ => panic!("unsupported type: {val:?}")
    }
}
