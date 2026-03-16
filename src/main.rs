mod config;
mod identity;
mod repository;

use std::{env, net::SocketAddr, sync::Arc};

use anyhow::{Result, anyhow};
use axum::{
    middleware,
    Router,
    extract::{Extension, Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use config::RepositoryConfig;
use identity::{IdentityConfig, RequestIdentity, capture_identity};
use repository::{FsRepository, Repository};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    repository: Arc<dyn Repository>,
}

#[derive(Debug)]
struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            self.message,
        )
            .into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self {
        Self::internal(error)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let repository_argument = env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("usage: workspace_fs <repository-path>"))?;
    let repository: Arc<dyn Repository> = Arc::new(FsRepository::open(repository_argument)?);
    let repository_config = RepositoryConfig::load(repository.repository_root())?;
    let identity = IdentityConfig::load();
    let state = Arc::new(AppState { repository });

    let app = Router::new()
        .route("/", get(root_handler))
        .route(
            "/{*path}",
            get(get_path_handler)
                .post(post_path_handler)
                .put(put_path_handler)
                .delete(delete_path_handler),
        )
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn_with_state(identity, capture_identity))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], repository_config.serve.port));
    tracing::info!("listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            env::var("RUST_LOG").unwrap_or_else(|_| "workspace_fs=debug,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

async fn root_handler(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    directory_response(&*state.repository, "").await
}

async fn get_path_handler(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    if path.ends_with('/') {
        return directory_response(&*state.repository, &path).await;
    }

    if state.repository.list_directory(&path).await.is_ok() {
        return Err(AppError::bad_request("directory path must end with /"));
    }

    let content = match state.repository.read_text_file(&path).await {
        Ok(content) => content,
        Err(error) => {
            let mapped = map_read_error(error);
            tracing::warn!(path = %path, status = %mapped.status, error = %mapped.message, "read failed");
            return Err(mapped);
        }
    };

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        content,
    )
        .into_response())
}

async fn put_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
    body: String,
) -> Result<Response, AppError> {
    reject_directory_path(&path, "cannot write to a directory path")?;

    match state.repository.write_text_file(&path, &body).await {
        Ok(()) => {
            tracing::info!(user = %identity.user, path = %path, "file updated");
        }
        Err(error) => {
            let mapped = map_write_error(error);
            tracing::warn!(
                user = %identity.user,
                path = %path,
                status = %mapped.status,
                error = %mapped.message,
                "file update failed"
            );
            return Err(mapped);
        }
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn post_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
    body: String,
) -> Result<Response, AppError> {
    if path.ends_with('/') {
        match state.repository.create_directory(&path).await {
            Ok(()) => {
                tracing::info!(user = %identity.user, path = %path, "directory created");
            }
            Err(error) => {
                let mapped = map_create_directory_error(error);
                tracing::warn!(
                    user = %identity.user,
                    path = %path,
                    status = %mapped.status,
                    error = %mapped.message,
                    "directory create failed"
                );
                return Err(mapped);
            }
        }
        return Ok(StatusCode::CREATED.into_response());
    }

    match state.repository.create_text_file(&path, &body).await {
        Ok(()) => {
            tracing::info!(user = %identity.user, path = %path, "file created");
        }
        Err(error) => {
            let mapped = map_create_error(error);
            tracing::warn!(
                user = %identity.user,
                path = %path,
                status = %mapped.status,
                error = %mapped.message,
                "file create failed"
            );
            return Err(mapped);
        }
    }

    Ok(StatusCode::CREATED.into_response())
}

async fn delete_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    if path.ends_with('/') {
        match state.repository.delete_directory(&path).await {
            Ok(()) => {
                tracing::info!(user = %identity.user, path = %path, "directory deleted");
            }
            Err(error) => {
                let mapped = map_delete_directory_error(error);
                tracing::warn!(
                    user = %identity.user,
                    path = %path,
                    status = %mapped.status,
                    error = %mapped.message,
                    "directory delete failed"
                );
                return Err(mapped);
            }
        }
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    match state.repository.delete_file(&path).await {
        Ok(()) => {
            tracing::info!(user = %identity.user, path = %path, "file deleted");
        }
        Err(error) => {
            let mapped = map_delete_error(error);
            tracing::warn!(
                user = %identity.user,
                path = %path,
                status = %mapped.status,
                error = %mapped.message,
                "file delete failed"
            );
            return Err(mapped);
        }
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn directory_response(
    repository: &dyn Repository,
    path: &str,
) -> Result<Response, AppError> {
    let entries = match repository.list_directory(path).await {
        Ok(entries) => entries,
        Err(error) => {
            let message = error.to_string();
            let mapped = if message.contains("not a directory") || message.contains("No such file") {
                AppError::not_found("directory not found")
            } else {
                AppError::internal(error)
            };
            tracing::warn!(path = %path, status = %mapped.status, error = %mapped.message, "directory listing failed");
            return Err(mapped);
        }
    };

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        entries.join("\n"),
    )
        .into_response())
}

fn map_create_error(error: anyhow::Error) -> AppError {
    let message = error.to_string();
    if message.contains("file already exists") {
        return AppError::conflict("file already exists");
    }
    map_parent_or_path_error(error)
}

fn map_create_directory_error(error: anyhow::Error) -> AppError {
    let message = error.to_string();
    if message.contains("directory already exists") {
        return AppError::conflict("directory already exists");
    }
    map_parent_or_path_error(error)
}

fn map_write_error(error: anyhow::Error) -> AppError {
    let message = error.to_string();
    if message.contains("file not found") {
        return AppError::not_found("file not found");
    }
    if message.contains("path is a directory") {
        return AppError::bad_request("path is a directory");
    }
    map_path_error(error)
}

fn map_read_error(error: anyhow::Error) -> AppError {
    let message = error.to_string();
    if message.contains("No such file") || message.contains("os error 2") {
        return AppError::not_found("path not found");
    }
    if message.contains("Is a directory") || message.contains("os error 21") {
        return AppError::bad_request("path is a directory");
    }
    map_path_error(error)
}

fn map_delete_error(error: anyhow::Error) -> AppError {
    let message = error.to_string();
    if message.contains("file not found") {
        return AppError::not_found("file not found");
    }
    if message.contains("path is a directory") {
        return AppError::bad_request("path is a directory");
    }
    map_path_error(error)
}

fn map_delete_directory_error(error: anyhow::Error) -> AppError {
    let message = error.to_string();
    if message.contains("directory not found") {
        return AppError::not_found("directory not found");
    }
    if message.contains("path is not a directory") {
        return AppError::bad_request("path is not a directory");
    }
    if message.contains("directory is not empty")
        || message.contains("Directory not empty")
        || message.contains("os error 39")
    {
        return AppError::conflict("directory is not empty");
    }
    map_path_error(error)
}

fn reject_directory_path(path: &str, message: &'static str) -> Result<(), AppError> {
    if path.ends_with('/') {
        return Err(AppError::bad_request(message));
    }
    Ok(())
}

fn map_parent_or_path_error(error: anyhow::Error) -> AppError {
    let message = error.to_string();
    if message.contains("parent directory not found") {
        return AppError::not_found("parent directory not found");
    }
    if message.contains("parent path is not a directory") {
        return AppError::bad_request("parent path is not a directory");
    }
    map_path_error(error)
}

fn map_path_error(error: anyhow::Error) -> AppError {
    let message = error.to_string();
    if is_invalid_path_message(&message) {
        return AppError::bad_request(message);
    }
    AppError::internal(error)
}

fn is_invalid_path_message(message: &str) -> bool {
    message.contains("path escapes repository root")
        || message.contains("absolute paths are not allowed")
        || message.contains("reserved path")
}
