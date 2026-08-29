#[cfg(all(windows, feature = "dokan"))]
mod dokan;
#[cfg(all(unix, feature = "fuse"))]
mod fuse;

use std::io::{self, Write};
use std::path::PathBuf;
use std::fs::File;

use wgtk::res::fs::{ResFilesystem, ResReadDir, ResReadFile};
use wgtk::util::SizeFmt;

use crate::{CliOptions, CliResult, ResArgs, ResCommand, ResCopyArgs, ResListArgs, ResReadArgs};


/// Entrypoint.
pub fn cmd_res(opts: CliOptions, args: ResArgs) -> CliResult<()> {

    let fs = ResFilesystem::new(args.dir)
        .map_err(|e| format!("Failed to open resource filesystem, reason: {e}"))?;

    match args.cmd {
        ResCommand::List(args) => cmd_res_list(opts, args, &fs),
        ResCommand::Read(args) => cmd_res_read(opts, args, &fs),
        ResCommand::Copy(args) => cmd_res_copy(opts, args, &fs),
        #[cfg(all(windows, feature = "dokan"))]
        ResCommand::Dokan(args) => dokan::cmd_res_dokan(opts, args, &fs),
        #[cfg(all(unix, feature = "fuse"))]
        ResCommand::Fuse(args) => fuse::cmd_res_fuse(opts, args, &fs),
    }

}

fn cmd_res_list(opts: CliOptions, args: ResListArgs, fs: &ResFilesystem) -> CliResult<()> {
    
    let path = args.path.as_str();
    let recurse = args.recurse.unwrap_or(Some(0)).unwrap_or(u16::MAX);

    let mut indent = String::new();
    let mut output = io::stdout().lock();

    print_dir(&mut output, fs, &mut indent, path, recurse, opts.human)
        .map_err(|e| format!("Can't find '{path}' resource directory, reason: {e}"))?;

    Ok(())

}

fn cmd_res_read(opts: CliOptions, args: ResReadArgs, fs: &ResFilesystem) -> CliResult<()> {

    let path = args.path.as_str();

    if opts.human {
        print!("Opening filesystem...\r");
        let _ = io::stdout().flush();
    }

    let mut read_file = fs.read(path)
        .map_err(|e| format!("Can't find '{path}' resource file, reason: {e}"))?;

    if opts.human {
        print!("                     \r");
    }

    io::copy(&mut read_file, &mut io::stdout().lock())
        .map_err(|e| format!("Failed to print file content to stdout, reason: {e}"))?;

    Ok(())

}

fn cmd_res_copy(_opts: CliOptions, args: ResCopyArgs, fs: &ResFilesystem) -> CliResult<()> {

    if !args.dest.is_dir() {
        return Err(format!("Destination directory {:?} does not exists.", args.dest));
    }

    // Internal function to copy a single file from its reader to destination path.
    // Source path is only used for printing.
    fn copy_file(mut read_file: ResReadFile, dest_path: PathBuf, source: &str) -> CliResult<()> {

        println!("{source}...");

        let mut dest_file = File::create(&dest_path)
            .map_err(|e| format!("Failed to create file to copy at {dest_path:?}, reason: {e}"))?;

        io::copy(&mut read_file, &mut dest_file)
            .map_err(|e| format!("Failed to copy file from '{source}' to {dest_path:?}, reason: {e}"))?;

        Ok(())

    }

    // Internal function to recursively copy a directory. Source path should not have
    // a trailing separator.
    fn copy_dir(fs: &ResFilesystem, read_dir: ResReadDir, source: &mut String, dest_path: PathBuf) -> CliResult<()> {

        println!("{source}/...");

        match std::fs::create_dir(&dest_path) {
            Ok(()) => {}
            Err(_) if dest_path.is_dir() => {} // Ignore if directory already exists.
            Err(e) => return Err(format!("Failed to create directory to copy in {dest_path:?}, reason: {e}")),
        }

        for entry in read_dir {

            let entry = entry.map_err(|e| format!("Failed to read entry, reason: {e}"))?;
            let entry_dest_path = dest_path.join(entry.name());
            
            let source_backup_len = source.len();
            source.push('/');
            source.push_str(entry.name());

            if entry.stat().is_dir() {
                
                let read_dir = fs.read_dir(&source)
                    .map_err(|e| format!("Failed to read directory entry '{source}', reason: {e}"))?;

                copy_dir(fs, read_dir, source, entry_dest_path)?;

            } else {

                let read_file = fs.read(&source)
                    .map_err(|e| format!("Failed to read a directory entry '{source}', reason: {e}"))?;

                copy_file(read_file, entry_dest_path, &source)?;

            }

            source.truncate(source_backup_len);

        }

        Ok(())

    }

    for source in args.source {

        // Extract the file name from the path, used if successfully copying.
        let file_name = source
            .strip_suffix('/').unwrap_or(&source)
            .rsplit_once('/').map(|(_, s)| s).unwrap_or(&source);

        let dest_path = args.dest.join(file_name);

        // Start by trying the path as a file (it will instantly fail if there is a 
        // leading or trailing separator anyway).
        if let Ok(read_file) = fs.read(&source) {
            copy_file(read_file, dest_path, &source)?;
            continue;
        }
        
        // The error here is generic because we don't know the expected type of entry.
        let read_dir = fs.read_dir(&source)
            .map_err(|e| format!("Can't find '{source}' resource file or directory to copy, reason: {e}"))?;

        // Make source mutable because we'll use it to print advancement and we want to
        // avoid string reallocation in loop...
        let mut source = source;
        if source.ends_with('/') {
            source.truncate(source.len() - 1);
        }

        copy_dir(fs, read_dir, &mut source, dest_path)?;

    }

    Ok(())

}

/// Print directory content
fn print_dir(output: &mut impl Write, fs: &ResFilesystem, indent: &mut String, dir_path: &str, recursion: u16, human: bool) -> io::Result<()> {

    if human && indent.is_empty() {
        let _ = write!(output, "Opening filesystem...\r");
        let _ = io::stdout().flush();
    }

    let mut list = fs.read_dir(dir_path)?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();

    if human && indent.is_empty() {
        let _ = write!(output, "                     \r");
    }

    list.sort_by_cached_key(|e| e.name().to_lowercase());

    let max_size;
    if human {
        max_size = list.iter()
            .map(|entry| entry.name().len())
            .max()
            .unwrap_or(0);
    } else {
        max_size = 0;
    }

    for entry in list {

        let entry_path = entry.path();

        if entry.stat().is_dir() {
            let _ = writeln!(output, "{indent}{}/", entry.name());
        } else if human { 
            let _ = writeln!(output, "{indent}{:<2$}  {}", entry.name(), SizeFmt(entry.stat().size()), max_size);
        } else {
            let _ = writeln!(output, "{indent}{} {}", entry.name(), entry.stat().size());
        }

        if recursion > 0 {
            indent.push_str("  ");
            let _ = print_dir(output, fs, indent, &entry_path, recursion - 1, human);
            indent.truncate(indent.len() - 2);
        }

    }

    Ok(())

}
