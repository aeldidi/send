use axum::{
    body::Body,
    extract::Multipart,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::post,
    Router,
};
use chrono::Duration;
use clap::Parser;
use dynerror::{bail, Context, Error};
use s3::{creds::Credentials, Bucket, Region};
use serde::{Deserialize, Serialize};
use std::{env, path::Path, sync::Arc};
use tokio::{io, net::TcpListener, sync::OnceCell};
use tokio_util::io::StreamReader;
use tracing::{error, info};
use tracing_subscriber::{
    layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

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
    listen_addr: String,
}

static CONFIG: OnceCell<Config> = OnceCell::const_new();
static BUCKET: OnceCell<Box<s3::Bucket>> = OnceCell::const_new();

#[derive(Debug, clap::Parser)]
struct Args {
    config_path: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let config_parser = config::Config::builder()
        .add_source(config::File::with_name(&args.config_path))
        .add_source(config::Environment::with_prefix("SEND"))
        .build()
        .context("error creating config parser")?;
    let config = config_parser
        .try_deserialize::<Config>()
        .context("error parsing config")?;
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
                .context("error initializing S3 credentials")?,
            )
            .context("error initializing S3 bucket")?,
        )
        .context("error setting global bucket")?;
    CONFIG.set(config).context("error setting config");

    let app = Router::new().route("/v1/upload", post(upload));

    let listener = TcpListener::bind(config.listen_addr)
        .await
        .context("couldn't bind to listen_addr")?;
    info!("listening on {}", config.listen_addr);
    axum::serve(listener, app)
        .await
        .context("error serving HTTP")
}

async fn upload(
    headers: HeaderMap,
    mut mp: Multipart,
) -> Result<Response, Error> {
    let config = CONFIG.get().context("couldn't get config")?;
    let bucket = BUCKET.get().context("couldn't get bucket")?.clone();

    let signing_key = {
        let auth = match headers.get("Authorization") {
            Some(x) => match x.to_str() {
                Ok(x) => x,
                Err(_) => {
                    return Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Body::empty())
                        .context("error creating request");
                }
            },
            None => {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::empty())
                    .context("error creating request");
            }
        };
        // TODO: get signing key from this
    };

    let field = if let Ok(Some(x)) = mp.next_field().await {
        x
    } else {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::empty())
            .context("error creating request");
    };
    let name = match field.name() {
        Some(x) => x,
        None => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::empty())
                .context("error creating request");
        }
    };

    if name != "metadata" {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::empty())
            .context("error creating request");
    }

    let bytes = field
        .bytes()
        .await
        .context("error getting data for metadata")?;
    let um = serde_json::from_slice::<UploadMetadata>(&bytes)
        .context("error parsing metadata as JSON")?;

    let field = if let Ok(Some(x)) = mp.next_field().await {
        x
    } else {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::empty())
            .context("error creating request");
    };
    let name = match field.name() {
        Some(x) => x,
        None => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::empty())
                .context("error creating request");
        }
    };

    if name != "data" {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::empty())
            .context("error creating request");
    }
}
