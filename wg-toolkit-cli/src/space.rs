use std::collections::HashMap;
use std::io::Cursor;

use wgtk::space::bin::{SpaceBin, BSMI, BSMO, BWST};

use super::{read_input, CliResult, SpaceArgs, SpaceCommand, SpaceVectorsArgs};


pub fn cmd_space(args: SpaceArgs) -> CliResult<()> {

    let content = read_input(args.file.as_deref())?;
    let mut space = SpaceBin::from_reader(Cursor::new(content))
        .map_err(|e| format!("Failed to read compiled space: {e}"))?;

    match args.cmd {
        SpaceCommand::Sections => {
            for header in space.sections() {
                println!("{} {:>10} {:>10}", id_str(&header.id), header.offset, header.len);
            }
        }
        SpaceCommand::Strings => {
            let strings = strings(&mut space)?;
            let mut strings: Vec<_> = strings.strings.iter().collect();
            strings.sort_by(|a, b| a.1.cmp(b.1));
            for (hash, string) in strings {
                println!("{hash:08x} {string}");
            }
        }
        SpaceCommand::Vectors(args) => cmd_vectors(&mut space, args)?,
        SpaceCommand::Models => cmd_models(&mut space)?,
    }

    Ok(())

}

fn cmd_vectors(space: &mut Bin, args: SpaceVectorsArgs) -> CliResult<()> {

    let strings = strings(space).ok();
    let id: [u8; 4] = args.section.as_bytes().try_into()
        .map_err(|_| format!("A section identifier has 4 characters, not '{}'", args.section))?;
    let bytes = space.section_bytes(id)
        .map_err(|e| format!("Failed to read section {}: {e}", args.section))?
        .ok_or_else(|| format!("No section {}", args.section))?;

    // Vectors follow one another as long as their heads make sense.
    let mut pos = 0;
    let mut index = 0;
    while pos + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let count = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if size == 0 || pos + 8 + size * count > bytes.len() {
            break;
        }
        println!("#{index} at {pos}: {count} × {size} bytes");
        for row in 0..count.min(args.rows) {
            let start = pos + 8 + row * size;
            let words: Vec<String> = bytes[start..start + size].chunks_exact(4)
                .map(|word| describe_word(u32::from_le_bytes(word.try_into().unwrap()), strings.as_ref()))
                .collect();
            println!("  [{row}] {}", words.join(" "));
        }
        pos += 8 + size * count;
        index += 1;
    }

    if pos < bytes.len() {
        println!("{} bytes left that aren't a vector, from {pos}", bytes.len() - pos);
    }

    Ok(())

}

fn cmd_models(space: &mut Bin) -> CliResult<()> {

    let strings = strings(space)?;
    let models: BSMO = space.read_section().map_err(|e| format!("Failed to read the static models: {e}"))?;
    let instances: BSMI = space.read_section().map_err(|e| format!("Failed to read the static model instances: {e}"))?;

    let mut placed = HashMap::<u32, usize>::new();
    for instance in &instances.instances {
        *placed.entry(instance.model).or_default() += 1;
    }

    let name = |hash: u32| strings.get_string(hash).unwrap_or("?").to_string();
    for (index, model) in models.models.iter().enumerate() {
        let [min, max] = model.bounds;
        println!("#{index} placed {}: {}", placed.get(&(index as u32)).copied().unwrap_or(0), name(model.primitives_fnv));
        match model.havok_fnv {
            Some(havok) => println!("  collision {} ({})", name(havok), name(model.material_fnv)),
            None => println!("  no collision"),
        }
        println!("  bounds {min} {max}");
    }

    Ok(())

}

/// A compiled space read whole from the input.
type Bin = SpaceBin<Cursor<Vec<u8>>>;

fn strings(space: &mut Bin) -> CliResult<BWST> {
    space.read_section().map_err(|e| format!("Failed to read the string table: {e}"))
}

/// A 32-bit word as an integer and a float, or the string it is the hash of.
fn describe_word(word: u32, strings: Option<&BWST>) -> String {
    if let Some(string) = strings.and_then(|strings| strings.get_string(word)) {
        return format!("\"{string}\"");
    }
    let float = f32::from_bits(word);
    if float.is_finite() && float != 0.0 && (1e-4..1e7).contains(&float.abs()) {
        format!("{word}|{float}")
    } else {
        format!("{word}")
    }
}

fn id_str(id: &[u8; 4]) -> String {
    id.iter().map(|&c| c as char).collect()
}
