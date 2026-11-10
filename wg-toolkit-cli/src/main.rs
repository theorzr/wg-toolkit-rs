//! The CLI for wg-toolkit library.

mod pxml;
mod res;
mod space;
mod havok;

use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};


/// Global options for the command line interface.
#[derive(Debug)]
pub struct CliOptions {
    /// Human readable mode enabled.
    pub human: bool,
}

/// Command line utility for interacting with codecs distributed by Wargaming.net studio.
/// 
/// This command line tries to provide UNIX-oriented commands that can be piped together
/// to make more complex operations.
#[derive(Debug, Parser)]
#[command(version, author, disable_help_subcommand = true, max_term_width = 100)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
    /// Optionally force human readable mode or not.
    /// 
    /// This is automatically enabled if stdout is a terminal, so it's automatically
    /// disabled when piping in your shell to a file or another program. This make 
    /// interoperability with UNIX like programs easier. Human readable output cannot
    /// be easily parsed!
    #[arg(short = 'H', long)]
    pub human: Option<bool>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(name = "pxml")]
    PackedXml(PackedXmlArgs),
    Res(ResArgs),
    Space(SpaceArgs),
    Havok(HavokArgs),
}

/// Packed XML read and write utilities.
/// 
/// This is a format that is commonly used in game resources, it provides a kind of 
/// serialization for a XML, packed XML files use the same extension as regular XML 
/// (.xml), a packed XML file can be replaced by a clear XML file and will work the
/// same.
#[derive(Debug, Args)]
pub struct PackedXmlArgs {
    /// If specified, the packed XML is read from a file instead of stdin (fd 0).
    /// 
    /// This is essentially the same as piping 'cat' of the packed XML file into it. 
    #[arg(short, long)]
    pub file: Option<PathBuf>,
    /// Enable XML output style.
    /// 
    /// This can be used if you want to replace a packed XML file with a clear one, 
    /// this flag will correctly format the packed XML as a regular clear XML file
    /// that can be read by the game engine.
    #[arg(short, long, conflicts_with = "raw")]
    pub xml: bool,
    /// Enable raw output style, outputting the binary encoded element.
    #[arg(short, long, conflicts_with = "xml")]
    pub raw: bool,
    /// If needed, the packed XML can be modified before outputting it.
    ///
    /// The filter is a ';'-separated sequence of assignments 'path=value', applied in
    /// order. A value is either another path, or a constructor: str(..), int(..),
    /// true(), false().
    ///
    /// A path navigates children with 'key/key2/...'. Append '[N]' to select the N-th
    /// occurrence of that key (negative counts from the end), or '[^]' to create and
    /// append a new child ('[^N]' to insert at index N instead). Use '^' alone to
    /// target the element's own value instead of a child. A path starting with
    /// '$name' refers to a temporary variable rather than the root element, e.g.
    /// '$tmp=login/host;$tmp/name=str(hello)'.
    pub filter: Option<String>,
}

/// Game resources virtual filesystem access (readonly).
/// 
/// The game resources are split in many directories under the game's resources (res/)
/// directory, most of resources are actually stored inside huge package files (.pkg).
/// This command uses efficient indexing on these packages to efficiently fetch and
/// interact with the files, this works with not-packaged files and packaged files at
/// the same time.
#[derive(Debug, Args)]
pub struct ResArgs {
    /// Path to the game's resource (res/) directory.
    pub dir: PathBuf,
    #[command(subcommand)]
    pub cmd: ResCommand
}

#[derive(Debug, Subcommand)]
pub enum ResCommand {
    Read(ResReadArgs),
    #[command(name = "ls")]
    List(ResListArgs),
    #[command(name = "cp")]
    Copy(ResCopyArgs),
    #[cfg(all(windows, feature = "dokan"))]
    Dokan(ResDokanArgs),
    #[cfg(all(unix, feature = "fuse"))]
    Fuse(ResFuseArgs),
}

/// Read a file and write its content on the standard output.
/// 
/// Like 'ls', this command may take some time to complete depending on where the file is
/// located in packages, this command return as soon as possible so you may be lucky if
/// the file is located in first opened packages.
#[derive(Debug, Args)]
pub struct ResReadArgs {
    /// Path to the file to read, no leading separator!
    pub path: String,
}

/// List directory contents with optional recursion.
/// 
/// Note that this function may take a really long time to proceed, because all packages
/// needs to be opened to ensures that the given directory is present or not. This should
/// be faster on subsequent calls because of your operating system filesystem cache.
#[derive(Debug, Args)]
pub struct ResListArgs {
    /// Path to the directory to list, no leading separator (empty to list root)!
    #[arg(default_value = "")]
    pub path: String,
    /// Enable recursion listing of directories.
    /// 
    /// By default this will recurse indefinitely, but you can provide a limit to the 
    /// recursion, for example '1' will show children of all root directories.
    #[arg(short, long)]
    pub recurse: Option<Option<u16>>,
}

/// Copy files and directories from resources.
#[derive(Debug, Args)]
pub struct ResCopyArgs {
    /// Source path of the file or directory to copy from resources.
    /// 
    /// Trailing separator '/' for directories is not necessary, where the file or 
    /// directory is copied is controlled by the destination path.
    #[arg(required = true)]
    pub source: Vec<String>,
    /// Destination directory, in your native filesystem.
    /// 
    /// The destination directory must exists. In general, this will error out if a file 
    /// is copied onto an existing directory, or if a directory is copied onto a existing 
    /// file, or for many other I/O errors.
    pub dest: PathBuf,
}

/// Start a Dokan (filesystem in userspace) that will make the virtual resource filesystem
/// accessible as a regular filesystem to the OS, to later mounted.
#[derive(Debug, Args)]
pub struct ResDokanArgs {
    pub mount_path: String,
}

/// Start a FUSE (filesystem in userspace) that will make the virtual resource filesystem
/// accessible as a regular filesystem to the OS, to later mounted.
#[derive(Debug, Args)]
pub struct ResFuseArgs {
    pub mount_path: String,
}

/// Compiled space (space.bin) inspection, to study its sections.
///
/// A compiled space is a table of sections, each identified by 4 characters, most of
/// them runs of vectors: a vector being its element size, its element count, then its
/// elements. Strings are referenced by their hash in the BWST section.
#[derive(Debug, Args)]
pub struct SpaceArgs {
    /// If specified, the compiled space is read from a file instead of stdin (fd 0),
    /// pipe 'wgtk res <dir> read spaces/<name>/space.bin' into it say.
    #[arg(short, long)]
    pub file: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: SpaceCommand,
}

#[derive(Debug, Subcommand)]
pub enum SpaceCommand {
    /// List its sections: their identifier, offset and length.
    Sections,
    /// List its string table (BWST), each string with its hash.
    Strings,
    /// Walk a section as a run of vectors, the elements of each as 32-bit words, both as
    /// integers and floats, the integers that are string hashes resolved.
    Vectors(SpaceVectorsArgs),
    /// List its static models (BSMO) with their collision, render primitives, bounds and
    /// how many times BSMI places each.
    Models,
}

#[derive(Debug, Args)]
pub struct SpaceVectorsArgs {
    /// The section's identifier, 'BSMO' say.
    pub section: String,
    /// How many elements of each vector to show.
    #[arg(short, long, default_value_t = 3)]
    pub rows: usize,
}

/// Havok tagfile (TAG0) inspection, as the game ships collision models (.havok).
#[derive(Debug, Args)]
pub struct HavokArgs {
    /// If specified, the tagfile is read from a file instead of stdin (fd 0), pipe
    /// 'wgtk res <dir> read <path>.havok' into it say.
    #[arg(short, long)]
    pub file: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: HavokCommand,
}

#[derive(Debug, Subcommand)]
pub enum HavokCommand {
    /// List its types: each as its id, its name with what it points to, and what it is,
    /// then a record's own members, their offset, name and type.
    ///
    /// A pointer, a string or a variable-length array is a 4-byte index of an item, the
    /// object or elements it refers to, whose type and count the items give.
    Types(HavokTypesArgs),
    /// Print its objects from the root, following pointers.
    Tree(HavokTreeArgs),
    /// Print it as a collision model: its bodies, their material kind, collision flags
    /// and triangle count, and its bounds.
    Collision,
}

#[derive(Debug, Args)]
pub struct HavokTypesArgs {
    /// Also list the types only declared, template parameters and opaque types, which
    /// no data has.
    #[arg(short, long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct HavokTreeArgs {
    /// How deep to follow members, pointers and arrays.
    #[arg(short, long, default_value_t = 12)]
    pub depth: usize,
    /// How many elements of each array to show.
    #[arg(short, long, default_value_t = 4)]
    pub elements: usize,
}

/// Type alias for a result that simply returns a string on error, this will be output
/// on stderr and process returns a failed exit code. This allows easier error handling
/// by just mapping the error type to an explanatory text.
pub type CliResult<T> = Result<T, String>;

/// Read a file, or the standard input without one.
pub fn read_input(file: Option<&Path>) -> CliResult<Vec<u8>> {
    match file {
        Some(path) => std::fs::read(path).map_err(|e| format!("Failed to read file at {path:?}: {e}")),
        None => {
            let mut content = Vec::new();
            io::stdin().read_to_end(&mut content).map_err(|e| format!("Failed to read content from stdin: {e}"))?;
            Ok(content)
        }
    }
}

/// Entrypoint.
fn main() -> ExitCode {

    let args = Cli::parse();
    let opts = CliOptions {
        human: args.human.unwrap_or_else(|| io::stdout().is_terminal()),
    };

    let res = match args.cmd {
        Command::PackedXml(args) => pxml::cmd_pxml(args),
        Command::Res(args) => res::cmd_res(opts, args),
        Command::Space(args) => space::cmd_space(opts, args),
        Command::Havok(args) => havok::cmd_havok(opts, args),
    };

    if let Err(message) = res {
        eprintln!("{message}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
    
}
