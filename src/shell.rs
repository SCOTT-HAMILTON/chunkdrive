use crate::global::GlobalTrait;
use futures::StreamExt;
use liner::{Completer, Context};
use walkdir::WalkDir;
use indicatif::{ProgressBar, ProgressStyle};
use std::{
    io::{BufReader, Read, Write},
    sync::Arc,
};
use tokio::runtime::Runtime;

use crate::{
    global::BlockingGlobal,
    inodes::{
        directory::Directory,
        file::File,
        inode::{Inode, InodeType},
        metadata::Metadata,
    },
    stored::Stored,
};

fn tokenize_line(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();

    let mut in_string = false;
    let mut escape = false;

    for c in line.chars() {
        if c == '\n' || c == '\r' {
            continue;
        } else if escape {
            token.push(c);
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else if c == '"' || c == '\'' || c == '`' {
            in_string = !in_string;
        } else if c == ' ' && !in_string {
            if !token.is_empty() {
                tokens.push(token);
                token = String::new();
            }
        } else {
            token.push(c);
        }
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

struct ShellCompleter;

impl Completer for ShellCompleter {
    fn completions(&mut self, _start: &str) -> Vec<String> {
        let tokens = tokenize_line(_start);
        match tokens.len() {
            0 => COMMANDS
                .iter()
                .map(|(name, _, _)| name.to_string())
                .collect(),
            1 => COMMANDS
                .iter()
                .filter(|(name, _, _)| name.starts_with(_start))
                .map(|(name, _, _)| name.to_string())
                .collect(),
            _ => Vec::new(),
        }
    }
}

pub fn shell(global: Arc<BlockingGlobal>) {
    println!(
        "Welcome to the ChunkDrive {} debug shell! Type \"help\" for a list of commands.",
        env!("CARGO_PKG_VERSION")
    );

    let mut path: Vec<String> = Vec::new();
    let mut stored_cwd: Vec<Stored> = Vec::new();
    let mut clipboard: Option<Stored> = None;
    let mut context = Context::new();
    context.key_bindings = liner::KeyBindings::Vi;

    loop {
        let prompt = format!(
            "{}/{}# ",
            match clipboard {
                Some(_) => "📋 ",
                None => "",
            },
            match path.len() {
                0 => String::from(""),
                _ => {
                    format!("{}", path.join("/"))
                },
            }
        );

        let line = match context.read_line(&prompt, None, &mut ShellCompleter) {
            Ok(line) => line,
            Err(_) => break,
        };
        context.history.push(line.clone().into()).unwrap();
        let tokens = tokenize_line(&line);

        if tokens.is_empty() {
            continue;
        }

        let command = tokens[0].as_str();
        let args = tokens[1..].to_vec();

        match COMMANDS.iter().find(|(name, _, _)| *name == command) {
            Some((_, func, _)) => {
                match func(&global, args, &mut path, &mut stored_cwd, &mut clipboard) {
                    Ok(_) => {}
                    Err(e) => {
                        if e == "SIGTERM" {
                            break;
                        }
                        println!("Error: {}", e)
                    }
                }
            }
            None => println!("Unknown command: {}", command),
        }
    }
}

type Command = (
    &'static str,
    fn(
        &Arc<BlockingGlobal>,
        Vec<String>,
        &mut Vec<String>,
        &mut Vec<Stored>,
        &mut Option<Stored>,
    ) -> Result<(), String>,
    &'static str,
);

const COMMANDS: &[Command] = &[
    ("help", help, "Prints this help message."),
    ("exit", exit, "Exits the shell."),
    ("ls", ls, "Lists the contents of the current directory."),
    ("mkdir", mkdir, "Creates a new directory."),
    ("cd", cd, "Changes the current working directory."),
    ("rm", rm, "Removes a file or directory."),
    ("cut", cut, "Cuts a file or directory."),
    ("paste", paste, "Pastes a file or directory."),
    ("up", upload, "Uploads a file to the drive"),
    ("up_tree", upload_tree, "Uploads a tree to the drive"),
    ("down", download, "Downloads a file from the drive."),
    ("stat", stat, "Prints metadata about a file or directory."),
    ("lsbk", bucket_list, "Lists all buckets."),
    ("bktest", bucket_test, "Tests a bucket."),
    ("dbg", dbg, "Prints debug information about an object."),
    (
        "root",
        |_, _, path, cwd, _| {
            path.clear();
            cwd.clear();
            Ok(())
        },
        "Returns to the root directory",
    ),
    (
        "cwd",
        |_, _, path, _, _| Ok(println!("/{}", path.join("/"))),
        "Prints the current working directory.",
    ),
];

fn help(
    _global: &Arc<BlockingGlobal>,
    _args: Vec<String>,
    _path: &mut Vec<String>,
    _cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    println!("Commands:");
    for (name, _, description) in COMMANDS {
        println!("  {:<10} {}", name, description);
    }
    Ok(())
}

fn dbg(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    _path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: dbg <global|.|<path>>".to_string());
    }
    if args[0] == "global" {
        dbg!(global);
        Ok(())
    } else if args[0] == "." {
        let rt = Runtime::new().unwrap();
        if cwd.is_empty() {
            dbg!(global.get_root());
        } else {
            let inode: InodeType = rt.block_on(cwd.last().unwrap().get(global.clone()))?;
            dbg!(inode);
        }
        Ok(())
    } else {
        let rt = Runtime::new().unwrap();
        let dir = match cwd.last() {
            Some(cwd) => {
                let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
                match inode {
                    InodeType::Directory(dir) => dir,
                    _ => Err("Not in a directory.".to_string())?,
                }
            }
            None => global.get_root(),
        };
        let stored = dir.get(&args[0])?;
        let inode: InodeType = rt.block_on(stored.get(global.clone()))?;
        dbg!(inode);
        Ok(())
    }
}

fn ls(
    global: &Arc<BlockingGlobal>,
    _args: Vec<String>,
    _path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    let rt = Runtime::new().unwrap();
    let dir = match cwd.last() {
        Some(cwd) => {
            let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
            match inode {
                InodeType::Directory(dir) => {
                    println!("..");
                    dir
                }
                _ => Err("Not in a directory.".to_string())?,
            }
        }
        None => global.get_root(),
    };

    for name in dir.list() {
        println!("{}", name);
    }
    Ok(())
}


fn mkdir_in_dir(
    global: &Arc<BlockingGlobal>,
    parent_dir: &mut Directory,
    new_dir: &str,
) -> Result<Stored, String> {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        parent_dir.add(global.clone(), &new_dir.to_string(), Directory::new().to_enum()).await
    }).map(|s|s.clone())
}

fn mkdir(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    _path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: mkdir <name>".to_string());
    }
    if cwd.is_empty() {
        // root directory
        let mut root = global.get_root();
        mkdir_in_dir(global, &mut root, &args[0])?;
        global.save_root(&root);
    } else {
        let rt = Runtime::new().unwrap();
        let cwd = cwd.last_mut().unwrap();
        let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
        let mut dir = match inode {
            InodeType::Directory(dir) => dir,
            _ => Err("Not in a directory.".to_string())?,
        };
        mkdir_in_dir(global, &mut dir, &args[0])?;
        rt.block_on(async { cwd.put(global.clone(), dir.to_enum()).await })?;
    }
    Ok(())
}

fn cd(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: cd <path>".to_string());
    }

    if args[0] == ".." {
        if !path.is_empty() {
            path.pop();
        }
        if !cwd.is_empty() {
            cwd.pop();
        }
        return Ok(());
    }

    let rt = Runtime::new().unwrap();
    let dir = match cwd.last() {
        Some(cwd) => {
            let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
            match inode {
                InodeType::Directory(dir) => dir,
                _ => Err("Not in a directory.".to_string())?,
            }
        }
        None => global.get_root(),
    };
    let mut found = false;
    for name in dir.list() {
        if name == args[0] {
            found = true;
            break;
        }
    }
    if !found {
        return Err("No such directory.".to_string());
    }
    path.push(args[0].clone());
    cwd.push(dir.get(&args[0])?.clone());
    Ok(())
}

fn rm(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    _path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: rm <name>".to_string());
    }
    if cwd.is_empty() {
        let rt = Runtime::new().unwrap();
        let mut root = global.get_root();
        let err = rt.block_on(async { root.remove(global.clone(), &args[0]).await });
        global.save_root(&root);
        err?;
    } else {
        let rt = Runtime::new().unwrap();
        let cwd = cwd.last_mut().unwrap();
        let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
        let mut dir = match inode {
            InodeType::Directory(dir) => dir,
            _ => Err("Not in a directory.".to_string())?,
        };
        let err = rt.block_on(async { dir.remove(global.clone(), &args[0]).await });
        rt.block_on(async { cwd.put(global.clone(), dir.to_enum()).await })?;
        err?;
    }
    Ok(())
}

fn cut(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    _path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: cut <name>".to_string());
    }
    if clipboard.is_some() {
        return Err("Clipboard is not empty.".to_string());
    }
    let rt = Runtime::new().unwrap();
    let mut dir = match cwd.last() {
        Some(cwd) => {
            let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
            match inode {
                InodeType::Directory(dir) => dir,
                _ => Err("Not in a directory.".to_string())?,
            }
        }
        None => global.get_root(),
    };
    let stored = dir.unlink(&args[0])?;
    if cwd.is_empty() {
        global.save_root(&dir);
    } else {
        let cwd = cwd.last_mut().unwrap();
        rt.block_on(async { cwd.put(global.clone(), dir.to_enum()).await })?;
    }
    let _ = clipboard.insert(stored);
    Ok(())
}

fn paste(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    _path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: cut <name>".to_string());
    }
    if clipboard.is_none() {
        return Err("Clipboard is empty.".to_string());
    }
    let rt = Runtime::new().unwrap();
    let mut dir = match cwd.last() {
        Some(cwd) => {
            let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
            match inode {
                InodeType::Directory(dir) => dir,
                _ => Err("Not in a directory.".to_string())?,
            }
        }
        None => global.get_root(),
    };

    let stored = clipboard.take().unwrap();
    dir.put(&args[0], stored)?;

    if cwd.is_empty() {
        global.save_root(&dir);
    } else {
        let cwd = cwd.last_mut().unwrap();
        rt.block_on(async { cwd.put(global.clone(), dir.to_enum()).await })?;
    }

    Ok(())
}

fn exit(
    _global: &Arc<BlockingGlobal>,
    _args: Vec<String>,
    _path: &mut Vec<String>,
    _cwd: &mut Vec<Stored>,
    clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if clipboard.is_some() {
        return Err("Clipboard is not empty. Paste it somewhere first.".to_string());
    }

    Err("SIGTERM".to_string())
}

fn stat_format(metadata: &Metadata) -> String {
    let mut s = String::new();
    s.push_str(&format!("Size: {}\n", metadata.size.human()));
    s.push_str(&format!("Created: {}\n", metadata.human_created()));
    s.push_str(&format!("Modified: {}", metadata.human_modified()));
    s
}

fn stat(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    _path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: stat <name|.>".to_string());
    }
    let rt = Runtime::new().unwrap();

    if args[0] == "." {
        if cwd.is_empty() {
            let metadata: Metadata = rt.block_on(async {
                let root = global.get_root();
                root.metadata().await.clone()
            });
            println!("Type: Directory");
            println!("{}", stat_format(&metadata));
        } else {
            let inode: InodeType = rt.block_on(cwd.last().unwrap().get(global.clone()))?;
            let metadata: &Metadata = rt.block_on(inode.metadata());
            println!("Type: Directory");
            println!("{}", stat_format(metadata));
        }
    } else {
        let dir = match cwd.last() {
            Some(cwd) => {
                let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
                match inode {
                    InodeType::Directory(dir) => dir,
                    _ => Err("Not in a directory.".to_string())?,
                }
            }
            None => global.get_root(),
        };
        let stored = dir.get(&args[0])?;
        let inode: InodeType = rt.block_on(stored.get(global.clone()))?;
        let metadata: &Metadata = rt.block_on(inode.metadata());
        match inode {
            InodeType::Directory(_) => println!("Type: Directory"),
            InodeType::File(_) => println!("Type: File"),
        }
        println!("{}", stat_format(metadata));
    }

    Ok(())
}

fn upload(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    _path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: up <file>".to_string());
    }

    match upload_file(global, cwd, args[0].as_str()) {
        Ok(bytes) => {
            println!("Uploaded {} bytes to {}.", bytes, args[0]);
            Ok(())
        },
        Err(err) => Err(err)
    }
}

fn upload_to_dir(
    global: &Arc<BlockingGlobal>,
    file_path: &str,
    parent: &mut Directory,
) -> Result<usize, String> {
    let path = std::path::Path::new(file_path);
    let file_name = path.file_name().ok_or(format!("can't upload {}, it has no filename", file_path))?;
    let file = std::fs::File::open(shellexpand::tilde(file_path).as_ref()).map_err(|_| "Failed to open file.")?;
    let mut reader = BufReader::new(file);
    let mut data = Vec::new();

    reader
        .read_to_end(&mut data)
        .map_err(|_| "Failed to read file.")?;
    
    let size = data.len();

    let rt = Runtime::new().unwrap();
    let file = rt.block_on(File::create(global.clone(), data))?;
    rt.block_on(parent.add(global.clone(), &file_name.to_string_lossy().as_ref().to_string(), file.to_enum()))?;
    Ok(size)
}

fn upload_file(
    global: &Arc<BlockingGlobal>,
    cwd: &mut Vec<Stored>,
    file_path: &str) -> Result<usize, String> {
    let mut dir = match cwd.last() {
        Some(cwd) => {
            let rt = Runtime::new().unwrap();
            let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
            match inode {
                InodeType::Directory(dir) => dir,
                _ => Err("Not in a directory.".to_string())?,
            }
        }
        None => global.get_root(),
    };
    let size = upload_to_dir(global, file_path, &mut dir)?;
    if cwd.is_empty() {
        global.save_root(&dir);
    } else {
        let cwd = cwd.last_mut().unwrap();
        let rt = Runtime::new().unwrap();
        rt.block_on(async { cwd.put(global.clone(), dir.to_enum()).await })?;
    }
    Ok(size)
}

fn upload_tree(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    _path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: up_tree <path/to/directory>".to_string());
    }
    let expanded_path = shellexpand::tilde(&args[0].as_str()).as_ref().to_string();
    let parent_path = std::path::Path::new(&expanded_path);
    let count = WalkDir::new(parent_path).into_iter().count();
    let root_parent: Stored = if cwd.is_empty() {
        Err("can't up_tree directly in root, need to be in subfolder...".to_string())
    } else {
        Ok(cwd.last().unwrap().clone())
    }?;

    let pb = ProgressBar::new(count as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {msg} ({pos}/{len}, ETA {eta})",
        )
        .unwrap(),
    ); 
    fn aux(cwd: &Stored, fs_cwd: &std::path::Path, global: &Arc<BlockingGlobal>,
        pb: &ProgressBar) -> Result<(), String> {
        let entries = std::fs::read_dir(fs_cwd)
            .map_err(|err|{
                err.to_string()
            })?.filter_map(|_entry|
            if let Ok(entry) = _entry {
                let file_name = entry.file_name();
                if file_name != "." && file_name != ".." {
                    Some(entry)
                } else {
                    None
                }
            } else {
                None
            }
        );
        let (directories, files): (Vec<_>, Vec<_>) = entries
            .filter(|entry|{entry.file_type().is_ok()})
            .partition(|entry| entry.file_type().map(|m| m.is_dir()).unwrap_or(false));
        
        let mut parent_dir = {
            let rt = Runtime::new().unwrap();
            let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
            match inode {
                InodeType::Directory(dir) => Ok(dir),
                _ => Err("Not in a directory.".to_string()),
            }
        }?;
        for file in files {
            let file_path = file.path();
            let file_name = file.file_name().to_string_lossy().as_ref().to_string();
            pb.set_message(file_name);
            pb.inc(1);
            upload_to_dir(global, file_path.to_string_lossy().as_ref(), &mut parent_dir)?;
        }
        for dir in directories {
            let dir_path = dir.path();
            let dir_name = dir.file_name().to_string_lossy().as_ref().to_string();
            pb.set_message(dir_name.clone());
            pb.inc(1);
            let new_cwd = mkdir_in_dir(global, &mut parent_dir, dir_name.as_str())?;
            aux(&new_cwd, &dir_path, global, pb)?;
        }
        {
            let rt = Runtime::new().unwrap();
            rt.block_on(async { cwd.put(global.clone(), &parent_dir.to_enum()).await })?;
        }
        Ok(())
    }
    
    aux(&root_parent, parent_path, global, &pb)?;

    Ok(())
}

fn download(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    _path: &mut Vec<String>,
    cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 2 {
        return Err("Usage: down <from> <to>".to_string());
    }

    let rt = Runtime::new().unwrap();
    let dir = match cwd.last() {
        Some(cwd) => {
            let inode: InodeType = rt.block_on(cwd.get(global.clone()))?;
            match inode {
                InodeType::Directory(dir) => dir,
                _ => Err("Not in a directory.".to_string())?,
            }
        }
        None => global.get_root(),
    };

    let stored = dir.get(&args[0])?;
    let inode: InodeType = rt.block_on(stored.get(global.clone()))?;
    let file = match inode {
        InodeType::File(file) => file,
        _ => Err("Not a file.".to_string())?,
    };
    let metadata = rt.block_on(file.metadata());
    println!("Downloading {}...", metadata.size.human());
    let mut buf_writer = std::io::BufWriter::new(
        std::fs::File::create(&args[1]).map_err(|_| "Failed to create file.")?,
    );
    let mut stream = file.get(global.clone());
    while let Some(chunk) = rt.block_on(stream.next()) {
        let slice = chunk.map_err(|_| "Failed to read file.")?;
        buf_writer
            .write_all(&slice)
            .map_err(|_| "Failed to write file.")?;
    }
    println!("Downloaded to {}.", args[1]);

    Ok(())
}

fn bucket_list(
    global: &Arc<BlockingGlobal>,
    _args: Vec<String>,
    _path: &mut Vec<String>,
    _cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    println!(
        "  {:<20} {:<20} {:<20} {}",
        "Name", "Source", "Encryption", "Max block size"
    );
    for bucket in global.list_buckets() {
        let b_type = match global.get_bucket(bucket) {
            Some(bucket) => bucket.human_readable(),
            None => "Missing?".to_string(),
        };
        println!("  {:<20} {}", bucket, b_type);
    }
    Ok(())
}

fn bucket_test(
    global: &Arc<BlockingGlobal>,
    args: Vec<String>,
    _path: &mut Vec<String>,
    _cwd: &mut Vec<Stored>,
    _clipboard: &mut Option<Stored>,
) -> Result<(), String> {
    if args.len() != 1 {
        return Err("Usage: bktest <name>".to_string());
    }
    let bucket = match global.get_bucket(&args[0]) {
        Some(bucket) => bucket,
        None => Err("No such bucket.".to_string())?,
    };

    let block = vec![0; bucket.max_size()];

    let rt = Runtime::new().unwrap();
    let descriptor = rt.block_on(bucket.create())?;
    println!("Created descriptor: {:?}", descriptor);

    rt.block_on(bucket.put(&descriptor, block.clone()))?;
    println!("Put data of size {}.", block.len());

    let retrieved = rt.block_on(bucket.get(&descriptor))?;
    println!("Retrieved data of size {}.", retrieved.len());

    rt.block_on(bucket.delete(&descriptor))?;
    println!("Deleted data.");

    if block != retrieved {
        return Err("Data mismatch.".to_string());
    } else {
        println!("Data matches.");
    }

    let recieved2 = rt.block_on(bucket.get(&descriptor));
    if recieved2.is_ok() {
        return Err("Data still exists.".to_string());
    }
    println!("Deleted data was not found.");

    println!("OK.");

    Ok(())
}
