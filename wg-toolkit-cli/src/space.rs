use std::collections::HashMap;
use std::io::Cursor;

use wgtk::space::bin::{SpaceBin, SpaceSection, BSMI, BSMO, BWAL, BWCS, BWSG, BWST, BWT2};

use super::{read_input, CliOptions, CliResult, SpaceArgs, SpaceCommand, SpaceVectorsArgs};


/// Machine readable, each command prints one record per line, its fields separated by
/// tabs, a record kind first when there are several; human readable, aligned and with
/// units.
pub fn cmd_space(opts: CliOptions, args: SpaceArgs) -> CliResult<()> {

    let content = read_input(args.file.as_deref())?;
    let mut space = SpaceBin::from_reader(Cursor::new(content))
        .map_err(|e| format!("Failed to read compiled space: {e}"))?;

    match args.cmd {
        SpaceCommand::Sections => {
            // What the sections decoded are, as `wgtk::space::bin` names them.
            let known = [
                (BWST::ID, "BWST, string table"),
                (BWAL::ID, "BWAL, asset list"),
                (BWCS::ID, "BWCS, compiled space settings"),
                (BWSG::ID, "BWSG, static geometry"),
                (*b"BSGD", "static geometry data, BWSG's"),
                (BWT2::ID, "BWT2, terrain"),
                (BSMO::ID, "BSMO, static models"),
                (BSMI::ID, "BSMI, static model instances"),
            ];
            if opts.human {
                println!("id        offset       size  decoded as");
            }
            for header in space.sections() {
                let id = id_str(&header.id);
                if opts.human {
                    let decoded = known.iter().find(|(id, _)| *id == header.id).map_or("-", |(_, name)| name);
                    println!("{id}  {:>10}  {:>9}  {decoded}", header.offset, kilobytes(header.len));
                } else {
                    println!("{id}\t{}\t{}", header.offset, header.len);
                }
            }
        }
        SpaceCommand::Strings => {
            let strings = strings(&mut space)?;
            let mut strings: Vec<_> = strings.strings.iter().collect();
            strings.sort_by(|a, b| a.1.cmp(b.1));
            let separator = if opts.human { " " } else { "\t" };
            for (hash, string) in strings {
                println!("{hash:08x}{separator}{string}");
            }
        }
        SpaceCommand::Vectors(args) => cmd_vectors(&opts, &mut space, args)?,
        SpaceCommand::Models => cmd_models(&opts, &mut space)?,
    }

    Ok(())

}

/// Machine readable, a `vector` record for each vector (its index, offset, count and
/// element size), then an `element` record for each element shown (its vector's index,
/// its own, then its words as hexadecimal, or its bytes), and a `rest` record for what
/// follows the vectors (its offset and size). Human readable, the words as what they
/// most likely are.
fn cmd_vectors(opts: &CliOptions, space: &mut Bin, args: SpaceVectorsArgs) -> CliResult<()> {

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
        if opts.human {
            let words = match size / 4 {
                _ if size % 4 != 0 => String::new(),
                1 => ", 1 word".to_string(),
                words => format!(", {words} words"),
            };
            println!("#{index} at {pos}: {count} elements of {size} bytes{words}");
        } else {
            println!("vector\t{index}\t{pos}\t{count}\t{size}");
        }
        for row in 0..count.min(args.rows) {
            let element = &bytes[pos + 8 + row * size..pos + 8 + (row + 1) * size];
            if opts.human {
                let words: Vec<String> = if size % 4 == 0 {
                    element.chunks_exact(4).map(|w| describe_word(word(w), strings.as_ref())).collect()
                } else {
                    element.iter().map(|byte| format!("{byte:02x}")).collect()
                };
                println!("  [{row}] {}", words.join("  "));
            } else {
                let words: Vec<String> = if size % 4 == 0 {
                    element.chunks_exact(4).map(|w| format!("{:08x}", word(w))).collect()
                } else {
                    vec![element.iter().map(|byte| format!("{byte:02x}")).collect()]
                };
                println!("element\t{index}\t{row}\t{}", words.join("\t"));
            }
        }
        pos += 8 + size * count;
        index += 1;
    }

    if pos < bytes.len() {
        if opts.human {
            println!("{} bytes left that aren't a vector, from {pos}", bytes.len() - pos);
        } else {
            println!("rest\t{pos}\t{}", bytes.len() - pos);
        }
    }

    Ok(())

}

/// Machine readable, a model per line: its index, how many times it is placed, its
/// primitives, its collision and its material, `-` for none, then its bounds' minimum and
/// maximum coordinates.
fn cmd_models(opts: &CliOptions, space: &mut Bin) -> CliResult<()> {

    let strings = strings(space)?;
    let models: BSMO = space.read_section().map_err(|e| format!("Failed to read the static models: {e}"))?;
    let instances: BSMI = space.read_section().map_err(|e| format!("Failed to read the static model instances: {e}"))?;

    let mut placed = HashMap::<u32, usize>::new();
    for instance in &instances.instances {
        *placed.entry(instance.model).or_default() += 1;
    }


    if !opts.human {
        for (index, model) in models.models.iter().enumerate() {
            let [min, max] = model.bounds.map(|v| v.to_array());
            let (havok, material) = match model.havok_fnv {
                Some(havok) => (name(&strings, havok), name(&strings, model.material_fnv)),
                None => ("-".to_string(), "-".to_string()),
            };
            println!("{index}\t{}\t{}\t{havok}\t{material}\t{}\t{}\t{}\t{}\t{}\t{}",
                placed.get(&(index as u32)).copied().unwrap_or(0), name(&strings, model.primitives_fnv),
                min[0], min[1], min[2], max[0], max[1], max[2]);
        }
        return Ok(());
    }

    let colliding = models.models.iter().filter(|model| model.havok_fnv.is_some()).count();
    println!("{} models, {} with collision, placed {} times", models.models.len(), colliding, instances.instances.len());

    for (index, model) in models.models.iter().enumerate() {
        let [min, max] = model.bounds;
        println!();
        println!("#{index} placed {}×: {}", placed.get(&(index as u32)).copied().unwrap_or(0), name(&strings, model.primitives_fnv));
        match model.havok_fnv {
            Some(havok) => println!("    collision {}, material {}", name(&strings, havok), name(&strings, model.material_fnv)),
            None => println!("    no collision"),
        }
        println!("    bounds {} to {}", point(min.to_array()), point(max.to_array()));
    }

    Ok(())

}

/// A compiled space read whole from the input.
type Bin = SpaceBin<Cursor<Vec<u8>>>;

fn strings(space: &mut Bin) -> CliResult<BWST> {
    space.read_section().map_err(|e| format!("Failed to read the string table: {e}"))
}

/// A 32-bit word as what it most likely is: the string it is the hash of, -1, a small
/// integer, a float, else its hexadecimal.
fn describe_word(word: u32, strings: Option<&BWST>) -> String {
    if let Some(string) = strings.and_then(|strings| strings.get_string(word)) {
        return format!("\"{string}\"");
    }
    let float = f32::from_bits(word);
    if word == u32::MAX {
        "-1".to_string()
    } else if word < 1_000_000 {
        word.to_string()
    } else if float.is_finite() && (1e-4..1e7).contains(&float.abs()) {
        let float = format!("{float:.4}");
        float.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        format!("{word:#010x}")
    }
}

/// A little-endian word.
fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

/// The string of a hash, `?` if it has none.
fn name(strings: &BWST, hash: u32) -> String {
    strings.get_string(hash).unwrap_or("?").to_string()
}

/// A point with 2 decimals.
fn point([x, y, z]: [f32; 3]) -> String {
    format!("({x:.2}, {y:.2}, {z:.2})")
}

/// A size in bytes, kilobytes or megabytes.
fn kilobytes(size: usize) -> String {
    match size {
        0..1024 => format!("{size} B"),
        1024..1048576 => format!("{:.1} kB", size as f32 / 1024.0),
        _ => format!("{:.1} MB", size as f32 / 1048576.0),
    }
}

fn id_str(id: &[u8; 4]) -> String {
    id.iter().map(|&c| c as char).collect()
}
