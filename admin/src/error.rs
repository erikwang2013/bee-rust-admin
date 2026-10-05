// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bee_orm::OrmError;
use serde_json::{Value, json};

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    Forbidden(String),
    NotFound,
    Internal(String),
}

impl ApiError {
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            ApiError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, "登录已失效，请重新登录".to_string())
            }
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "资源不存在".to_string()),
            ApiError::Internal(m) => {
                // 细节只进日志，不外泄给客户端
                tracing::error!("内部错误: {m}");
                (StatusCode::INTERNAL_SERVER_ERROR, "服务器内部错误".to_string())
            }
        };
        let body = json!({ "code": status.as_u16(), "msg": msg, "data": Value::Null });
        (status, Json(body)).into_response()
    }
}

impl From<OrmError> for ApiError {
    fn from(e: OrmError) -> Self {
        match e {
            OrmError::DuplicateKey(_) => ApiError::BadRequest("数据已存在（唯一约束冲突）".into()),
            OrmError::NotFound => ApiError::NotFound,
            other => ApiError::Internal(format!("数据库错误: {other}")),
        }
    }
}

/// 成功信封：`{"code":0,"msg":"ok","data":…}`。
pub fn ok<T: serde::Serialize>(data: T) -> Json<Value> {
    Json(json!({ "code": 0, "msg": "ok", "data": data }))
}

/// 统一 JSON 提取器：把 axum 的 422 纯文本拒绝转成 `{code,msg,data}` 信封，
/// 让所有错误响应形状一致（前端拦截器按 msg 提示）。
pub struct AppJson<T>(pub T);

impl<S, T> axum::extract::FromRequest<S> for AppJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(v)) => Ok(AppJson(v)),
            Err(rej) => Err(ApiError::BadRequest(format!("请求体格式错误: {}", rej.body_text()))),
        }
    }
}
