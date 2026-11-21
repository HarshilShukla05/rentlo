//! Interpret a shared Google Maps link. Coordinates are taken from a place
//! or explicit pin, never from the camera position of a named place.

use app::Body;
use percent_encoding::percent_decode_str;
use std::time::{Duration, Instant};
use url::Url;

pub async fn resolve_location(text: &str) -> Option<Body> {
    let text = text.trim();
    let mut parts = text.split_whitespace();
    let link = parts.find(|part| part.starts_with("https://"))?;
    if parts.next().is_some() || text.len() > 4096 {
        return None;
    }
    let url = Url::parse(link).ok()?;
    let route = route(&url)?;
    let caption = text[..text.len() - link.len()].trim().to_owned();
    if caption.contains("://") || caption.chars().count() > 80 {
        return None;
    }
    match route {
        Route::Maps => Some(maps(&url)),
        Route::Short | Route::Share => {
            let share = route == Route::Share;
            let result = tokio::task::spawn_blocking(move || {
                let deadline = Instant::now() + Duration::from_secs(3);
                resolve_short(url, &caption, share, |url| {
                    let remaining = deadline.checked_duration_since(Instant::now())?;
                    let agent = ureq::Agent::config_builder()
                        .max_redirects(0)
                        .http_status_as_error(false)
                        .timeout_global(Some(remaining))
                        .build()
                        .new_agent();
                    let response = agent.get(url.as_str()).call().ok()?;
                    if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                        return None;
                    }
                    response.headers().get("location")?.to_str().ok().map(str::to_owned)
                })
            })
            .await;
            Some(result.unwrap_or(Body::UnresolvedMapLink))
        }
        Route::Search => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    Maps,
    Short,
    Share,
    Search,
}

fn route(url: &Url) -> Option<Route> {
    if url.scheme() != "https" || url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let path = url.path();
    match url.host_str()? {
        "maps.app.goo.gl" if path.len() > 1 => Some(Route::Short),
        "goo.gl" if path.starts_with("/maps/") => Some(Route::Short),
        "share.google" if path.len() > 1 => Some(Route::Share),
        "google.com" | "www.google.com" | "maps.google.com" | "www.google.co.in" | "maps.google.co.in" => match path {
            "/share.google" => Some(Route::Share),
            "/search" => Some(Route::Search),
            "/maps" => Some(Route::Maps),
            "/" if matches!(url.host_str(), Some("maps.google.com" | "maps.google.co.in")) => Some(Route::Maps),
            path if path.starts_with("/maps/") => Some(Route::Maps),
            _ => None,
        },
        _ => None,
    }
}

fn resolve_short(mut url: Url, caption: &str, share: bool, mut redirect: impl FnMut(&Url) -> Option<String>) -> Body {
    for _ in 0..5 {
        match route(&url) {
            Some(Route::Maps) => return maps(&url),
            Some(Route::Search) if share => return google_place(&url, caption),
            Some(Route::Short) | Some(Route::Share) => {
                let Some(next) = redirect(&url).and_then(|target| url.join(&target).ok()) else { break };
                url = next;
            }
            _ => break,
        }
    }
    Body::UnresolvedMapLink
}

fn google_place(url: &Url, caption: &str) -> Body {
    let mut query = None;
    let mut is_place = false;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "q" => query = Some(value),
            "kgmid" => is_place = !value.is_empty(),
            _ => {}
        }
    }
    if is_place && let Some(query) = query {
        let name = query.split(',').next().unwrap_or_default().trim();
        if caption.is_empty() || caption.eq_ignore_ascii_case(name) || caption.eq_ignore_ascii_case(&query) {
            return named_area(name);
        }
    }
    Body::UnresolvedMapLink
}

fn maps(url: &Url) -> Body {
    let path = percent_decode_str(url.path()).decode_utf8_lossy();
    if path.starts_with("/maps/dir") {
        return Body::UnresolvedMapLink;
    }
    let mut name = None;
    for (key, value) in url.query_pairs() {
        if matches!(key.as_ref(), "q" | "query") {
            if let Some(pin) = coordinates(&value) {
                return pin;
            }
            if !value.is_empty() {
                name = Some(value);
            }
        }
    }
    // A selected place can carry both a viewport at /@lat,lng and an
    // actual place point at !3dlat!4dlng. Only the latter is its pin.
    if let Some((_, data)) = path.split_once("/data=")
        && let Some((_, point)) = data.rsplit_once("!3d")
        && let Some((lat, rest)) = point.split_once("!4d")
        && let Some(pin) = pin(lat, rest.split('!').next().unwrap_or_default())
    {
        return pin;
    }
    if let Some(path_name) = path.strip_prefix("/maps/place/").or_else(|| path.strip_prefix("/maps/search/")) {
        let place = path_name.split('/').next().unwrap_or_default().replace('+', " ");
        return coordinates(&place).unwrap_or_else(|| named_area(&place));
    }
    match name {
        Some(name) => named_area(&name),
        None => Body::UnresolvedMapLink,
    }
}

fn coordinates(value: &str) -> Option<Body> {
    let (lat, lng) = value.split_once(',')?;
    pin(lat, lng)
}

fn pin(lat: &str, lng: &str) -> Option<Body> {
    let (lat, lng): (f64, f64) = (lat.trim().parse().ok()?, lng.trim().parse().ok()?);
    (lat.is_finite() && lng.is_finite()).then_some(Body::Location { lat, lng })
}

fn named_area(name: &str) -> Body {
    if name.chars().any(char::is_alphabetic) { Body::Area(name.to_owned()) } else { Body::UnresolvedMapLink }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[tokio::test]
    async fn direct_maps_pin_and_place() {
        assert!(matches!(
            resolve_location("Preferred area\nhttps://www.google.com/maps/search/?api=1&query=17.42%2C78.36\n").await,
            Some(Body::Location { lat, lng }) if lat == 17.42 && lng == 78.36
        ));
        assert!(matches!(
            maps(&Url::parse("https://www.google.com/maps/place/Test+District/@17.0,78.0,12z/data=!3d17.42!4d78.36!1sxyz").unwrap()),
            Body::Location { lat, lng } if lat == 17.42 && lng == 78.36
        ));
        assert!(matches!(
            maps(&Url::parse("https://www.google.com/maps/place/Test+District/@17.0,78.0,12z").unwrap()),
            Body::Area(area) if area == "Test District"
        ));
    }

    #[test]
    fn short_share_with_google_place_but_no_point() {
        let start = Url::parse("https://share.google/sample").unwrap();
        let reply = resolve_short(start, "Test District", true, |url| match url.host_str()? {
            "share.google" => Some("https://www.google.com/share.google?q=opaque".into()),
            "www.google.com" => Some("/search?q=Test+District%2C+Hyderabad&kgmid=%2Fg%2Fxyz".into()),
            _ => None,
        });
        assert!(matches!(reply, Body::Area(area) if area == "Test District"));
    }

    #[test]
    fn redirects_and_route_shares_are_bounded() {
        let start = Url::parse("https://maps.app.goo.gl/sample").unwrap();
        let mut requests = 0;
        let reply = resolve_short(start.clone(), "", false, |_| {
            requests += 1;
            Some("https://127.0.0.1/maps?q=17.42,78.36".into())
        });
        assert_eq!(requests, 1);
        assert!(matches!(reply, Body::UnresolvedMapLink));
        let reply = resolve_short(start, "", false, |_| {
            Some("https://www.google.com/maps/dir/Hyderabad/Pune/@17,78,10z".into())
        });
        assert!(matches!(reply, Body::UnresolvedMapLink));
    }
}
