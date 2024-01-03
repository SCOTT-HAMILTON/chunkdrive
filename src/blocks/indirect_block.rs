use std::{ops::Range, sync::Arc};

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use serde::{Deserialize, Serialize};

use super::{
    block::{Block, BlockType},
    direct_block::DirectBlock,
    stored_block::StoredBlock,
};
use crate::global::GlobalTrait;

#[derive(Debug, Serialize, Deserialize)]
pub struct IndirectBlock {
    #[serde(rename = "b")]
    blocks: Vec<BlockType>, // we will make sure that these are in order
}

#[async_trait]
impl Block for IndirectBlock {
    async fn range<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &self,
        global: Arc<U>,
    ) -> Result<Range<usize>, String> {
        let first = match self.blocks.first() {
            Some(block) => block,
            None => return Ok(0..0),
        };
        let last = match self.blocks.last() {
            Some(block) => block,
            None => return Ok(0..0),
        };

        let first_range = first.range(global.clone()).await?;
        let last_range = last.range(global.clone()).await?;

        Ok(first_range.start..last_range.end)
    }

    fn get<'a, U: GlobalTrait + std::marker::Send + std::marker::Sync + 'a>(
        &'a self,
        global: Arc<U>,
        range: Range<usize>,
    ) -> BoxStream<'a, Result<Vec<u8>, String>> {
        Box::pin(async_stream::stream! {
            for block in self.blocks.iter() {
                let global_clone = global.clone();
                let range_clone = range.clone();
                let mut stream = block.get(global_clone, range_clone);
                while let Some(data) = stream.next().await {
                    yield data;
                }
            }
        })
    }

    async fn put<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &mut self,
        global: Arc<U>,
        data: Vec<u8>,
        range: Range<usize>,
    ) -> Result<(), String> {
        let start_offset = range.start;
        let mut start = range.start;

        for block in self.blocks.iter_mut() {
            let block_range = block.range(global.clone()).await?;
            if block_range.end <= start {
                break;
            }
            let slice_range = block_range.start - start_offset..block_range.end - start_offset;
            let slice = data[slice_range].to_vec();
            block.put(global.clone(), slice, block_range).await?;
        }

        start = match self.blocks.last() {
            Some(block) => block.range(global.clone()).await?.end,
            None => range.start,
        };

        // if data is left, we create new blocks just like we did in the create function
        while start < range.end && self.blocks.len() < global.get_direct_block_count() {
            let block = DirectBlock::create(
                global.clone(),
                data[(start - start_offset)..].to_vec(),
                start,
            )
            .await?;
            let range = block.range(global.clone()).await?;
            start = range.end;
            self.blocks.push(block.to_enum());
        }

        // if there is still data left, we create a stored block
        if start < range.end {
            let slice = data[start - start_offset..].to_vec();
            let block = StoredBlock::create(global, slice, start).await?;
            self.blocks.push(block.to_enum());
        }

        Ok(())
    }

    async fn delete<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &self,
        global: Arc<U>,
    ) -> Result<(), String> {
        let mut errors = Vec::new();
        for block in self.blocks.iter() {
            match block.delete(global.clone()).await {
                Ok(_) => (),
                Err(err) => errors.push(err),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join(", "))
        }
    }

    async fn create<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        global: Arc<U>,
        data: Vec<u8>,
        start: usize,
    ) -> Result<BlockType, String> {
        let mut blocks = Vec::new(); // we will make sure that these are in order
        let slice_offset = start;
        let mut start = start;
        let end = start + data.len();

        let mut error: Option<String> = None;
        while start < end && blocks.len() < global.get_direct_block_count() {
            let block = DirectBlock::create(
                global.clone(),
                data[(start - slice_offset)..].to_vec(),
                start,
            )
            .await?;
            let range = match block.range(global.clone()).await {
                Ok(range) => range,
                Err(err) => {
                    error = Some(err);
                    break;
                }
            };
            start = range.end;
            blocks.push(block.to_enum());
        }

        // if we encountered an error, we delete all the blocks we created
        if let Some(err) = error {
            let mut errors = vec![err];
            for block in blocks.iter() {
                match block.delete(global.clone()).await {
                    Ok(_) => (),
                    Err(err) => errors.push(err),
                }
            }

            return Err(errors.join(", "));
        }

        // if there is still data left, we create a stored block
        if start < end {
            let slice = data[start - slice_offset..].to_vec();
            let block = StoredBlock::create(global, slice, start).await?;
            blocks.push(block.to_enum());
        }

        Ok(BlockType::Indirect(IndirectBlock { blocks }))
    }

    fn to_enum(self) -> BlockType {
        BlockType::Indirect(self)
    }
}
