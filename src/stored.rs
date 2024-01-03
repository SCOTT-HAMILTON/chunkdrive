/*
   This module implements Stored object, which serializes and deserializes objects to and from the database.
   It has no knowledge of the data types, so make sure to use the correct type when deserializing.
   It uses messagepack for serialization for backwards compatibility.
*/

use crate::global::{Descriptor, GlobalTrait};
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Stored {
    #[serde(rename = "b")]
    bucket: String,
    #[serde(rename = "d")]
    descriptor: Descriptor,
}

impl PartialEq for Stored {
    fn eq(&self, other: &Self) -> bool {
        self.bucket == other.bucket && self.descriptor == other.descriptor
    }
}

impl Stored {
    pub async fn get<T: Deserialize<'static>, U: GlobalTrait>(
        &self,
        global: Arc<U>,
    ) -> Result<T, String> {
        // Get bucket
        let bucket = global.get_bucket(&self.bucket).ok_or("Bucket not found")?;

        // Get data
        let data = bucket.get(&self.descriptor).await?;

        // Deserialize data
        let mut deserializer = Deserializer::new(&data[..]);
        T::deserialize(&mut deserializer).map_err(|e| e.to_string())
    }

    pub async fn put<T: Serialize, U: GlobalTrait>(
        &self,
        global: Arc<U>,
        data: T,
    ) -> Result<(), String> {
        // Serialize data
        let mut serializer = Serializer::new(Vec::new()).with_struct_map(); // https://github.com/3Hren/msgpack-rust/issues/318
        data.serialize(&mut serializer).map_err(|e| e.to_string())?;
        let data = serializer.into_inner();

        // Get bucket
        let bucket = global.get_bucket(&self.bucket).ok_or("Bucket not found")?;

        // Put data
        bucket.put(&self.descriptor, data).await?;

        Ok(())
    }

    pub async fn create<T: Serialize, U: GlobalTrait>(
        global: Arc<U>,
        data: T,
    ) -> Result<Stored, String> {
        // Serialize data
        let mut serializer = Serializer::new(Vec::new()).with_struct_map(); // https://github.com/3Hren/msgpack-rust/issues/318
        data.serialize(&mut serializer).map_err(|e| e.to_string())?;
        let data = serializer.into_inner();

        // Find bucket
        let bucket_name = global
            .next_bucket(data.len(), &Vec::new())
            .ok_or(format!("No bucket found for data of size {}", data.len()))?;
        let bucket = global.get_bucket(bucket_name).ok_or("Bucket not found")?;

        // Put data
        let descriptor = bucket.create().await?;
        bucket.put(&descriptor, data).await?;

        Ok(Stored {
            bucket: bucket_name.to_owned(),
            descriptor,
        })
    }

    pub async fn delete<U: GlobalTrait>(&self, global: Arc<U>) -> Result<(), String> {
        // Get bucket
        let bucket = global.get_bucket(&self.bucket).ok_or("Bucket not found")?;

        // Delete data
        bucket.delete(&self.descriptor).await
    }

    pub fn as_url(&self) -> String {
        format!(
            "{}${}",
            urlencoding::encode(&self.bucket).replace('$', "%24"),
            urlencoding::encode_binary(&self.descriptor).replace('$', "%24")
        )
    }

    pub fn from_url(bucket: &str, descriptor: &str) -> Result<Stored, String> {
        let bucket = urlencoding::decode(bucket)
            .map_err(|_| "Invalid bucket")?
            .to_string();

        let descriptor = urlencoding::decode_binary(descriptor.as_bytes()).to_vec();

        Ok(Stored { bucket, descriptor })
    }
}
