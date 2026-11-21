use app::{Error, Settings, Web};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::Utc;
use domain::{City, GeoPoint, MatchId, area};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use store::PgStore;
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

#[derive(Clone)]
struct App {
    web: Arc<Web>,
    store: PgStore,
    site: String,
    secure: bool,
    whatsapp: Option<String>,
}

pub async fn serve(
    store: PgStore,
    settings: Settings,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await.map_err(|e| e.to_string())?;
    let root = PathBuf::from(std::env::var("RENTLO_WEB_ROOT").unwrap_or_else(|_| "web/dist".into()));
    let index = root.join("index.html");
    if !index.is_file() {
        return Err("web assets are missing. Build web/ or set RENTLO_WEB_ROOT".into());
    }
    let secure =
        !settings.public_url.starts_with("http://localhost:") && !settings.public_url.starts_with("http://127.0.0.1:");
    let whatsapp = settings
        .bot_phone
        .as_deref()
        .filter(|phone| (8..=15).contains(&phone.len()) && phone.bytes().all(|b| b.is_ascii_digit()))
        .map(|phone| format!("https://wa.me/{phone}"));
    let app = App {
        web: Arc::new(Web::new(Arc::new(store.clone()), settings.clone())),
        store,
        site: settings.public_url,
        secure,
        whatsapp,
    };
    let api = Router::new()
        .route("/places", get(places))
        .route("/map/activity", get(activity))
        .route("/session", post(sign_in))
        .route("/session/sign-out", post(sign_out))
        .route("/me", get(me))
        .route("/me/participation", put(participation))
        .route("/me/matches", get(matches))
        .route("/me/matches/{id}/decision", post(decide))
        .layer(DefaultBodyLimit::max(4096))
        .layer(always(header::CACHE_CONTROL, "no-store"));
    let routes = Router::new()
        .nest("/api/v1", api)
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/readyz", get(ready))
        .route("/privacy", get(|| async { Html(include_str!("privacy.html")) }))
        .route_service("/", ServeFile::new(&index))
        .route_service("/m/{token}", ServeFile::new(index))
        .nest_service("/assets", ServeDir::new(root.join("assets")))
        .with_state(app)
        .layer(always(
            header::CONTENT_SECURITY_POLICY,
            concat!(
                "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; ",
                "img-src 'self' data: https://tile.openstreetmap.org; ",
                "connect-src 'self'; ",
                "base-uri 'none'; frame-ancestors 'none'; form-action 'self'"
            ),
        ))
        .layer(always(header::REFERRER_POLICY, "no-referrer"))
        .layer(always(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(always(HeaderName::from_static("permissions-policy"), "geolocation=(), camera=(), microphone=()"));
    tracing::info!(%port, "Rentlo map listening");
    axum::serve(listener, routes).with_graceful_shutdown(shutdown).await.map_err(|e| e.to_string())
}

fn always(name: HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value))
}

async fn ready(State(app): State<App>) -> StatusCode {
    if app.store.ping().await.is_ok() { StatusCode::NO_CONTENT } else { StatusCode::SERVICE_UNAVAILABLE }
}

#[derive(Serialize)]
struct Place {
    name: String,
    point: GeoPoint,
}

#[derive(Serialize)]
struct Places {
    cities: Vec<Place>,
    areas: Vec<Place>,
    whatsapp: Option<String>,
}

#[derive(Deserialize)]
struct CityQuery {
    city: City,
}

async fn places(State(app): State<App>, Query(query): Query<CityQuery>) -> Json<Places> {
    let cities = City::known().map(|(city, point)| Place { name: city.to_string(), point }).collect();
    let areas = area::in_city(&query.city).map(|area| Place { name: area.name.into(), point: area.centre }).collect();
    Json(Places { cities, areas, whatsapp: app.whatsapp })
}

async fn activity(
    State(app): State<App>,
    Query(query): Query<CityQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    Ok(Json(app.web.area_activity(session(&app, &headers)?, &query.city, Utc::now()).await?).into_response())
}

#[derive(Deserialize)]
struct Access {
    token: String,
}

async fn sign_in(State(app): State<App>, headers: HeaderMap, Json(input): Json<Access>) -> Result<Response, ApiError> {
    same_origin(&app, &headers)?;
    if !valid_token(&input.token) {
        return Err(Error::Unauthorized.into());
    }
    let token = app.web.sign_in(&input.token, session(&app, &headers).ok(), Utc::now()).await?;
    cookie_response(&app, &token, app::policy::session_ttl().num_seconds())
}

async fn sign_out(State(app): State<App>, headers: HeaderMap) -> Result<Response, ApiError> {
    same_origin(&app, &headers)?;
    if let Ok(token) = session(&app, &headers) {
        app.web.sign_out(token).await?;
    }
    cookie_response(&app, "", 0)
}

fn cookie_response(app: &App, token: &str, max_age: i64) -> Result<Response, ApiError> {
    let secure = if app.secure { "; Secure" } else { "" };
    let cookie = format!("{}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}", cookie_name(app));
    let cookie = HeaderValue::from_str(&cookie)
        .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Could not start a session"))?;
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

async fn me(State(app): State<App>, headers: HeaderMap) -> Result<Response, ApiError> {
    Ok(Json(app.web.me(session(&app, &headers)?, Utc::now()).await?).into_response())
}

async fn matches(State(app): State<App>, headers: HeaderMap) -> Result<Response, ApiError> {
    Ok(Json(app.web.matches(session(&app, &headers)?, Utc::now()).await?).into_response())
}

#[derive(Deserialize)]
struct Participation {
    paused: bool,
}

async fn participation(
    State(app): State<App>,
    headers: HeaderMap,
    Json(input): Json<Participation>,
) -> Result<StatusCode, ApiError> {
    same_origin(&app, &headers)?;
    app.web.set_paused(session(&app, &headers)?, input.paused, Utc::now()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Decision {
    accept: bool,
}

async fn decide(
    State(app): State<App>,
    Path(id): Path<MatchId>,
    headers: HeaderMap,
    Json(input): Json<Decision>,
) -> Result<Response, ApiError> {
    same_origin(&app, &headers)?;
    Ok(Json(app.web.decide(session(&app, &headers)?, id, input.accept, Utc::now()).await?).into_response())
}

fn same_origin(app: &App, headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.get(header::ORIGIN).and_then(|value| value.to_str().ok()) != Some(app.site.as_str()) {
        return Err(ApiError(StatusCode::FORBIDDEN, "Open Rentlo directly from its own page"));
    }
    Ok(())
}

fn cookie_name(app: &App) -> &'static str {
    if app.secure { "__Host-rentlo_session" } else { "rentlo_session" }
}

fn session<'a>(app: &App, headers: &'a HeaderMap) -> Result<&'a str, ApiError> {
    let token = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .find(|(name, _)| *name == cookie_name(app))
        .map(|(_, token)| token)
        .filter(|token| valid_token(token));
    token.ok_or_else(|| Error::Unauthorized.into())
}

fn valid_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

struct ApiError(StatusCode, &'static str);

impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        match error {
            Error::Unauthorized => Self(StatusCode::UNAUTHORIZED, "Open your private map link from WhatsApp"),
            Error::NotFound => Self(StatusCode::NOT_FOUND, "This item is no longer available"),
            Error::Invalid(_) => Self(StatusCode::CONFLICT, "This action is unavailable for your profile"),
            Error::Store(error) => {
                tracing::error!(%error, "map database operation failed");
                Self(StatusCode::INTERNAL_SERVER_ERROR, "Rentlo could not complete the request")
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Message {
            message: &'static str,
        }
        (self.0, Json(Message { message: self.1 })).into_response()
    }
}
