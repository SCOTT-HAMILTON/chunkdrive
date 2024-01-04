use serde::Deserialize;

use rusoto_core::{ByteStream, HttpClient, Region, RusotoError};
use rusoto_credential::StaticProvider;
use rusoto_s3::{
    GetObjectRequest, ListObjectsV2Request, PutObjectOutput, PutObjectRequest, S3Client, S3,
};

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub struct S3Type {
    access_key_id: String,
    secret_access_key: String,
    endpoint: String,
    bucket_name: String,
    region: String,
}

pub async fn list_files_in_bucket(
    s3: &S3Type,
) -> Result<Vec<String>, rusoto_core::RusotoError<rusoto_s3::ListObjectsV2Error>> {
    let access_key_id = &s3.access_key_id;
    let secret_access_key = &s3.secret_access_key;
    let endpoint = &s3.endpoint;
    let bucket_name = &s3.bucket_name;
    let provider = StaticProvider::new_minimal(access_key_id.into(), secret_access_key.into());
    let region = Region::Custom {
        name: s3.region.to_owned(),
        endpoint: endpoint.to_owned(),
    };

    let client = S3Client::new_with(
        HttpClient::new().expect("Failed to create HTTP client"),
        provider,
        region,
    );

    let request = ListObjectsV2Request {
        bucket: bucket_name.to_string(),
        ..Default::default()
    };

    Ok(client
        .list_objects_v2(request)
        .await?
        .contents
        .unwrap_or_default()
        .into_iter()
        .filter_map(|object| object.key)
        .collect())
}

pub async fn download_file(
    s3: &S3Type,
    object_key: &str,
) -> Result<ByteStream, rusoto_core::RusotoError<rusoto_s3::GetObjectError>> {
    let access_key_id = &s3.access_key_id;
    let secret_access_key = &s3.secret_access_key;
    let endpoint = &s3.endpoint;
    let bucket_name = &s3.bucket_name;
    let provider = StaticProvider::new_minimal(access_key_id.into(), secret_access_key.into());
    let region = Region::Custom {
        name: s3.region.to_owned(),
        endpoint: endpoint.to_owned(),
    };

    let client = S3Client::new_with(
        HttpClient::new().expect("Failed to create HTTP client"),
        provider,
        region,
    );

    let request = GetObjectRequest {
        bucket: bucket_name.to_string(),
        key: object_key.to_string(),
        ..Default::default()
    };

    let output = client.get_object(request).await?;

    output.body.ok_or(RusotoError::Validation(
        "can't download file, GetObjectRequest body missing".to_string(),
    ))
}

// Fonction pour uploader un fichier vers le bucket
pub async fn upload_file(
    s3: &S3Type,
    object_key: &str,
    file: ByteStream,
) -> Result<PutObjectOutput, rusoto_core::RusotoError<rusoto_s3::PutObjectError>> {
    let access_key_id = &s3.access_key_id;
    let secret_access_key = &s3.secret_access_key;
    let endpoint = &s3.endpoint;
    let bucket_name = &s3.bucket_name;
    let provider = StaticProvider::new_minimal(access_key_id.into(), secret_access_key.into());
    let region = Region::Custom {
        name: s3.region.to_owned(),
        endpoint: endpoint.to_owned(),
    };
    let client = S3Client::new_with(
        HttpClient::new().expect("Failed to create HTTP client"),
        provider,
        region,
    );

    let request = PutObjectRequest {
        bucket: bucket_name.to_string(),
        key: object_key.to_string(),
        body: Some(file), // Replace this with your file content
        ..Default::default()
    };

    client.put_object(request).await
}
