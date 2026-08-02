//! HTTP 鉴权中间件：统一作用于全部路由。
//!
//! - 配置了 `[auth] token`：请求必须带 `Authorization: Bearer <token>`、
//!   `Cookie: ipkvm_token=<token>` 或 query 参数 `?token=<token>` 之一；
//!   后两者放行时附带 Set-Cookie（query token 只在静态页首访时用，换来
//!   cookie 后后续请求自动带）。
//! - 未配置 token：仅放行回环来源（防默认暴露），其余 403。
//!
//! 判定逻辑抽成纯函数 `authorize`，不依赖 HTTP 运行时，便于单元测试。

use std::net::IpAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// 鉴权 cookie 名。值即 token。
pub const AUTH_COOKIE: &str = "ipkvm_token";

/// 中间件共享的鉴权状态。
#[derive(Clone, Debug)]
pub struct AuthState {
    pub token: Option<String>,
}

/// 纯判定函数（不依赖运行时）：`None` 表示未配置 token。
///
/// 返回 `true` 放行；`false` 拒绝（401/403 由调用方按配置与否选择）。
pub fn authorize(
    peer_ip: IpAddr,
    bearer: Option<&str>,
    cookie: Option<&str>,
    query_token: Option<&str>,
    configured: Option<&str>,
) -> bool {
    let Some(configured) = configured else {
        return peer_ip.is_loopback();
    };
    bearer == Some(configured) || cookie == Some(configured) || query_token == Some(configured)
}

/// 从 `Authorization` 头提取 Bearer token。
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// 从 `Cookie` 头提取 `ipkvm_token` 值。
fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::COOKIE)?.to_str().ok()?;
    value.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == AUTH_COOKIE).then_some(value)
    })
}

/// 从 URI query 提取 `token` 参数（token 为 ASCII，不做 URL 解码）。
fn query_token(uri: &Uri) -> Option<&str> {
    uri.query()?.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == "token").then_some(value)
    })
}

pub async fn require_auth(
    State(state): State<AuthState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let bearer = bearer_token(request.headers());
    let cookie = cookie_token(request.headers());
    let query = query_token(request.uri());
    let configured = state.token.as_deref();
    let via_query = query.is_some_and(|value| Some(value) == configured);

    if !authorize(peer.ip(), bearer, cookie, query, configured) {
        return if configured.is_some() {
            let mut response = StatusCode::UNAUTHORIZED.into_response();
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            response
        } else {
            StatusCode::FORBIDDEN.into_response()
        };
    }

    let mut response = next.run(request).await;
    // query token 放行时种下 cookie：后续同源请求（含 /rfb 升级）自动带。
    if via_query && let Some(token) = configured {
        let value = format!("{AUTH_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax");
        if let Ok(value) = HeaderValue::try_from(value) {
            response.headers_mut().insert(header::SET_COOKIE, value);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_token_only_loopback_is_allowed() {
        let loopback = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
        let remote = std::net::IpAddr::V4([192, 168, 1, 5].into());
        assert!(authorize(loopback, None, None, None, None));
        assert!(!authorize(remote, None, None, None, None));
    }

    #[test]
    fn configured_token_accepts_bearer_cookie_or_query() {
        let remote = std::net::IpAddr::V4([192, 168, 1, 5].into());
        assert!(authorize(
            remote,
            Some("secret"),
            None,
            None,
            Some("secret")
        ));
        assert!(authorize(
            remote,
            None,
            Some("secret"),
            None,
            Some("secret")
        ));
        assert!(authorize(
            remote,
            None,
            None,
            Some("secret"),
            Some("secret")
        ));
        assert!(!authorize(
            remote,
            Some("wrong"),
            None,
            None,
            Some("secret")
        ));
        assert!(!authorize(remote, None, None, None, Some("secret")));
    }

    #[test]
    fn cookie_parser_handles_other_cookies_and_whitespace() {
        let headers = HeaderMap::from_iter([(
            header::COOKIE,
            "session=abc; ipkvm_token=secret; other=1".parse().unwrap(),
        )]);
        assert_eq!(cookie_token(&headers), Some("secret"));
    }

    #[test]
    fn query_parser_extracts_token_parameter() {
        let uri: Uri = "/?token=secret&x=1".parse().unwrap();
        assert_eq!(query_token(&uri), Some("secret"));
        let no_token: Uri = "/".parse().unwrap();
        assert_eq!(query_token(&no_token), None);
    }

    #[test]
    fn bearer_parser_requires_prefix_and_valid_header() {
        let headers =
            HeaderMap::from_iter([(header::AUTHORIZATION, "Bearer secret".parse().unwrap())]);
        assert_eq!(bearer_token(&headers), Some("secret"));
        let wrong_prefix =
            HeaderMap::from_iter([(header::AUTHORIZATION, "Basic secret".parse().unwrap())]);
        assert_eq!(bearer_token(&wrong_prefix), None);
    }
}
