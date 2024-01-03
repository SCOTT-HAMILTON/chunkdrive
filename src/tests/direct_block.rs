use serde_yaml::from_str;
use std::sync::Arc;

use super::utils::make_temp_config;
use crate::{
    blocks::{block::Block, direct_block::DirectBlock},
    global::Global,
};

#[tokio::test]
async fn empty_data() {
    let global = Arc::new(from_str::<Global>(&make_temp_config(false, 30)).unwrap());
    let data = Vec::new();
    let block = DirectBlock::create(global.clone(), data.clone(), 0).await;
    assert!(block.is_err());
}
