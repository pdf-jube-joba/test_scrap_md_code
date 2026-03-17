mod config;
mod identity;
mod plugin;
mod repository;
mod workspace;

use std::{env, net::SocketAddr, sync::Arc};

use anyhow::{Result, anyhow, bail};
use axum::{
    Router,
    extract::{Extension, Path, State},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use config::RepositoryConfig;
use identity::{IdentityConfig, RequestIdentity, capture_identity};
use repository::{FsRepository, Repository};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use workspace::WorkspaceService;

#[derive(Clone)]
struct AppState {
    workspace: Arc<WorkspaceService>,
}

#[derive(Debug)]
struct CliOptions {
    repository_path: String,
    task: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = parse_cli_options()?;
    let repository: Arc<dyn Repository> = Arc::new(FsRepository::open(cli.repository_path)?);
    let repository_name = repository
        .repository_root()
        .file_name()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| repository.repository_root().as_str().to_owned());
    let config = Arc::new(RepositoryConfig::load(repository.repository_root())?);
    let workspace = Arc::new(WorkspaceService::new(repository, config, repository_name));

    if let Some(task_name) = &cli.task {
        tracing::info!(task = %task_name, "running task before serve");
        workspace.run_task(task_name).await?;
    }

    let identity = IdentityConfig::load();
    let state = Arc::new(AppState { workspace: workspace.clone() });

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/.plugin/{name}/run", post(run_plugin_handler))
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

    let addr = SocketAddr::from(([127, 0, 0, 1], workspace.serve_port()));
    tracing::info!("listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn parse_cli_options() -> Result<CliOptions> {
    let mut args = env::args().skip(1);
    let repository_path = args
        .next()
        .ok_or_else(|| anyhow!("usage: workspace_fs <repository-path> [--task <name>]"))?;
    let mut task = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--task" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --task"))?;
                task = Some(value);
            }
            _ => bail!("unknown argument: {arg}"),
        }
    }

    Ok(CliOptions { repository_path, task })
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            env::var("RUST_LOG").unwrap_or_else(|_| "workspace_fs=info,tower_http=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

async fn root_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
) -> Result<Response, workspace::WorkspaceError> {
    state.workspace.get_root(&identity.user).await
}

async fn run_plugin_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(name): Path<String>,
) -> Result<Response, workspace::WorkspaceError> {
    state.workspace.run_manual_plugin(&name, &identity.user).await
        .map(|_| axum::http::StatusCode::NO_CONTENT.into_response())
        .map_err(workspace::WorkspaceError::internal)
}

async fn get_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
) -> Result<Response, workspace::WorkspaceError> {
    state.workspace.get_path(&path, &identity.user).await
}

async fn post_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
    body: String,
) -> Result<Response, workspace::WorkspaceError> {
    state.workspace.create_path(&path, &body, &identity.user).await
}

async fn put_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
    body: String,
) -> Result<Response, workspace::WorkspaceError> {
    state.workspace.update_file(&path, &body, &identity.user).await
}

async fn delete_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
) -> Result<Response, workspace::WorkspaceError> {
    state.workspace.delete_path(&path, &identity.user).await
}
