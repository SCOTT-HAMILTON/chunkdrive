/*
   This block type uses Stored to store the description of a block it wraps.
*/

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use serde::{Deserialize, Serialize};
use std::{ops::Range, sync::Arc};

use crate::{
    blocks::block::{Block, BlockType},
    global::GlobalTrait,
    stored::Stored,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredBlock {
    #[serde(rename = "s")]
    pub stored: Stored,
}

#[async_trait]
impl Block for StoredBlock {
    async fn range<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &self,
        global: Arc<U>,
    ) -> Result<Range<usize>, String> {
        self.stored
            .get::<BlockType, U>(global.clone())
            .await?
            .range(global)
            .await
    }

    async fn put<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &mut self,
        global: Arc<U>,
        data: Vec<u8>,
        range: Range<usize>,
    ) -> Result<(), String> {
        let mut block = self.stored.get::<BlockType, U>(global.clone()).await?;
        block.put(global.clone(), data, range).await?;
        self.stored.put(global, block).await
    }

    fn get<'a, U: GlobalTrait + std::marker::Send + std::marker::Sync + 'a>(
        &'a self,
        global: Arc<U>,
        range: Range<usize>,
    ) -> BoxStream<'a, Result<Vec<u8>, String>> {
        Box::pin(async_stream::stream! {
            let global = global.clone();
            let block = self.stored.get::<BlockType, U>(global.clone()).await?;
            let mut stream = block.get(global, range.clone());
            while let Some(chunk) = stream.next().await {
                yield chunk;
            }
        })
    }

    async fn delete<U: GlobalTrait + std::marker::Send + std::marker::Sync>(
        &self,
        global: Arc<U>,
    ) -> Result<(), String> {
        let mut errors = Vec::new();
        match self
            .stored
            .get::<BlockType, U>(global.clone())
            .await
            .unwrap()
            .delete(global.clone())
            .await
        {
            Ok(_) => (),
            Err(e) => errors.push(e),
        }
        match self.stored.delete(global).await {
            Ok(_) => (),
            Err(e) => errors.push(e),
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
        let block = BlockType::create(global.clone(), data, start).await?;
        let stored = Stored::create(global.clone(), block).await?;
        Ok(BlockType::Stored(StoredBlock { stored }))
    }

    fn to_enum(self) -> BlockType {
        BlockType::Stored(self)
    }
}
