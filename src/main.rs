use axum::{
    body::Body,
    extract::{
        multipart::Field,
        ws::{Message, WebSocket},
        Multipart, WebSocketUpgrade,
    },
    http::StatusCode,
    response::Response,
    routing::post,
    Router,
};
use chrono::Duration;
use clap::Parser;
use config;
use dynerror::{bail, Context, Error, Result};
use s3::{creds::Credentials, Bucket, Region};
use serde::{Deserialize, Serialize};
use std::{env, path::Path, sync::Arc};
use tokio::{io, net::TcpListener, sync::OnceCell};
use tokio_util::io::StreamReader;
use tracing::error;

mod bytes_base64;
mod duration_positive_seconds;

#[derive(Debug, Serialize, Deserialize, Default)]
struct UploadMetadata {
    #[serde(with = "duration_positive_seconds")]
    duration: Duration,
    download_limit: i32,
    #[serde(with = "bytes_base64")]
    file_metadata: Vec<u8>,
}

#[derive(Debug, Serialize)]
struct JsonError {
    error: String,
}

#[derive(Debug, Deserialize, Default)]
struct Config {
    s3_bucket: String,
    s3_endpoint: String,
    s3_access_key_id: String,
    s3_secret_key: String,
    s3_region: String,
    upload_buffer_size: i32, // Minimum size of 5 MiB
    upload_size_limit: i32,
}

static CONFIG: OnceCell<Config> = OnceCell::const_new();
static BUCKET: OnceCell<Box<s3::Bucket>> = OnceCell::const_new();

#[derive(Debug, clap::Parser)]
struct Args {
    config_path: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let config_parser = config::Config::builder()
        .add_source(config::File::with_name(&args.config_path))
        .add_source(config::Environment::with_prefix("SEND"))
        .build()
        .unwrap();
    let config = match config_parser.try_deserialize::<Config>() {
        Ok(x) => x,
        Err(err) => {
            eprintln!("failed to parse {}: {}", args.config_path, err);
            return;
        }
    };
    BUCKET
        .set(
            s3::Bucket::new(
                &config.s3_bucket,
                config.s3_region.parse().unwrap(),
                Credentials::new(
                    Some(&config.s3_access_key_id),
                    Some(&config.s3_secret_key),
                    None,
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    CONFIG.set(config).unwrap();

    let app = Router::new().route("/v1/upload", post(upload));

    let listener = TcpListener::bind(":13370").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn upload(mut mp: Multipart) -> Result<Response> {
    let config = CONFIG.get().context("couldn't get config")?;
    let bucket = BUCKET.get().context("couldn't get bucket")?.clone();
    let mut meta: Option<UploadMetadata> = None;
    loop {
        let field = match mp.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(_) => {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::empty())
                    .context("Error getting next field");
            }
        };

        let name = match field.name() {
            Some(x) => x.to_string(),
            None => {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::empty())
                    .context(
                        "error parsing unnamed multipart/form-data part",
                    );
            }
        };

        if name == "metadata" {
            if meta.is_some() {
                bail!("multiple metadata fields in multipart/form-data")
            }
            let data = match field.bytes().await {
                Ok(x) => x,
                Err(_) => {
                    return Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Body::empty())
                        .context(
                            "error getting multipart/form-data field content",
                        );
                }
            };

            let um = serde_json::from_slice::<UploadMetadata>(&data)
                .context("error parsing metadata as JSON")?;
            meta = Some(um);
        } else if name == "file" {
            bucket
                .put_object_stream(
                    &mut StreamReader::new(field),
                    format!("cool"),
                )
                .await;
            return Response::builder()
                .body(Body::from_stream(field))
                .context("couldn't send body as response");
        } else {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::empty())
                .context("unexpected multipart/form-data field");
        }
    }
    unreachable!()
}
