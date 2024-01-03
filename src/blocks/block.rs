/*
   This module contains the logic for the block.
   Blocks split the data into chunks and store each chunk in a random bucket or buckets (depending on the redundancy).
   The block should know how to reassemble the chunks into the original data.
   The block should be also able to detect and repair missing chunks if redundancy is enabled.
   The block should also know which range of bytes it contains.
*/

use std::{ops::Range, sync::Arc};

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use super::{direct_block::DirectBlock, indirect_block::IndirectBlock, stored_block::StoredBlock};
use crate::global::GlobalTrait;

#[async_trait]
pub trait Block {
    async fn range<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &self,
        global: Arc<U>,
    ) -> Result<Range<usize>, String>;
    fn get<'a, U: GlobalTrait + std::marker::Send + std::marker::Sync + 'a>(
        &'a self,
        global: Arc<U>,
        range: Range<usize>,
    ) -> BoxStream<'a, Result<Vec<u8>, String>>;
    async fn put<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &mut self,
        global: Arc<U>,
        data: Vec<u8>,
        range: Range<usize>,
    ) -> Result<(), String>;
    async fn delete<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &self,
        global: Arc<U>,
    ) -> Result<(), String>;
    async fn create<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        global: Arc<U>,
        data: Vec<u8>,
        start: usize,
    ) -> Result<BlockType, String>;
    fn to_enum(self) -> BlockType;
}

#[derive(Debug, Serialize, Deserialize)]
pub enum BlockType {
    #[serde(rename = "d")]
    Direct(DirectBlock),
    #[serde(rename = "i")]
    Indirect(IndirectBlock),
    #[serde(rename = "s")]
    Stored(StoredBlock),
} // we use short names to reduce the size of the serialized data while allowing backwards compatibility

macro_rules! match_method {
    ($self:ident, $method:ident, $($arg:expr),*) => {
        match $self {
            BlockType::Direct(block) => block.$method($($arg),*),
            BlockType::Indirect(block) => block.$method($($arg),*),
            BlockType::Stored(block) => block.$method($($arg),*),
        }
    };
}

#[async_trait]
impl Block for BlockType {
    async fn range<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &self,
        global: Arc<U>,
    ) -> Result<Range<usize>, String> {
        match_method!(self, range, global).await
    }

    fn get<'a, U: GlobalTrait + std::marker::Send + std::marker::Sync + 'a>(
        &'a self,
        global: Arc<U>,
        range: Range<usize>,
    ) -> BoxStream<'a, Result<Vec<u8>, String>> {
        match_method!(self, get, global, range)
    }

    async fn put<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &mut self,
        global: Arc<U>,
        data: Vec<u8>,
        range: Range<usize>,
    ) -> Result<(), String> {
        match_method!(self, put, global, data, range).await
    }

    async fn delete<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &self,
        global: Arc<U>,
    ) -> Result<(), String> {
        match_method!(self, delete, global).await
    }

    async fn create<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        global: Arc<U>,
        data: Vec<u8>,
        start: usize,
    ) -> Result<BlockType, String> {
        IndirectBlock::create(global, data, start).await // we use indirect blocks, because they will fit any data size
    }

    fn to_enum(self) -> BlockType {
        self
    }
}
