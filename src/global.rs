use delegate::delegate;
use rand::seq::IteratorRandom;
use rmp_serde::{Deserializer, Serializer};
use rusoto_core::ByteStream;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::runtime::Runtime;

use crate::{
    bucket::Bucket,
    inodes::directory::Directory,
    s3::s3::{download_file, list_files_in_bucket, upload_file, S3Type},
    services::service::{Service, ServiceType},
};

pub type Descriptor = Vec<u8>;

#[derive(Deserialize, Debug)]
pub struct Global {
    buckets: HashMap<String, Bucket>,

    #[serde(default = "default_direct_block_count")]
    pub direct_block_count: usize,

    #[serde(default = "default_root_path")]
    root_path: String,

    #[serde(default)]
    services: Vec<ServiceType>,

    s3: Option<S3Type>,
}

pub trait GlobalTrait {
    fn get_bucket(&self, name: &str) -> Option<&Bucket>;
    fn next_bucket(&self, max_size: usize, exclude: &[String]) -> Option<&String>;
    fn list_buckets(&self) -> Vec<&String>;
    fn random_bucket(&self) -> Option<&String>;
    fn get_direct_block_count(&self) -> usize;
}

#[derive(Debug)]
enum GetS3RootError {
    CorruptedRoot(String),
    DownloadFailed(String),
    MissingRoot,
    CantListBucketContent(String),
    NoS3Config,
}

const fn default_direct_block_count() -> usize {
    10
}
fn default_root_path() -> String {
    "./root.dat".to_string()
}
fn s3_root_file() -> String {
    "chunkdrive-root.dat".to_string()
}

pub fn run_services(global: Arc<AsyncGlobal>) {
    for service in global.0.services.iter() {
        service.run(global.clone());
    }
}

#[derive(Debug)]
pub struct AsyncGlobal(Global);
#[derive(Debug)]
pub struct BlockingGlobal(Global);

impl GlobalTrait for Global {
    fn get_bucket(&self, name: &str) -> Option<&Bucket> {
        self.buckets.get(name)
    }

    fn next_bucket(&self, max_size: usize, exclude: &[String]) -> Option<&String> {
        self.buckets
            .iter()
            .filter(|(_, bucket)| bucket.max_size() >= max_size)
            .filter(|(bucket, _)| !exclude.contains(bucket))
            .choose(&mut rand::thread_rng())
            .map(|(bucket, _)| bucket)
    }

    fn list_buckets(&self) -> Vec<&String> {
        self.buckets.keys().collect()
    }

    fn random_bucket(&self) -> Option<&String> {
        self.buckets
            .iter()
            .choose(&mut rand::thread_rng())
            .map(|(bucket, _)| bucket)
    }

    fn get_direct_block_count(&self) -> usize {
        self.direct_block_count
    }
}

async fn save_s3_root(_s3: &Option<S3Type>, root: &Directory) {
    if let Some(s3) = _s3 {
        let mut buf = Vec::new();
        let mut serializer = Serializer::new(&mut buf).with_struct_map(); // https://github.com/3Hren/msgpack-rust/issues/318
        root.serialize(&mut serializer).unwrap();
        match upload_file(s3, s3_root_file().as_str(), ByteStream::from(buf)).await {
            Ok(_) => println!("root uploaded to s3 !"),
            Err(err) => println!("failed to upload root to s3: {}", err),
        }
    } else {
        eprintln!("No s3, can't save s3 root")
    }
}

async fn get_s3_root(s3: &Option<S3Type>) -> Result<Directory, GetS3RootError> {
    match s3 {
        Some(s3) => {
            let files = list_files_in_bucket(&s3).await;
            match files {
                Ok(files) => {
                    let root_file = files.iter().find(|f| f.to_string() == s3_root_file());
                    match root_file {
                        Some(f) => match download_file(&s3, f).await {
                            Ok(stream) => {
                                let res = tokio::task::spawn_blocking(|| {
                                    let mut de = Deserializer::new(stream.into_blocking_read());
                                    let res: Result<Directory, rmp_serde::decode::Error> =
                                        Deserialize::deserialize(&mut de);
                                    res.map_err(|err| {
                                        format!("deserialize error: {}", err.to_string())
                                    })
                                })
                                .await;
                                match res {
                                    Ok(v) => v.map_err(|err| {
                                        GetS3RootError::CorruptedRoot(err.to_string())
                                    }),
                                    Err(err) => Err(GetS3RootError::CorruptedRoot(err.to_string())),
                                }
                            }
                            Err(err) => Err(GetS3RootError::DownloadFailed(err.to_string())),
                        },
                        None => Err(GetS3RootError::MissingRoot),
                    }
                }
                Err(err) => Err(GetS3RootError::CantListBucketContent(err.to_string())),
            }
        }
        None => Err(GetS3RootError::NoS3Config),
    }
}

impl AsyncGlobal {
    pub fn new(global: Global) -> Self {
        AsyncGlobal(global)
    }
    pub async fn get_root(&self) -> Directory {
        let mut should_save_to_s3 = false;
        match get_s3_root(&self.0.s3).await {
            Ok(root) => {
                println!("async got root from s3 !");
                return root;
            }
            Err(err) => match err {
                GetS3RootError::MissingRoot => {
                    println!("async can't get missing root, will try to save it...");
                    should_save_to_s3 = true;
                }
                _ => {
                    println!("async could not get root from s3: {:?}", err);
                }
            },
        }
        match std::fs::File::open(&self.0.root_path) {
            Ok(file) => {
                let mut de = Deserializer::new(&file);
                match Deserialize::deserialize(&mut de) {
                    Ok(root) => {
                        if should_save_to_s3 {
                            println!("async no root in s3, saving current...");
                            save_s3_root(&self.0.s3, &root).await;
                        }
                        root
                    }
                    Err(_) => {
                        println!("async failed to deserialize local root");
                        std::fs::remove_file(&self.0.root_path).unwrap();
                        Directory::new()
                    }
                }
            }
            Err(_) => {
                println!("async failed to open local root");
                Directory::new()
            }
        }
    }

    pub async fn save_root(&self, root: &Directory) {
        let mut file = std::fs::File::create(&self.0.root_path).unwrap();
        let mut serializer = Serializer::new(&mut file).with_struct_map(); // https://github.com/3Hren/msgpack-rust/issues/318
        root.serialize(&mut serializer).unwrap();
        save_s3_root(&self.0.s3, root).await;
    }
}

impl BlockingGlobal {
    pub fn new(global: Global) -> Self {
        BlockingGlobal(global)
    }
    pub fn get_root(&self) -> Directory {
        let mut should_save_to_s3 = false;
        let rt = Runtime::new().unwrap();
        match rt.block_on(async { get_s3_root(&self.0.s3).await }) {
            Ok(root) => {
                println!("blocking got root from s3 !");
                return root;
            }
            Err(err) => match err {
                GetS3RootError::MissingRoot => {
                    println!("blocking can't get missing root, will try to save it...");
                    should_save_to_s3 = true;
                }
                _ => {
                    println!("blocking could not get root from s3: {:?}", err);
                }
            },
        }
        match std::fs::File::open(&self.0.root_path) {
            Ok(file) => {
                let mut de = Deserializer::new(&file);
                match Deserialize::deserialize(&mut de) {
                    Ok(root) => {
                        if should_save_to_s3 {
                            println!("blocking no root in s3, saving current...");
                            let rt = Runtime::new().unwrap();
                            rt.block_on(async {
                                save_s3_root(&self.0.s3, &root).await;
                            })
                        }
                        root
                    }
                    Err(_) => {
                        std::fs::remove_file(&self.0.root_path).unwrap();
                        Directory::new()
                    }
                }
            }
            Err(_) => Directory::new(),
        }
    }
    pub fn save_root(&self, root: &Directory) {
        let mut file = std::fs::File::create(&self.0.root_path).unwrap();
        let mut serializer = Serializer::new(&mut file).with_struct_map(); // https://github.com/3Hren/msgpack-rust/issues/318
        root.serialize(&mut serializer).unwrap();
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            save_s3_root(&self.0.s3, root).await;
        })
    }
}

impl GlobalTrait for BlockingGlobal {
    delegate! {
        to self.0 {
            fn get_bucket(&self, name: &str) -> Option<&Bucket>;
            fn next_bucket(&self, max_size: usize, exclude: &[String]) -> Option<&String>;
            fn list_buckets(&self) -> Vec<&String>;
            fn random_bucket(&self) -> Option<&String>;
            fn get_direct_block_count(&self) -> usize;
        }
    }
}

impl GlobalTrait for AsyncGlobal {
    delegate! {
        to self.0 {
            fn get_bucket(&self, name: &str) -> Option<&Bucket>;
            fn next_bucket(&self, max_size: usize, exclude: &[String]) -> Option<&String>;
            fn list_buckets(&self) -> Vec<&String>;
            fn random_bucket(&self) -> Option<&String>;
            fn get_direct_block_count(&self) -> usize;
        }
    }
}
