use axum::{
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

#[derive(Debug, Clone)]
pub struct IdentityConfig;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RequestIdentity {
    pub user: String,
}

impl IdentityConfig {
    pub fn load() -> Self {
        Self
    }
}

pub async fn capture_identity(
    State(_identity): State<IdentityConfig>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let user = request
        .headers()
        .get("user-identity")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .map(ToOwned::to_owned);

    request.extensions_mut().insert(RequestIdentity {
        user: user.unwrap_or_default(),
    });
    next.run(request).await
}
