use crate::global::GlobalTrait;
use futures::StreamExt;
use liner::{Completer, Context};
use walkdir::WalkDir;
use indicatif::{ProgressBar, ProgressStyle, ProgressIterator};
use std::{
    io::{BufReader, Read, Write},
    sync::Arc, cmp::Ordering,
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
) -> Result<Directory, String> {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let stored: Result<&Stored, String> = parent_dir.add(global.clone(), &new_dir.to_string(), Directory::new().to_enum()).await;
        match stored {
            Ok(s) => {
                let inode: InodeType = s.get(global.clone()).await?;
                match inode {
                    InodeType::File(_) => {
                        Err("impossible, directory turned into a file !".to_string())
                    },
                    InodeType::Directory(dir) => Ok(dir)
                }
            },
            Err(err) => Err(err)
        }
    })
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

fn stored_to_dir(global: &Arc<BlockingGlobal>, stored: &Stored) -> Result<Directory, String> {
    let rt = Runtime::new().unwrap();
    let inode: InodeType = rt.block_on(stored.get(global.clone()))?;
    match inode {
        InodeType::Directory(dir) => Ok(dir),
        InodeType::File(_) => {
            Err("Can't convert Stored to dir, it's a file.".to_string())
        }
    }
}

fn directory_of_rel_fs_path(
    global: &Arc<BlockingGlobal>,
    root_path: &std::path::Path,
    root_dir: DirectoryOrStored,
    entry_path: &std::path::Path,
) -> Result<Stored, String> {
    if !entry_path.starts_with(root_path) {
        Err(format!("Path {} does not come from directory {}", entry_path.to_string_lossy(), root_path.to_string_lossy()))
    } else {
        let subentries: Vec<String> = entry_path.strip_prefix(root_path)
                .iter().map(|x| x.to_string_lossy().to_string()).collect();
        if root_path == entry_path {
            return Err(format!("both path {} and {} seem to be equal !", root_path.to_string_lossy(), entry_path.to_string_lossy()));
        }
        let rt = Runtime::new().unwrap();
        let mut cur_dir: DirectoryOrStored = root_dir;
        println!("dorfsp({}, {})", root_path.to_string_lossy(), entry_path.to_string_lossy());
        for subentry in &subentries[..subentries.len() - 1] {
            let real_cur_dir: Directory  = match cur_dir {
                DirectoryOrStored::Dir(dir) => dir,
                DirectoryOrStored::Stored(stored) => stored_to_dir(global, &stored)?
            };
            let sub_stored: &Stored = real_cur_dir.get(&subentry.to_string())?;
            let inode: InodeType = rt.block_on(sub_stored.get(global.clone())).map_err(|err|{
                format!("can't get {} from cur_dir: {}", subentry, err)
            })?;
            match inode {
                InodeType::Directory(_) => { cur_dir = DirectoryOrStored::Stored(sub_stored.clone()); },
                InodeType::File(_) => {
                    return Err(format!("{} in {} is a file not a directory", subentry, entry_path.to_string_lossy()))
                }
            }
        }
        match cur_dir {
            DirectoryOrStored::Stored(stored) => Ok(stored.clone()),
            DirectoryOrStored::Dir(_) => Err("Impossible, cur_dir is Directory not Stored".to_string())
        }
    }
}

#[derive(Clone)]
enum DirectoryOrStored {
    Dir(Directory),
    Stored(Stored),
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
    let count = WalkDir::new(std::path::Path::new(shellexpand::tilde(args[0].as_str()).as_ref()))
        .into_iter().count();
    let pb = ProgressBar::new(count as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {msg} ({pos}/{len}, ETA {eta})",
        )
        .unwrap(),
    ); 
    let (root_parent, parent_is_root): (DirectoryOrStored, bool) = if cwd.is_empty() {
        (DirectoryOrStored::Dir(global.get_root()), true)
    } else {
        (DirectoryOrStored::Stored(cwd.last().unwrap().clone()), false)
    };
    let mut cur_dir: DirectoryOrStored = root_parent.clone();
    let mut failed = Vec::new();
    let tmp_str: String = shellexpand::tilde(args[0].as_str()).as_ref().to_string();
    let tree_root_path = std::path::Path::new(tmp_str.as_str());
    for entry in WalkDir::new(tree_root_path)
        .sort_by(|a,b| {
            if a.file_type().is_dir() && !b.file_type().is_dir() {
                Ordering::Less
            } else {
                a.file_name().cmp(b.file_name())
            }
        })
        .into_iter().filter_map(|e| e.ok()).progress_with(pb.clone()) {
        if entry.path() == tree_root_path {
            continue;
        }
        match directory_of_rel_fs_path(global, tree_root_path, root_parent.clone(), entry.path()) {
            Ok(dir) => { cur_dir = DirectoryOrStored::Stored(dir); }
            Err(err) => {
                failed.push((entry.clone(), format!("failed to find folder of {} in root: {}", entry.path().to_string_lossy(), err)));
                continue;
            }
        }
        match entry.path().file_name() {
            Some(file_name) => {
                pb.set_message(file_name.to_string_lossy().to_string());
                let mut real_cur_dir: Directory  = match cur_dir {
                    DirectoryOrStored::Dir(ref dir) => dir.clone(),
                    DirectoryOrStored::Stored(ref stored) => stored_to_dir(global, &stored)?
                };
                if entry.file_type().is_dir() {
                    mkdir_in_dir(global, &mut real_cur_dir, file_name.to_string_lossy().as_ref()).err().and_then(|err|{
                        failed.push((entry, format!("failed to add dir to root: {}", err)));
                        Some(()) 
                    });
                    let rt = Runtime::new().unwrap();
                    match cur_dir {
                        DirectoryOrStored::Dir(_) => {},
                        DirectoryOrStored::Stored(stored) => {
                            rt.block_on(async { stored.put(global.clone(), real_cur_dir.to_enum()).await })?;
                        }
                    }
                } else if entry.file_type().is_file() {
                    upload_to_dir(global, entry.path().to_string_lossy().as_ref(), &mut real_cur_dir)?;
                    match cur_dir {
                        DirectoryOrStored::Dir(_) => { },
                        DirectoryOrStored::Stored(stored) => {
                            let rt = Runtime::new().unwrap();
                            rt.block_on(async { stored.put(global.clone(), real_cur_dir.to_enum()).await })?;
                        }
                    }
                }
            }
            None => { failed.push((entry.clone(), "invalid file name".to_string())) }
        }
    }
    if failed.len() > 0 {
        println!("Failed to upload: ");
    }
    for (entry, err) in failed {
       println!("{} -> {}", entry.path().to_string_lossy(), err); 
    }
    
    if parent_is_root {
        match root_parent {
            DirectoryOrStored::Dir(dir) => {
                global.save_root(&dir);
            },
            DirectoryOrStored::Stored(_) => { }
        }
    }

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
