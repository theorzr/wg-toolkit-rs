use std::io::Cursor;

use wgtk::havok::tagfile::{Kind, Node, Tagfile};
use wgtk::havok::collision::Collision;

use super::{read_input, CliResult, HavokArgs, HavokCommand, HavokTreeArgs};


pub fn cmd_havok(args: HavokArgs) -> CliResult<()> {

    let content = read_input(args.file.as_deref())?;
    let tagfile = Tagfile::from_reader(Cursor::new(content))
        .map_err(|e| format!("Failed to read Havok tagfile: {e}"))?;

    match args.cmd {
        HavokCommand::Types => {
            println!("SDK {}", tagfile.sdk_version);
            for ty in tagfile.types() {
                let parent = ty.parent.map(|parent| format!(" : {parent}")).unwrap_or_default();
                println!("#{} {}{parent} (format {:#x}, {} bytes)", ty.id, ty.name, ty.format, ty.size);
                for (name, offset, ty) in ty.members {
                    println!("  +{offset} {name}: {ty}");
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
            println!("bounds {min} {max}");
            for body in &collision.bodies {
                println!("{} material kind {}: {} triangles", body.name, body.material_kind, body.triangles.len());
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
