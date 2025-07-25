/*
   This file implements the basics of the inode system.
   Inodes are the core of the filesystem, they are the files and directories.
*/

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::global::GlobalTrait;

use super::{directory::Directory, file::File, metadata::Metadata};

#[async_trait]
pub trait Inode {
    async fn metadata(&self) -> &Metadata;
    async fn delete<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &mut self,
        global: Arc<U>,
    ) -> Result<(), String>;
}

#[derive(Debug, Serialize, Deserialize)]
pub enum InodeType {
    #[serde(rename = "f")]
    File(File),
    #[serde(rename = "d")]
    Directory(Directory),
}

macro_rules! match_method {
    ($self:ident, $method:ident, $($arg:expr),*) => {
        match $self {
            InodeType::File(inode) => inode.$method($($arg),*),
            InodeType::Directory(inode) => inode.$method($($arg),*),
        }
    };
}

#[async_trait]
impl Inode for InodeType {
    async fn metadata(&self) -> &Metadata {
        match_method!(self, metadata,).await
    }

    async fn delete<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &mut self,
        global: Arc<U>,
    ) -> Result<(), String> {
        match_method!(self, delete, global).await
    }
}
