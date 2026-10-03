//! Canvas LMS source (read-only).
//!
//! POLICY GUARDRAILS — these are product requirements, not suggestions:
//! - Personal-token mode is for the token owner's own use / development only. Instructure's
//!   OAuth docs state that asking other users to manually generate a token for your app
//!   violates the Canvas API Policy; multi-user distribution must use OAuth2 with an
//!   institution-issued Developer Key (future work).
//! - Only HTTP GET. The client must make it impossible to issue POST/PUT/DELETE
//!   (no submission, quiz answers, discussion posts, "mark done", etc.).
//! - Endpoint allow-list (all under `{base_url}/api/v1`):
//!
//!   ```text
//!   GET /users/self
//!   GET /courses?enrollment_state=active&include[]=term&include[]=syllabus_body&include[]=concluded&per_page=100
//!   GET /courses/:id/tabs
//!   GET /courses/:id/modules?include[]=items&include[]=content_details&per_page=100
//!   GET /courses/:id/modules/:module_id/items?include[]=content_details&per_page=100 (fallback)
//!   GET /courses/:id/files?per_page=100          (skip if Files tab hidden / 401 / 403)
//!   GET /courses/:id/files/:file_id               (metadata for module file items)
//!   GET /courses/:id/pages?per_page=100 and /courses/:id/pages/:url_or_id (body)
//!   GET /courses/:id/assignments?per_page=100     (name + due_at + html_url ONLY; never
//!                                                  store assignment descriptions)
//!   GET /announcements?context_codes[]=course_:id&start_date=…&per_page=100
//!   GET /planner/items?start_date=…&end_date=…&per_page=100
//!   file download URL from the file object (follow redirects; never forward the
//!     Authorization header to another host)
//!   ```
//!
//! - Canvas is never called from the MCP server; only `pagelamp sync` calls this crate.
//! - Pagination: follow `Link: <…>; rel="next"` as an opaque URL (must stay on base host).
//! - Throttling: at most 2 concurrent requests; if `X-Rate-Limit-Remaining` < 100 slow down;
//!   on 403 with body containing "Rate Limit Exceeded" or on 429, exponential backoff
//!   (1s, 2s, 4s … max 5 tries).
//! - Tokens come from `pagelamp_core::secrets` and must never be logged.
//!
//! Mapping to the store (ids per `pagelamp_core::model` conventions, source id
//! `canvas:<host>`): courses → `CourseUpsert` (term dates from `term.start_at/end_at`,
//! syllabus_body → text); modules → `Module` (week_hint via `timeline::parse_week_hint`);
//! module items of type File/Page/ExternalUrl + course files + pages → `MaterialUpsert`;
//! announcements → `MaterialUpsert { kind: Announcement }` indexed via `ingest::index_html`;
//! assignments/quizzes due dates + planner items → `Event`s.

mod api;
mod endpoint;
mod json;
mod map;
mod sync;
mod transport;

#[cfg(test)]
mod tests_sync;

use std::path::{Path, PathBuf};

use pagelamp_core::source::{ProgressFn, SourceError};

use crate::api::Api;
use crate::transport::{CanvasTransport, RetryPolicy, TokenTransport};

/// Connection settings for token mode. `Debug` is implemented by hand so the token can never
/// end up in logs.
#[derive(Clone)]
pub struct CanvasConfig {
    /// e.g. "https://lms.example.edu" (no trailing slash, no /api/v1); see `normalize_base_url`.
    pub base_url: String,
    pub token: String,
}

impl std::fmt::Debug for CanvasConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanvasConfig")
            .field("base_url", &self.base_url)
            .field("token", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct SyncOptions {
    /// Download files and index their text. When false (the default for Canvas — downloads
    /// count as views, see crate docs), files are recorded as NotDownloaded.
    pub download_files: bool,
    /// Skip files larger than this (bytes). Default 50 MB.
    pub max_file_bytes: u64,
    /// Where downloaded files are cached (normally `paths::files_dir()`).
    pub files_dir: PathBuf,
    /// Only sync these course ids/codes (empty = all active courses).
    pub only_courses: Vec<String>,
    /// With `download_files`: download only these files (material ids), e.g. the syllabus
    /// candidates the student chose (D46); every other file is handled as in a sync without
    /// downloads. `None`: all of them.
    pub only_files: Option<std::collections::BTreeSet<String>>,
    /// How downloaded files are read (`ingest::Extractor`; one per sync).
    pub extractor: pagelamp_core::ingest::Extractor,
}

#[derive(Clone, Debug, Default)]
pub struct SyncReport {
    pub courses: usize,
    pub modules: usize,
    pub materials: usize,
    pub files_downloaded: usize,
    pub files_indexed: usize,
    pub events: usize,
    /// Non-fatal problems (e.g. "DEMO101: Files tab hidden, used module items only").
    pub warnings: Vec<String>,
    /// One line per course synced (for the summary).
    pub course_summaries: Vec<pagelamp_core::source::CourseSyncSummary>,
    /// HTTP requests made to Canvas and file storage.
    pub requests: u64,
}

/// Validate and normalise a user-entered Canvas address to `scheme://host[:port]`. Only the
/// address itself is accepted ("https://lms.example.edu", a trailing slash is fine, a missing
/// scheme means https); a path or query ("…/courses/1", "…/?x=1") is rejected so a pasted
/// course link isn't silently reinterpreted. https only, except http for localhost (tests).
///
/// Errors never repeat the input (a student may paste a secret calendar-feed link here); at
/// most the host name is shown.
pub fn normalize_base_url(input: &str) -> Result<String, SourceError> {
    let trimmed = input.trim();
    // A pasted token ("7~AbC…") is not an address: never look it up as a host name.
    let token_shaped = trimmed.contains('~');
    let invalid = || {
        SourceError::other(
            "That is not a Canvas address. Enter just the address, like https://lms.example.edu",
        )
    };
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    if token_shaped {
        return Err(invalid());
    }
    // The host as typed (before the URL parser turns "7" into 0.0.0.7), without a trailing
    // dot ("canvas."): a school's Canvas has a full domain name; a single word is a typo or
    // something pasted into the wrong field.
    let typed_host = with_scheme
        .split_once("://")
        .map_or("", |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    let typed_host = if typed_host.starts_with('[') {
        typed_host // IPv6 literal
    } else {
        typed_host.split(':').next().unwrap_or("")
    };
    let typed_host = typed_host.trim_end_matches('.');
    if !typed_host.contains('.') && !typed_host.starts_with('[') && typed_host != "localhost" {
        return Err(invalid());
    }
    let url = url::Url::parse(&with_scheme).map_err(|_| invalid())?;
    let host = url
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(invalid)?;
    // A school's Canvas has a full domain name; a single word ("canvas", "7") is a typo or
    // something pasted into the wrong field, and would only cause a pointless DNS lookup.
    let single_label = !host.contains('.') && !host.contains(':') && host != "localhost";
    if single_label {
        return Err(invalid());
    }
    let only_address =
        matches!(url.path(), "" | "/") && url.query().is_none() && url.fragment().is_none();
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid());
    }
    if !only_address {
        return Err(SourceError::other(format!(
            "That is a link to a page, not a Canvas address. Enter just the address, like \
             https://{host}"
        )));
    }
    let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        _ => return Err(invalid()),
    }
    Ok(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

/// Source id for a (normalised) Canvas base URL: `canvas:<host>` (plus `:<port>` if any).
pub fn source_id(base_url: &str) -> String {
    let rest = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or(rest);
    format!("canvas:{}", authority.to_ascii_lowercase())
}

/// Validate a token by calling `GET /api/v1/users/self`; returns the user's display name.
pub async fn check_token(config: &CanvasConfig) -> Result<String, SourceError> {
    let api = token_api(config, RetryPolicy::default())?;
    let user: json::User = api
        .get_one(endpoint::Endpoint::UsersSelf)
        .await
        .map_err(probe_error)?;
    Ok(user
        .name
        .or(user.short_name)
        .unwrap_or_else(|| "Canvas user".into()))
}

/// Where `sync` keeps the downloaded files of a course: `<files_dir>/<CODE>-<canvas id>`
/// (sanitised; always a direct child of `files_dir`).
pub fn course_files_dir(files_dir: &Path, code: Option<&str>, external_id: &str) -> PathBuf {
    files_dir.join(sync::course_dir_name(code, external_id))
}

/// The end of every download directory name a course has had (`-<canvas id>`, sanitised),
/// whatever its course code was at the time.
pub fn course_dir_suffix(external_id: &str) -> String {
    format!("-{}", sync::safe_name(external_id, 40))
}

/// How a failed `/users/self` probe is reported (adding a source, and the first request of
/// every sync). An answer that isn't Canvas's — a web page or JSON without a user id, a
/// redirect (e.g. to a login page), a 403 or a "not authorized" 401, another 3xx/4xx
/// status — means there is no Canvas at that address (or it moved), not an internal error.
pub(crate) fn probe_error(err: transport::CanvasError) -> SourceError {
    use transport::CanvasError;
    match err {
        CanvasError::BadResponse(_) | CanvasError::Forbidden => sync::no_canvas_here(),
        CanvasError::Http(status) if (300..500).contains(&status) => sync::no_canvas_here(),
        other => sync::required(other, "your Canvas account"),
    }
}

/// Full sync of the active courses into the store at `db_path`. The source row must already
/// exist. The DB is opened per unit of work inside `spawn_blocking` (the future is `Send`);
/// store writes happen in short transactions per course; the caller records the outcome with
/// `Store::record_sync`. Files are downloaded only when `options.download_files` (Canvas
/// counts a download as viewing the file).
pub async fn sync(
    db_path: &Path,
    config: &CanvasConfig,
    options: &SyncOptions,
    progress: ProgressFn<'_>,
) -> Result<SyncReport, SourceError> {
    let api = token_api(config, RetryPolicy::default())?;
    // Same id the App stored when the source was added (from the normalised URL).
    let source_id = source_id(&normalize_base_url(&config.base_url)?);
    sync_with(&api, db_path, &source_id, options, progress).await
}

/// The sync over any transport (tests use short retry delays).
pub(crate) async fn sync_with<T: CanvasTransport>(
    api: &Api<T>,
    db_path: &Path,
    source_id: &str,
    options: &SyncOptions,
    progress: ProgressFn<'_>,
) -> Result<SyncReport, SourceError> {
    sync::Syncer {
        api,
        db: db_path,
        source_id,
        options,
        progress,
        now: chrono::Utc::now(),
    }
    .run()
    .await
}

fn token_api(
    config: &CanvasConfig,
    retry: RetryPolicy,
) -> Result<Api<TokenTransport>, SourceError> {
    let base = normalize_base_url(&config.base_url)?;
    let base = url::Url::parse(&base).map_err(|_| SourceError::other("invalid Canvas URL"))?;
    if config.token.trim().is_empty() {
        return Err(SourceError::auth("No Canvas access token was given."));
    }
    let transport = TokenTransport::new(base.clone(), &config.token, retry)
        .map_err(|e| SourceError::auth(format!("The Canvas access token looks wrong ({e}).")))?;
    Ok(Api::new(transport, base))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_address_without_canvas_is_not_found_not_an_internal_error() {
        use pagelamp_core::model::SourceErrorKind;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let answers = [
            ResponseTemplate::new(200).set_body_string("<html>Demo University</html>"),
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "ok"})),
            ResponseTemplate::new(302).insert_header("Location", "https://sso.example.edu/login"),
            ResponseTemplate::new(403).set_body_string("Forbidden"),
            ResponseTemplate::new(401)
                .set_body_string(r#"{"status":"unauthorized","errors":[{"message":"user not authorized to perform that action"}]}"#),
            ResponseTemplate::new(405),
        ];
        for answer in answers {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/users/self"))
                .respond_with(answer)
                .mount(&server)
                .await;
            let config = CanvasConfig {
                base_url: server.uri(),
                token: "demo-token".into(),
            };
            let err = check_token(&config).await.unwrap_err();
            assert_eq!(err.kind, SourceErrorKind::NotFound, "{}", err.message);
            assert!(err.message.contains("No Canvas"), "{}", err.message);
        }
        // A server error is not "no Canvas here".
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let config = CanvasConfig {
            base_url: server.uri(),
            token: "demo-token".into(),
        };
        let err = check_token(&config).await.unwrap_err();
        assert_ne!(err.kind, SourceErrorKind::NotFound, "{}", err.message);
    }

    #[test]
    fn address_errors_never_repeat_the_input() {
        for input in [
            "https://q.example.edu/feeds/calendars/user_S3CR3T.ics",
            "webcal://q.example.edu/feeds/calendars/user_S3CR3T.ics",
            "https://student:S3CR3T@q.example.edu",
            "not a url S3CR3T",
        ] {
            let err = normalize_base_url(input).unwrap_err();
            assert!(!err.message.contains("S3CR3T"), "{}", err.message);
            assert!(
                err.message.contains("Enter just the address"),
                "{}",
                err.message
            );
        }
        for bad in [
            "7~AbCdEfGhIjKlMnOpQrStUvWxYz",
            "https://7~AbCdEfGhIjKl",
            "canvas",
            "canvas.",
            "https://intranet/",
            "7",
            "https://7:8443",
        ] {
            let err = normalize_base_url(bad).unwrap_err();
            assert!(!err.message.contains("AbCdEf"), "{}", err.message);
        }
        let err = normalize_base_url("https://q.example.edu/courses/1").unwrap_err();
        assert!(
            err.message.ends_with("like https://q.example.edu"),
            "{}",
            err.message
        );
    }

    #[test]
    fn base_urls_are_normalised() {
        let ok = |input: &str| normalize_base_url(input).unwrap();
        assert_eq!(ok("lms.example.edu"), "https://lms.example.edu");
        assert_eq!(ok(" https://lms.example.edu/ "), "https://lms.example.edu");
        assert_eq!(ok("https://LMS.Example.edu"), "https://lms.example.edu");
        assert_eq!(
            ok("https://lms.example.edu:8443/"),
            "https://lms.example.edu:8443"
        );
        assert_eq!(ok("http://127.0.0.1:9999"), "http://127.0.0.1:9999");
        for bad in [
            "",
            "http://lms.example.edu",
            "ftp://lms.example.edu",
            "https://user:pass@lms.example.edu",
            "https://",
            "not a url at all",
            "https://lms.example.edu/courses/1",
            "https://lms.example.edu/?x=y",
            "https://lms.example.edu/#top",
            "lms.example.edu/login",
        ] {
            assert!(normalize_base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn source_ids_keep_host_and_port() {
        assert_eq!(
            source_id("https://lms.example.edu"),
            "canvas:lms.example.edu"
        );
        assert_eq!(source_id("http://127.0.0.1:9999"), "canvas:127.0.0.1:9999");
    }
}
