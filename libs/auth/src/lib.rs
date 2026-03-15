use anyhow::{bail, Result};
use axum::{
    extract::{FromRequestParts, Extension},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use http::{header::AUTHORIZATION, HeaderMap};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::{ops::Deref, sync::Arc};
use task_model::{target_count, CommandRequest};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
    pub approvals: Vec<String>,
    pub exp: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationOutcome {
    pub subject: String,
    pub tenant_id: String,
    pub approval_required: bool,
}

#[derive(Clone)]
pub struct JwtSecret(pub Arc<str>);

#[derive(Debug, Clone)]
pub struct VerifiedClaims(pub Claims);

#[derive(Debug)]
pub struct AuthError {
    status: StatusCode,
    message: String,
}

pub fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

pub fn validate_token(token: &str, jwt_secret: &str) -> Result<Claims> {
    if token.is_empty() || jwt_secret.is_empty() {
        bail!("missing token or jwt secret");
    }

    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;

    let claims = decode::<Claims>(
        token,
        &DecodingKey::from_secret(jwt_secret.as_bytes()),
        &validation,
    )?
    .claims;

    if claims.exp < Utc::now().timestamp() as usize {
        bail!("token expired");
    }

    Ok(claims)
}

pub fn authorize_command(claims: &Claims, request: &CommandRequest) -> Result<AuthorizationOutcome> {
    if claims.tenant_id != request.tenant_id && !has_role(claims, "super-admin") {
        bail!("tenant mismatch");
    }

    if claims.sub != request.requested_by && !has_role(claims, "super-admin") {
        bail!("requested_by does not match token subject");
    }

    let destructive = matches!(request.action.as_str(), "unban" | "unban_all" | "ban" | "kick");

    if destructive && !has_any_role(claims, &["admin", "moderator", "tenant-admin", "super-admin"]) {
        bail!("insufficient role for destructive action");
    }

    let targets = target_count(request);
    let approval_required = destructive
        && targets > 100
        && !has_role(claims, "bulk-approver")
        && !claims.approvals.iter().any(|scope| scope == &request.action || scope == "*");

    Ok(AuthorizationOutcome {
        subject: claims.sub.clone(),
        tenant_id: claims.tenant_id.clone(),
        approval_required,
    })
}

pub fn authorize_approval_read(claims: &Claims, tenant_id: &str) -> Result<()> {
    if claims.tenant_id != tenant_id && !has_role(claims, "super-admin") {
        bail!("tenant mismatch");
    }

    if !has_any_role(claims, &["admin", "tenant-admin", "super-admin", "bulk-approver", "auditor"]) {
        bail!("insufficient role for approval access");
    }

    Ok(())
}

pub fn authorize_approval_decision(claims: &Claims, tenant_id: &str, actor: &str) -> Result<()> {
    if claims.sub != actor && !has_role(claims, "super-admin") {
        bail!("approved_by does not match token subject");
    }

    authorize_approval_read(claims, tenant_id)?;

    if !has_any_role(claims, &["tenant-admin", "super-admin", "bulk-approver"]) {
        bail!("insufficient role for approval decisions");
    }

    Ok(())
}

pub fn authorize_analytics_read(claims: &Claims, requested_tenant_id: Option<&str>) -> Result<Option<String>> {
    if !has_any_role(claims, &["admin", "tenant-admin", "super-admin", "bulk-approver", "auditor"]) {
        bail!("insufficient role for analytics access");
    }

    if has_role(claims, "super-admin") {
        return Ok(requested_tenant_id.map(ToOwned::to_owned));
    }

    match requested_tenant_id {
        Some(tenant_id) if tenant_id != claims.tenant_id => bail!("tenant mismatch"),
        _ => Ok(Some(claims.tenant_id.clone())),
    }
}

impl Deref for VerifiedClaims {
    type Target = Claims;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

impl<S> FromRequestParts<S> for VerifiedClaims
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> std::result::Result<Self, Self::Rejection> {
        let Extension(secret) = Extension::<JwtSecret>::from_request_parts(parts, state)
            .await
            .map_err(|_| AuthError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "JWT secret extension not configured".to_string(),
            })?;

        let token = extract_bearer(&parts.headers).ok_or_else(|| AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "missing bearer token".to_string(),
        })?;

        let claims = validate_token(token, secret.0.as_ref()).map_err(|error| AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: error.to_string(),
        })?;

        Ok(Self(claims))
    }
}

fn has_role(claims: &Claims, role: &str) -> bool {
    claims.roles.iter().any(|candidate| candidate == role)
}

fn has_any_role(claims: &Claims, roles: &[&str]) -> bool {
    roles.iter().any(|role| has_role(claims, role))
}
