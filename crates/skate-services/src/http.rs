use super::*;
use reqwest::{
    Client, Url,
    header::{HeaderName, HeaderValue},
    redirect::Policy,
};

pub(super) fn client() -> Result<Client, ServiceError> {
    Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|_| error(ErrorCode::Http, "cannot configure TLS HTTP client"))
}
pub(super) fn origin(input: &str) -> Result<String, ServiceError> {
    let url = Url::parse(input).map_err(|_| error(ErrorCode::Invalid, "invalid HTTP URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(error(
            ErrorCode::Denied,
            "HTTP requires http/https without URL credentials or fragments",
        ));
    }
    Ok(url.origin().ascii_serialization())
}
pub(super) fn grant_origin(input: &str) -> Result<String, ServiceError> {
    let value = origin(input)?;
    let parsed = Url::parse(input).map_err(|_| error(ErrorCode::Invalid, "invalid HTTP origin"))?;
    if parsed.path() != "/" || parsed.query().is_some() {
        return Err(error(
            ErrorCode::Invalid,
            "HTTP grants must be origins without paths or queries",
        ));
    }
    Ok(value)
}
pub(super) fn validate(grants: &Grants, request: &HttpRequest) -> Result<(), ServiceError> {
    if !grants.http_origins.contains(&origin(&request.url)?) {
        return Err(error(
            ErrorCode::Denied,
            "HTTP origin is not granted by the administrator",
        ));
    }
    if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]
        .contains(&request.method.as_str())
    {
        return Err(error(ErrorCode::Denied, "unsupported HTTP method"));
    }
    if request.headers.len() > 32 || request.url.len() > 8192 {
        return Err(error(
            ErrorCode::TooLarge,
            "HTTP header or URL budget exceeded",
        ));
    }
    for (name, value) in &request.headers {
        if [
            "host",
            "connection",
            "transfer-encoding",
            "content-length",
            "upgrade",
            "proxy-authorization",
        ]
        .iter()
        .any(|n| name.eq_ignore_ascii_case(n))
        {
            return Err(error(
                ErrorCode::Denied,
                "HTTP framing, routing and proxy headers are host-owned",
            ));
        }
        HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| error(ErrorCode::Invalid, "invalid HTTP header name"))?;
        HeaderValue::from_str(value)
            .map_err(|_| error(ErrorCode::Invalid, "invalid HTTP header value"))?;
    }
    Ok(())
}
pub(super) async fn run(
    mut rx: tokio::sync::mpsc::Receiver<Job>,
    tx: mpsc::SyncSender<Completion>,
    limits: Limits,
    client: Client,
) {
    let mut active = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            job = rx.recv(), if active.len() < limits.http_workers => {
                match job {
                    Some(job) => {
                        let (client, limits, tx) = (client.clone(), limits.clone(), tx.clone());
                        active.spawn(async move {
                            let result = execute(&client, &limits, &job).await;
                            let _ = tx.try_send(job.complete(result));
                        });
                    },
                    None => break,
                }
            }
            _ = active.join_next(), if !active.is_empty() => {}
        }
    }
    while active.join_next().await.is_some() {}
}
async fn execute(client: &Client, limits: &Limits, job: &Job) -> Result<Response, ServiceError> {
    job.check()?;
    let Operation::Http { request } = &job.operation else {
        return Err(error(ErrorCode::Invalid, "not an HTTP operation"));
    };
    let operation = async {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| error(ErrorCode::Invalid, "invalid method"))?;
        let mut outgoing = client
            .request(method, &request.url)
            .body(request.body.clone());
        for (name, value) in &request.headers {
            outgoing = outgoing.header(name, value);
        }
        let mut response = outgoing.send().await.map_err(|_| {
            error(
                ErrorCode::Http,
                "HTTP connection or TLS verification failed",
            )
        })?;
        if response
            .content_length()
            .is_some_and(|length| length > limits.response_bytes as u64)
        {
            return Err(error(
                ErrorCode::TooLarge,
                "HTTP response exceeds byte budget",
            ));
        }
        let status = response.status().as_u16();
        let mut headers = BTreeMap::new();
        let mut bytes = 0;
        for (name, value) in response.headers() {
            bytes += name.as_str().len() + value.as_bytes().len();
            if bytes > limits.response_bytes || headers.len() >= 64 {
                return Err(error(
                    ErrorCode::TooLarge,
                    "HTTP response headers exceed budget",
                ));
            }
            headers.insert(
                name.as_str().into(),
                value.to_str().unwrap_or("<binary>").into(),
            );
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| error(ErrorCode::Http, "HTTP body read failed"))?
        {
            bytes += chunk.len();
            if bytes > limits.response_bytes {
                return Err(error(
                    ErrorCode::TooLarge,
                    "HTTP response exceeds byte budget",
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Response::Http {
            status,
            headers,
            body,
        })
    };
    tokio::pin!(operation);
    let mut poll = tokio::time::interval(Duration::from_millis(5));
    loop {
        tokio::select! {
            biased;
            _ = poll.tick() => job.check()?,
            result = &mut operation => { job.check()?; return result; }
        }
    }
}
