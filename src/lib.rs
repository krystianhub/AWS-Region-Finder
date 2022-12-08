use std::{
    borrow::Cow,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
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
        let ranges: AWSIpRanges = fetch_request.json().await?;

        let response_headers = fetch_request.headers();
        let cf_header = response_headers.get(CF_CACHE_STATUS_HEADER)?;
        let cf_cache_status = cf_header.unwrap_or_else(|| "UNKNOWN".to_owned());

        let aws_response = calculate_aws_response(ranges, cf_cache_status);
        let aws_response = Arc::new(aws_response);

        let mut write_lock = AWS_RESPONSE.write().unwrap();
        write_lock.replace(Arc::clone(&aws_response));

        aws_response_storage.replace(aws_response);
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
                None => {
                    return Response::error(r#""ip" parameter is missing!"#, 400)?
                        .with_cors(&CORS_HEADERS)
                }
            };

            if ip_param.is_empty() {
                return Response::error(r#""ip" parameter is empty!"#, 400)?
                    .with_cors(&CORS_HEADERS);
            }

            let ip_address = match ip_param.parse::<IpAddr>() {
                Ok(ip_param) => ip_param,
                Err(err) => {
                    console_error!("IP parameter is not valid: {:?}", err);
                    return Response::error(r#""ip" parameter is not a valid IP address!"#, 400)?
                        .with_cors(&CORS_HEADERS);
                }
            };

            let (aws_response, is_local) = match fetch_aws_ranges().await {
                Ok(aws_response) => aws_response,
                Err(err) => {
                    console_error!("Unable to fetch AWS ranges: {:?}", err);
                    return Response::error("Unable to fetch AWS ranges", 500)?
                        .with_cors(&CORS_HEADERS);
                }
            };

            let cache_status = if is_local {
                "LOCAL"
            } else {
                aws_response.cf_cache_status.as_str()
            };

            // Check if we have matches against ip_address value
            let matches = ip_match(&aws_response.ranges, &ip_address);

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

fn calculate_aws_response(mut ranges: AWSIpRanges, cf_cache_status: String) -> AWSResponse {
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

    AWSResponse {
        ranges,
        cf_cache_status,
    }
}

fn ip_match<'a>(aws_ranges: &'a AWSIpRanges, ip_address: &'a IpAddr) -> Vec<APIMatch<'a>> {
    match ip_address {
        IpAddr::V4(ipv4) => ipv4_match(&aws_ranges.prefixes, ipv4),
        IpAddr::V6(ipv6) => ipv6_match(&aws_ranges.ipv6_prefixes, ipv6),
    }
}

fn ipv4_match<'a>(aws_ranges: &'a [Ipv4Prefix], ip_address: &'a Ipv4Addr) -> Vec<APIMatch<'a>> {
    aws_ranges
        .iter()
        .filter(|x| x.ipv4_prefix_compute.contains(ip_address))
        .map(|x| APIMatch {
            ip_prefix: &x.ip_prefix,
            region: &x.region,
            service: &x.service,
            network_border_group: &x.network_border_group,
        })
        .collect::<Vec<_>>()
}

fn ipv6_match<'a>(aws_ranges: &'a [Ipv6Prefix], ip_address: &'a Ipv6Addr) -> Vec<APIMatch<'a>> {
    aws_ranges
        .iter()
        .filter(|x| x.ipv6_prefix_compute.contains(ip_address))
        .map(|x| APIMatch {
            ip_prefix: &x.ipv6_prefix,
            region: &x.region,
            service: &x.service,
            network_border_group: &x.network_border_group,
        })
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_aws_response() -> AWSResponse {
        let test_aws_ranges = include_str!("../test_data/example_ip-ranges.json");
        let ranges: AWSIpRanges =
            serde_json::from_str(test_aws_ranges).expect("JSON deserialization error");

        calculate_aws_response(ranges, "TEST".to_owned())
    }

    #[test]
    fn test_ipv4_matching() {
        let aws_response = init_aws_response();

        // Match expected
        let ip_address: IpAddr = "52.1.1.1".parse().unwrap();
        let result_ranges = ip_match(&aws_response.ranges, &ip_address);
        assert!(!result_ranges.is_empty());

        // Match not expected
        let ip_address: IpAddr = "8.8.8.8".parse().unwrap();
        let result_ranges = ip_match(&aws_response.ranges, &ip_address);
        assert!(result_ranges.is_empty());
    }

    #[test]
    fn test_ipv6_matching() {
        let aws_response = init_aws_response();

        // Match expected
        let ip_address: IpAddr = "2406:da60:c000::00".parse().unwrap();
        let result_ranges = ip_match(&aws_response.ranges, &ip_address);
        assert!(!result_ranges.is_empty());

        // Match not expected
        let ip_address: IpAddr = "2206:de60:c000::00".parse().unwrap();
        let result_ranges = ip_match(&aws_response.ranges, &ip_address);
        assert!(result_ranges.is_empty());
    }
}
