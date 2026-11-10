use std::io::Cursor;

use wgtk::havok::tagfile::{Kind, Node, Tagfile, TypeInfo};
use wgtk::havok::collision::Collision;

use super::{read_input, CliOptions, CliResult, HavokArgs, HavokCommand, HavokTreeArgs};


/// Machine readable, `types` and `collision` print one record per line, its kind first,
/// its fields separated by tabs; human readable, as sentences. `tree` is the same both
/// ways, indented as `pxml` does.
pub fn cmd_havok(opts: CliOptions, args: HavokArgs) -> CliResult<()> {

    let content = read_input(args.file.as_deref())?;
    let tagfile = Tagfile::from_reader(Cursor::new(content))
        .map_err(|e| format!("Failed to read Havok tagfile: {e}"))?;

    match args.cmd {
        HavokCommand::Types(args) => {
            let types: Vec<TypeInfo> = tagfile.types().collect();
            if !opts.human {
                // `sdk`, then `type` (its id, name, parent, kind, size, format and pointee
                // id, `-` or 0 for none) and its `member`s (its type's id, their offset,
                // name and type id).
                println!("sdk\t{}", tagfile.sdk_version);
                for ty in types.iter().filter(|ty| args.all || !(ty.kind == Kind::Other && ty.size == 0)) {
                    println!("type\t{}\t{}\t{}\t{}\t{}\t{:#x}\t{}", ty.id, ty.name, ty.parent.unwrap_or("-"),
                        kind_name(ty.kind), ty.size, ty.format, ty.pointee);
                    for &(name, offset, member) in &ty.members {
                        println!("member\t{}\t{offset}\t{name}\t{member}", ty.id);
                    }
                }
                return Ok(());
            }
            println!("SDK {}", tagfile.sdk_version);
            for ty in &types {
                let declared_only = ty.kind == Kind::Other && ty.size == 0;
                if declared_only && !args.all {
                    continue;
                }
                let parent = ty.parent.map(|parent| format!(" (extends {parent})")).unwrap_or_default();
                println!("#{} {}{parent}: {}", ty.id, label(&types, ty.id, 0), describe(&types, ty));
                if ty.kind == Kind::Record {
                    for &(name, offset, member) in &ty.members {
                        println!("    +{offset:<3} {name}: {}", label(&types, member, 0));
                    }
                }
            }
        }
        HavokCommand::Tree(args) => {
            let root = tagfile.root().ok_or("The tagfile has no root object")?;
            print_node(root, "root", 0, &args);
        }
        HavokCommand::Collision => {
            let collision = Collision::from_tagfile(&tagfile).ok_or("The tagfile isn't a collision model")?;
            let [min, max] = collision.bounds;
            if opts.human {
                println!("bounds {min} {max}");
                for body in &collision.bodies {
                    println!("{} material kind {}, flags {}: {} triangles", body.name, body.material_kind, body.flags, body.triangles.len());
                }
            } else {
                // `bounds` (minimum then maximum coordinates), then each `body` (its name,
                // material kind, collision flags and triangle count).
                println!("bounds\t{}\t{}\t{}\t{}\t{}\t{}", min.x, min.y, min.z, max.x, max.y, max.z);
                for body in &collision.bodies {
                    println!("body\t{}\t{}\t{}\t{}", body.name, body.material_kind, body.flags, body.triangles.len());
                }
            }
        }
    }

    Ok(())

}

/// Print a node as `name: type = value`, then its members, pointee or elements below it,
/// down to the depth asked.
fn print_node(node: Node, name: &str, depth: usize, args: &HavokTreeArgs) {

    let indent = "  ".repeat(depth);
    let ty = node.type_name();
    let value = match node.kind() {
        Kind::Bool => node.as_bool().map(|value| value.to_string()),
        Kind::Int => node.as_int().map(|value| value.to_string()),
        Kind::Float => node.as_f32().map(|value| value.to_string()),
        Kind::String => Some(node.as_str().map_or("null".to_string(), |value| format!("{value:?}"))),
        Kind::Array => node.elements().map(|elements| format!("[{}]", elements.len())),
        Kind::Pointer => node.deref().is_none().then(|| "null".to_string()),
        // A record wrapping a string, an `hkStringPtr`, shows as one.
        Kind::Record => node.as_str().map(|value| format!("{value:?}")),
        Kind::Other => None,
    };

    match &value {
        Some(value) => println!("{indent}{name}: {ty} = {value}"),
        None => println!("{indent}{name}: {ty}"),
    }

    if depth >= args.depth {
        return;
    }

    match node.kind() {
        Kind::Record if value.is_none() => {
            for (member, child) in node.members() {
                print_node(child, member, depth + 1, args);
            }
        }
        Kind::Pointer => {
            if let Some(pointee) = node.deref() {
                print_node(pointee, "*", depth + 1, args);
            }
        }
        Kind::Array => {
            let Some(elements) = node.elements() else { return };
            // Arrays of numbers on one line.
            let numbers: Option<Vec<String>> = elements.into_iter().take(args.elements)
                .map(|element| element.as_f32().map(|v| v.to_string()).or_else(|| element.as_int().map(|v| v.to_string())))
                .collect();
            match numbers {
                Some(numbers) if !numbers.is_empty() => {
                    let more = if elements.len() > args.elements { " …" } else { "" };
                    println!("{indent}  {}{more}", numbers.join(" "));
                }
                _ => {
                    for (index, element) in elements.into_iter().take(args.elements).enumerate() {
                        print_node(element, &format!("[{index}]"), depth + 1, args);
                    }
                    if elements.len() > args.elements {
                        println!("{indent}  … {} more", elements.len() - args.elements);
                    }
                }
            }
        }
        _ => {}
    }

}

/// A type's name, with what a pointer or an array points to: `hkArray<hkVector4f>`,
/// `hkRefPtr<hknpShape>`, `float[4]`.
fn label(types: &[TypeInfo], id: usize, depth: usize) -> String {
    let Some(ty) = id.checked_sub(1).and_then(|index| types.get(index)) else { return "void".to_string() };
    // One extending another, `hkVector4` of `hkVector4f` say, keeps its own name.
    if depth > 4 || ty.pointee == 0 || ty.parent.is_some() || !matches!(ty.kind, Kind::Pointer | Kind::Array) {
        return ty.name.to_string();
    }
    let pointee = label(types, ty.pointee, depth + 1);
    match ty.kind {
        Kind::Array if ty.format & 0x20 != 0 => format!("{pointee}[{}]", ty.format >> 8),
        Kind::Pointer if ty.name == "T*" => format!("{pointee}*"),
        _ => format!("{}<{pointee}>", ty.name),
    }
}

/// What a type is, and how it is laid out in the data.
fn describe(types: &[TypeInfo], ty: &TypeInfo) -> String {
    let item = "a 4-byte item index";
    match ty.kind {
        Kind::Bool => format!("bool, {}", bytes(ty.size)),
        Kind::String => format!("string, {item}"),
        Kind::Int => format!("{}{}", if ty.format & 0x200 != 0 { "i" } else { "u" }, ty.size * 8),
        Kind::Float => format!("f{}", ty.size * 8),
        Kind::Pointer => format!("pointer, {item}"),
        Kind::Array if ty.format & 0x20 != 0 => format!("{} × {}, inline, {}", ty.format >> 8, label(types, ty.pointee, 0), bytes(ty.size)),
        Kind::Array => format!("array, {item}"),
        Kind::Record if ty.members.is_empty() && ty.size <= 1 => "record, no members of its own".to_string(),
        Kind::Record => format!("record, {}", bytes(ty.size)),
        Kind::Other if ty.size == 0 => "declared only".to_string(),
        Kind::Other => format!("opaque, {}", bytes(ty.size)),
    }
}

fn bytes(size: usize) -> String {
    if size == 1 { "1 byte".to_string() } else { format!("{size} bytes") }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Bool => "bool",
        Kind::String => "string",
        Kind::Int => "int",
        Kind::Float => "float",
        Kind::Pointer => "pointer",
        Kind::Record => "record",
        Kind::Array => "array",
        Kind::Other => "other",
    }
}
