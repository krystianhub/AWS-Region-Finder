use std::{
    borrow::Cow,
    net::IpAddr,
    sync::{Arc, RwLock},
};

use ipnet::{Ipv4Net, Ipv6Net};
use iprange::IpRange;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use worker::*;

const CF_CACHE_STATUS_HEADER: &str = "cf-cache-status";
const AWS_RANGE_URL: &str = "https://ip-ranges.amazonaws.com/ip-ranges.json";

lazy_static! {
    static ref INSTANCE_ID: String = Uuid::new_v4().to_string();
    static ref CORS_HEADERS: Cors = Cors::new()
        .with_origins(["*"])
        .with_methods([Method::Get, Method::Head, Method::Options])
        .with_max_age(86400);
    static ref AWS_RESPONSE: RwLock<Option<Arc<AWSResponse>>> = RwLock::new(None);
}

#[derive(Debug)]
pub struct AWSResponse {
    pub ranges: AWSIpRanges,
    pub cf_cache_status: String,
}

#[derive(Debug, Deserialize)]
pub struct AWSIpRanges {
    pub prefixes: Vec<Ipv4Prefix>,
    #[serde(rename = "ipv6_prefixes")]
    pub ipv6_prefixes: Vec<Ipv6Prefix>,
}

#[derive(Debug, Deserialize)]
pub struct Ipv4Prefix {
    #[serde(rename = "ip_prefix")]
    pub ip_prefix: String,
    #[serde(skip)]
    pub ipv4_prefix_compute: IpRange<Ipv4Net>,
    pub region: String,
    pub service: String,
    #[serde(rename = "network_border_group")]
    pub network_border_group: String,
}

#[derive(Debug, Deserialize)]
pub struct Ipv6Prefix {
    #[serde(rename = "ipv6_prefix")]
    pub ipv6_prefix: String,
    #[serde(skip)]
    pub ipv6_prefix_compute: IpRange<Ipv6Net>,
    pub region: String,
    pub service: String,
    #[serde(rename = "network_border_group")]
    pub network_border_group: String,
}

#[derive(Serialize)]
pub struct VersionJSON<'a> {
    pub instance_id: &'a str,
    pub local_version: &'a str,
    pub workers_version: String,
}

#[derive(Debug, Serialize)]
pub struct APIResponse<'a> {
    #[serde(rename = "requested_ip")]
    pub requested_ip: &'a str,
    #[serde(rename = "cache_status")]
    pub cache_status: &'a str,
    pub matches: Vec<APIMatch<'a>>,
}

#[derive(Debug, Serialize)]
pub struct APIMatch<'a> {
    #[serde(rename = "ip_prefix")]
    pub ip_prefix: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    #[serde(rename = "network_border_group")]
    pub network_border_group: &'a str,
}

pub async fn fetch_aws_ranges() -> Result<(Arc<AWSResponse>, bool)> {
    let mut aws_response_storage: Option<Arc<AWSResponse>> = {
        let read_lock = AWS_RESPONSE.read().unwrap();
        read_lock.as_ref().map(Arc::clone)
    };

    let is_local = aws_response_storage.is_some();

    if aws_response_storage.is_none() {
        // Fetch
        let mut fetch_options = RequestInit::default();
        fetch_options.cf.cache_everything = Some(true);
        fetch_options.cf.cache_ttl = Some(3600);

        let fetch_request = Request::new_with_init(AWS_RANGE_URL, &fetch_options)?;
        let mut fetch_request = Fetch::Request(fetch_request).send().await?;
        let mut ranges: AWSIpRanges = fetch_request.json().await?;

        // Compute all ranges
        ranges.prefixes.iter_mut().for_each(|range| {
            range.ipv4_prefix_compute = [range.ip_prefix.parse::<Ipv4Net>().unwrap()]
                .into_iter()
                .collect();
        });
        ranges.ipv6_prefixes.iter_mut().for_each(|range| {
            range.ipv6_prefix_compute = [range.ipv6_prefix.parse::<Ipv6Net>().unwrap()]
                .into_iter()
                .collect();
        });

        let response_headers = fetch_request.headers();
        let cf_header = response_headers.get(CF_CACHE_STATUS_HEADER)?;

        let aws_response = AWSResponse {
            ranges,
            cf_cache_status: cf_header.unwrap_or_else(|| "UNKNOWN".to_owned()),
        };

        let aws_resp = Arc::new(aws_response);

        let mut write_lock = AWS_RESPONSE.write().unwrap();
        write_lock.replace(Arc::clone(&aws_resp));

        aws_response_storage.replace(aws_resp);
    }

    let aws_response_storage = aws_response_storage.unwrap();

    Ok((aws_response_storage, is_local))
}

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    // Optionally, use the Router to handle matching endpoints, use ":name" placeholders, or "*name"
    // catch-alls to match on specific patterns. Alternatively, use `Router::with_data(D)` to
    // provide arbitrary data that will be accessible in each route via the `ctx.data()` method.
    let router = Router::with_data(INSTANCE_ID.as_str());

    // Add as many routes as your Worker needs! Each route will get a `Request` for handling HTTP
    // functionality and a `RouteContext` which you can use to  and get route parameters and
    // Environment bindings like KV Stores, Durable Objects, Secrets, and Variables.
    router
        .get_async("/", |req, _| async move {
            let request_url = match req.url() {
                Ok(url) => url,
                Err(err) => {
                    console_error!("Unknown URL parse error: {:?}", err);
                    return Response::error("Unknown error", 500)?.with_cors(&CORS_HEADERS);
                }
            };

            let ip_param: Option<Cow<str>> =
                request_url.query_pairs().find(|i| i.0 == "ip").map(|i| i.1);

            let ip_param = match ip_param {
                Some(ip_param) => ip_param,
                None => return Response::error(r#""ip" parameter is missing!"#, 400)?.with_cors(&CORS_HEADERS),
            };

            if ip_param.is_empty() {
                return Response::error(r#""ip" parameter is empty!"#, 400)?.with_cors(&CORS_HEADERS);
            }

            let ip_address = match ip_param.parse::<IpAddr>() {
                Ok(ip_param) => ip_param,
                Err(err) => {
                    console_error!("IP parameter is not valid: {:?}", err);
                    return Response::error(r#""ip" parameter is not a valid IP address!"#, 400)?.with_cors(&CORS_HEADERS);
                }
            };

            let (aws_response, is_local) = match fetch_aws_ranges().await {
                Ok(aws_response) => aws_response,
                Err(err) => {
                    console_error!("Unable to fetch AWS ranges: {:?}", err);
                    return Response::error("Unable to fetch AWS ranges", 500)?.with_cors(&CORS_HEADERS);
                }
            };

            let cache_status = if is_local {
                "LOCAL"
            } else {
                aws_response.cf_cache_status.as_str()
            };

            // Check if we have matches against ip_address value
            let matches = match ip_address {
                IpAddr::V4(ipv4) => aws_response
                    .ranges
                    .prefixes
                    .iter()
                    .filter(|x| x.ipv4_prefix_compute.contains(&ipv4))
                    .map(|x| APIMatch {
                        ip_prefix: &x.ip_prefix,
                        region: &x.region,
                        service: &x.service,
                        network_border_group: &x.network_border_group,
                    })
                    .collect::<Vec<_>>(),
                IpAddr::V6(ipv6) => aws_response
                    .ranges
                    .ipv6_prefixes
                    .iter()
                    .filter(|x| x.ipv6_prefix_compute.contains(&ipv6))
                    .map(|x| APIMatch {
                        ip_prefix: &x.ipv6_prefix,
                        region: &x.region,
                        service: &x.service,
                        network_border_group: &x.network_border_group,
                    })
                    .collect::<Vec<_>>(),
            };

            let api_response = APIResponse {
                requested_ip: &ip_param,
                cache_status,
                matches,
            };

            Response::from_json(&api_response)?.with_cors(&CORS_HEADERS)
        })
        .get("/version", |_, ctx| {
            let local_version = env!("CARGO_PKG_VERSION");
            let workers_version = ctx.var("WORKERS_RS_VERSION")?.to_string();

            let version_response = VersionJSON {
                instance_id: ctx.data,
                local_version,
                workers_version,
            };

            Response::from_json(&version_response)?.with_cors(&CORS_HEADERS)
        })
        .run(req, env)
        .await
}
